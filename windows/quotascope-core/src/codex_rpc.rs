//! A lazy, resident client for Codex's read-only app-server methods.
//! Pipes are bounded, requests time out, EOF tears down the child, and the
//! Windows child has no console window. No auth file is changed by this client.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcError {
    Missing,
    Failed,
    TimedOut,
    Refused,
}

pub struct Client {
    child: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<Value>,
    next_id: u64,
    timeout: Duration,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Client {
    fn start(mut command: Command, timeout: Duration) -> Result<Self, RpcError> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|_| RpcError::Failed)?;
        let input = child.stdin.take().ok_or(RpcError::Failed)?;
        let output = child.stdout.take().ok_or(RpcError::Failed)?;
        let (tx, replies) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            let mut line = Vec::new();
            loop {
                let Ok(chunk) = reader.fill_buf() else {
                    break;
                };
                if chunk.is_empty() {
                    break;
                } // EOF must end this thread.
                let end = chunk
                    .iter()
                    .position(|b| *b == b'\n')
                    .map(|i| i + 1)
                    .unwrap_or(chunk.len());
                if line.len().saturating_add(end) > 8 * 1024 * 1024 {
                    break;
                }
                line.extend_from_slice(&chunk[..end]);
                let complete = chunk[end - 1] == b'\n';
                reader.consume(end);
                if complete {
                    if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                    line.clear();
                }
            }
        });
        let mut client = Self {
            child,
            input,
            replies,
            next_id: 1,
            timeout,
        };
        client.call("initialize", Some(json!({"clientInfo":{"name":"QuotaScope","title":"QuotaScope","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true}})))?;
        client.write(&json!({"method":"initialized"}))?;
        Ok(client)
    }

    fn write(&mut self, value: &Value) -> Result<(), RpcError> {
        serde_json::to_writer(&mut self.input, value).map_err(|_| RpcError::Failed)?;
        self.input
            .write_all(b"\n")
            .and_then(|_| self.input.flush())
            .map_err(|_| RpcError::Failed)
    }

    pub fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value, RpcError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(RpcError::Failed)?;
        let mut request = json!({"id":id,"method":method});
        if let Some(params) = params {
            request["params"] = params;
        }
        self.write(&request)?;
        let started = Instant::now();
        loop {
            let remaining = self
                .timeout
                .checked_sub(started.elapsed())
                .ok_or(RpcError::TimedOut)?;
            let reply = self.replies.recv_timeout(remaining).map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout => RpcError::TimedOut,
                mpsc::RecvTimeoutError::Disconnected => RpcError::Failed,
            })?;
            if reply.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = reply.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                return Err(
                    if [
                        "not authenticated",
                        "not signed in",
                        "unauthorized",
                        "login required",
                    ]
                    .iter()
                    .any(|s| message.contains(s))
                    {
                        RpcError::Refused
                    } else {
                        RpcError::Failed
                    },
                );
            }
            return reply.get("result").cloned().ok_or(RpcError::Failed);
        }
    }
}

static SERVER: OnceLock<Mutex<Option<Client>>> = OnceLock::new();

pub fn request(method: &str) -> Result<Value, RpcError> {
    let mut guard = SERVER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| RpcError::Failed)?;
    if guard.is_none() {
        let path = locate_codex().ok_or(RpcError::Missing)?;
        let mut command = Command::new(path);
        command.arg("app-server");
        *guard = Some(Client::start(command, Duration::from_secs(20))?);
    }
    let result = guard.as_mut().ok_or(RpcError::Failed)?.call(method, None);
    if result.is_err() {
        *guard = None;
    }
    result
}

pub fn shutdown() {
    if let Some(server) = SERVER.get() {
        if let Ok(mut guard) = server.lock() {
            *guard = None;
        }
    }
}

