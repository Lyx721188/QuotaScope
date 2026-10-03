//! Short-lived, read-only Kiro ACP requests. The CLI owns auth and refresh.
//! Handshake order follows Pulse's KiroACPClient at 3696a65; Windows uses a
//! hidden process and a kill-on-close job so child helpers leave with it.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_LINE: usize = 1024 * 1024;
const MAX_OUTPUT: usize = 8 * MAX_LINE;

#[derive(Debug, PartialEq)]
pub enum Failure {
    Missing,
    Start,
    TimedOut,
    Closed,
    Server(String),
    Unsupported,
    Unreadable,
}

struct Client {
    child: Child,
    writes: mpsc::SyncSender<Value>,
    written: mpsc::Receiver<Result<(), Failure>>,
    replies: mpsc::Receiver<Result<Value, Failure>>,
    timeout: Duration,
    #[cfg(windows)]
    job: Job,
}

impl Drop for Client {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.job.close();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Client {
    fn start(mut command: Command, timeout: Duration) -> Result<Self, Failure> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|_| Failure::Start)?;
        #[cfg(windows)]
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        let (writes, incoming) = mpsc::sync_channel::<Value>(1);
        let (completed, written) = mpsc::sync_channel(1);
        // A helper that stops reading stdin must not defeat the RPC deadline.
        // Killing its job after a timeout also releases a blocked pipe writer.
        std::thread::spawn(move || {
            while let Ok(value) = incoming.recv() {
                let result = serde_json::to_writer(&mut input, &value)
                    .map_err(|_| Failure::Closed)
                    .and_then(|_| {
                        input
                            .write_all(b"\n")
                            .and_then(|_| input.flush())
                            .map_err(|_| Failure::Closed)
                    });
                let failed = result.is_err();
                if completed.send(result).is_err() || failed {
                    break;
                }
            }
        });
        let (tx, replies) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            let mut line = Vec::new();
            let mut total: usize = 0;
            while let Ok(chunk) = reader.fill_buf() {
                if chunk.is_empty() {
                    break;
                }
                let end = chunk
                    .iter()
                    .position(|b| *b == b'\n')
                    .map(|i| i + 1)
                    .unwrap_or(chunk.len());
                total = total.saturating_add(end);
                if total > MAX_OUTPUT || line.len().saturating_add(end) > MAX_LINE {
                    let _ = tx.try_send(Err(Failure::Unreadable));
                    break;
                }
                line.extend_from_slice(&chunk[..end]);
                let complete = chunk[end - 1] == b'\n';
                reader.consume(end);
                if complete {
                    // Ignore banners and notifications; never log raw frames.
                    if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                        if tx.send(Ok(value)).is_err() {
                            break;
                        }
                    }
                    line.clear();
                }
            }
        });
        Ok(Self {
            child,
            writes,
            written,
            replies,
            timeout,
            #[cfg(windows)]
            job,
        })
    }

    fn write(&self, value: Value, started: Instant) -> Result<(), Failure> {
        self.writes.try_send(value).map_err(|_| Failure::Closed)?;
        let remaining = self
            .timeout
            .checked_sub(started.elapsed())
            .ok_or(Failure::TimedOut)?;
        self.written
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => Failure::TimedOut,
                mpsc::RecvTimeoutError::Disconnected => Failure::Closed,
            })?
    }

    fn call(&mut self, id: u64, method: &str, params: Value) -> Result<Value, Failure> {
        let started = Instant::now();
        self.write(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            started,
        )?;
        loop {
            let remaining = self
                .timeout
                .checked_sub(started.elapsed())
                .ok_or(Failure::TimedOut)?;
            let reply = self
                .replies
                .recv_timeout(remaining)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => Failure::TimedOut,
                    mpsc::RecvTimeoutError::Disconnected => Failure::Closed,
                })??;
            // An unsolicited server request is never executed. Answer it so
            // the server does not wait indefinitely for a client capability.
            if reply.get("method").is_some() {
                if let Some(server_id) = reply.get("id") {
                    self.write(json!({"jsonrpc":"2.0","id":server_id,
                        "error":{"code":-32601,"message":"Method not supported by this usage client"}}), started)?;
                }
                continue;
            }
            if reply["id"].as_u64() != Some(id) {
                continue;
            }
            if reply["jsonrpc"].as_str() != Some("2.0") {
                return Err(Failure::Unreadable);
            }
            if let Some(error) = reply.get("error").filter(|v| !v.is_null()) {
                if error["code"].as_i64() == Some(-32601) {
                    return Err(Failure::Unsupported);
                }
                return Err(Failure::Server(
                    error["message"].as_str().unwrap_or_default().into(),
                ));
            }
            return reply
                .get("result")
                .filter(|v| v.is_object())
                .cloned()
                .ok_or(Failure::Unreadable);
        }
    }
}