pub fn locate_codex() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in if cfg!(windows) {
                &["codex.exe"][..]
            } else {
                &["codex"][..]
            } {
                let file = dir.join(name);
                if file.is_file() {
                    return Some(file);
                }
            }
        }
    }
    // GUI apps may not inherit npm's or the desktop app's updated PATH.
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::home_dir().join("AppData/Local"));
    newest_binary(&local.join("OpenAI/Codex/bin")).or_else(|| {
        npm_binary(&crate::home_dir().join("AppData/Roaming/npm/node_modules/@openai/codex"))
    })
}

fn newest_binary(root: &Path) -> Option<PathBuf> {
    fs_entries(root)
        .into_iter()
        .map(|p| p.join("codex.exe"))
        .filter(|p| p.is_file())
        .max_by_key(|p| p.metadata().ok().and_then(|m| m.modified().ok()))
}

fn npm_binary(root: &Path) -> Option<PathBuf> {
    for vendor in [
        root.join("vendor"),
        root.join("node_modules/@openai/codex-win32-x64/vendor"),
        root.join("node_modules/@openai/codex-win32-arm64/vendor"),
    ] {
        for triple in fs_entries(&vendor) {
            let binary = triple.join("codex/codex.exe");
            if binary.is_file() {
                return Some(binary);
            }
        }
    }
    None
}

fn fs_entries(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the user's installed and logged-in Codex CLI"]
    fn live_read_only_account_protocol() {
        struct Shutdown;
        impl Drop for Shutdown {
            fn drop(&mut self) {
                super::shutdown();
            }
        }
        let _shutdown = Shutdown;
        let root = request("account/rateLimits/read").expect("local Codex rate-limit read");
        assert!(root.get("rateLimits").is_some() || root.get("rateLimitsByLimitId").is_some());
        let reading = crate::providers::codex::parse_app_server_response(&root);
        assert_eq!(reading.origin.as_deref(), Some("appServer"));
        println!(
            "rate-limit protocol readable; windows decoded: {}",
            reading.windows.len()
        );
        let account = request("account/usage/read");
        println!(
            "account-usage method supported: {}",
            account
                .as_ref()
                .ok()
                .and_then(|usage| crate::codex_account::parse_usage(usage, &root))
                .is_some()
        );
    }

    #[test]
    fn helper() {
        if std::env::var_os("QUOTASCOPE_RPC_FIXTURE").is_none() {
            return;
        }
        let input = std::io::stdin();
        for line in input.lock().lines().map_while(Result::ok) {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap();
            if method == "close" {
                break;
            }
            if method == "silence" {
                std::thread::sleep(Duration::from_secs(30));
                break;
            }
            if let Some(id) = request.get("id") {
                println!(
                    "{}",
                    json!({"method":"account/rateLimits/updated","params":{}})
                );
                println!("{}", json!({"id":id,"result":{"method":method}}));
                std::io::stdout().flush().unwrap();
            }
        }
    }

    fn client(timeout: Duration) -> Client {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "codex_rpc::tests::helper", "--nocapture"])
            .env("QUOTASCOPE_RPC_FIXTURE", "1");
        Client::start(command, timeout).unwrap()
    }

    #[test]
    fn handshake_reuses_one_child_and_ignores_notifications_and_test_harness_output() {
        let mut client = client(Duration::from_secs(5));
        let pid = client.child.id();
        assert_eq!(
            client.call("account/rateLimits/read", None).unwrap()["method"],
            "account/rateLimits/read"
        );
        assert_eq!(
            client.call("account/usage/read", None).unwrap()["method"],
            "account/usage/read"
        );
        assert_eq!(client.child.id(), pid);
    }

    #[test]
    fn eof_and_timeout_are_bounded_and_the_child_can_be_restarted() {
        let mut closing = client(Duration::from_secs(5));
        assert_eq!(closing.call("close", None), Err(RpcError::Failed));
        drop(closing);
        let mut stalled = client(Duration::from_secs(5));
        stalled.timeout = Duration::from_millis(30);
        assert_eq!(stalled.call("silence", None), Err(RpcError::TimedOut));
        drop(stalled);
        let mut restarted = client(Duration::from_secs(5));
        assert!(restarted.call("account/rateLimits/read", None).is_ok());
    }
}