fn usage_with(command: Command, timeout: Duration) -> Result<Value, Failure> {
    // Every fetch owns its receiver and process. No stale ID, partial line or
    // timeout callback can cross into the next fetch.
    let mut client = Client::start(command, timeout)?;
    client.call(
        1,
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{},
        "clientInfo":{"name":"QuotaScope","version":env!("CARGO_PKG_VERSION")}}),
    )?;
    client.call(2, "_kiro/account/getUsage", json!({}))
}

pub fn usage() -> Result<Value, Failure> {
    let executable = locate().ok_or(Failure::Missing)?;
    let mut command = Command::new(&executable);
    command.args(["acp", "--agent-engine", "v3", "--auth-method", "cli"]);
    // The helper runs in an empty application directory, not the user's repo.
    let root = crate::data_dir().join("KiroACP");
    std::fs::create_dir_all(&root).map_err(|_| Failure::Start)?;
    command.current_dir(root);
    if let Some(parent) = executable.parent() {
        let mut paths = vec![parent.to_path_buf()];
        if let Some(path) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&path));
        }
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
    }
    crate::proxy::environment(&mut command);
    usage_with(command, Duration::from_secs(20))
}

pub fn locate() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend([
        crate::home_dir().join(".local/bin"),
        crate::home_dir().join("bin"),
    ]);
    dirs.into_iter()
        .map(|dir| {
            dir.join(if cfg!(windows) {
                "kiro-cli.exe"
            } else {
                "kiro-cli"
            })
        })
        .find(|path| path.is_file())
}

#[cfg(windows)]
struct Job(Option<windows::Win32::Foundation::HANDLE>);

#[cfg(windows)]
impl Job {
    fn attach(child: &Child) -> Result<Self, Failure> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::*;
        let job = Self(Some(
            unsafe { CreateJobObjectW(None, None) }.map_err(|_| Failure::Start)?,
        ));
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job.0.unwrap(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
            .map_err(|_| Failure::Start)?;
            AssignProcessToJobObject(job.0.unwrap(), HANDLE(child.as_raw_handle()))
                .map_err(|_| Failure::Start)?;
        }
        Ok(job)
    }
    fn close(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "kiro_acp::tests::helper",
                "--ignored",
                "--nocapture",
            ])
            .env("QUOTASCOPE_KIRO_FIXTURE", mode);
        command
    }

    fn emit(value: Value) {
        println!("{value}");
        std::io::stdout().flush().unwrap();
    }

    #[test]
    #[ignore = "isolated ACP fixture subprocess; invoked by the tests below"]
    fn helper() {
        let mode = std::env::var("QUOTASCOPE_KIRO_FIXTURE").unwrap();
        if mode == "sleep" {
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        let mut lines = std::io::stdin().lock().lines();
        let init: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(init["method"], "initialize");
        assert_eq!(init["params"]["protocolVersion"], 1);
        assert_eq!(init["params"]["clientCapabilities"], json!({}));
        if mode == "initialize-error" {
            emit(json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"changed"}}));
            return;
        }
        if mode == "blocked-write" {
            emit(json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}));
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        // Sending usage before initialize completes is a real upstream race.
        // Fail the fixture if the second request is already on the pipe.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        drop(lines);
        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        emit(json!({"jsonrpc":"2.0","id":2,"result":{"premature":true}}));
        // A server request may reuse our request ID, and is still a request.
        emit(json!({"jsonrpc":"2.0","id":1,"method":"fixture/unsupported","params":{}}));
        let response: Value =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        assert_eq!(response["error"]["code"], -32601);
        emit(json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}));
        let usage: Value =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        assert_eq!(usage["method"], "_kiro/account/getUsage");
        assert_eq!(usage["params"], json!({}));
        match mode.as_str() {
            "timeout" => {
                std::thread::sleep(Duration::from_secs(30));
                return;
            }
            "eof" => return,
            "partial" => {
                print!("{{\"jsonrpc\":\"2.0\",\"id\":2");
                return;
            }
            "large" => {
                print!("{}", "x".repeat(MAX_LINE + 1));
                return;
            }
            "login-error" => {
                emit(
                    json!({"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Please sign in"}}),
                );
                return;
            }
            "no-result" => {
                emit(json!({"jsonrpc":"2.0","id":2}));
                return;
            }
            _ => {}
        }
        // stderr must not fill its pipe; notifications and old IDs are skipped.
        eprint!("{}", "x".repeat(256 * 1024));
        emit(json!({"jsonrpc":"2.0","method":"fixture/update","params":{}}));
        emit(json!({"jsonrpc":"2.0","id":1,"result":{"stale":true}}));
        let mut result: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/upstream-kiro-pro-plus-usage.json"
        ))
        .unwrap();
        if mode == "descendant" {
            let child = command("sleep")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            result["descendant"] = json!(child.id());
            // Production's Windows job owns descendants even when the CLI
            // stops tracking them. Keep this child alive until job teardown.
            std::mem::forget(child);
        }
        result["helperPid"] = json!(std::process::id());
        emit(json!({"jsonrpc":"2.0","id":2,"result":result}));
        let _ = rx.recv(); // Only EOF/parent teardown ends the helper.
    }

    #[test]
    fn handshake_waits_and_a_noisy_helper_reads_and_terminates() {
        let result = usage_with(command("happy"), Duration::from_secs(5)).unwrap();
        assert_eq!(result["data"]["planName"], "KIRO PRO+");
        assert_eq!(crate::providers::kiro::reading(&result).windows.len(), 2);
        #[cfg(windows)]
        assert_stopped(result["helperPid"].as_u64().unwrap() as u32);
    }

    #[test]
    fn rpc_errors_eof_partial_frames_and_output_limits_are_explicit() {
        for (mode, expected) in [
            ("initialize-error", Failure::Unsupported),
            ("login-error", Failure::Server("Please sign in".into())),
            ("eof", Failure::Closed),
            ("partial", Failure::Closed),
            ("large", Failure::Unreadable),
            ("no-result", Failure::Unreadable),
        ] {
            assert_eq!(
                usage_with(command(mode), Duration::from_secs(5)),
                Err(expected),
                "{mode}"
            );
        }
    }

    #[test]
    fn timeout_tears_down_and_the_next_fetch_has_no_old_frames() {
        let mut client = Client::start(command("timeout"), Duration::from_secs(5)).unwrap();
        client
            .call(
                1,
                "initialize",
                json!({"protocolVersion":1,"clientCapabilities":{}}),
            )
            .unwrap();
        client.timeout = Duration::from_millis(50);
        let started = Instant::now();
        assert_eq!(
            client.call(2, "_kiro/account/getUsage", json!({})),
            Err(Failure::TimedOut)
        );
        let pid = client.child.id();
        drop(client);
        assert!(started.elapsed() < Duration::from_secs(2));
        #[cfg(windows)]
        assert_stopped(pid);
        #[cfg(not(windows))]
        let _ = pid;
        let result = usage_with(command("happy"), Duration::from_secs(5)).unwrap();
        assert_eq!(result["success"], true);
    }

    #[test]
    fn a_helper_that_stops_reading_stdin_cannot_defeat_the_timeout() {
        let mut client = Client::start(command("blocked-write"), Duration::from_secs(5)).unwrap();
        client
            .call(
                1,
                "initialize",
                json!({"protocolVersion":1,"clientCapabilities":{}}),
            )
            .unwrap();
        client.timeout = Duration::from_millis(50);
        let started = Instant::now();
        assert_eq!(
            client.call(
                2,
                "_kiro/account/getUsage",
                json!({"fixture":"x".repeat(256 * 1024)})
            ),
            Err(Failure::TimedOut)
        );
        drop(client);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(windows)]
    fn assert_stopped(pid: u32) {
        use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        if let Ok(handle) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
            let status = unsafe { WaitForSingleObject(handle, 2000) };
            let _ = unsafe { CloseHandle(handle) };
            assert_eq!(status, WAIT_OBJECT_0, "helper {pid} survived teardown");
        }
    }

    #[test]
    #[cfg(windows)]
    fn windows_teardown_also_stops_cli_descendants() {
        let result = usage_with(command("descendant"), Duration::from_secs(5)).unwrap();
        assert_stopped(result["descendant"].as_u64().unwrap() as u32);
    }
}
