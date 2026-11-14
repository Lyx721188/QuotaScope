//! Bounded, hidden read-only CLI probes. Never log command output.
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
pub enum Failure {
    Missing,
    Start,
    TimedOut,
    TooMuchOutput,
    Exited(String),
}

pub fn locate(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|root| {
            #[cfg(windows)]
            let suffixes = [".exe", ".cmd", ".bat", ""];
            #[cfg(not(windows))]
            let suffixes = [""];
            suffixes
                .into_iter()
                .map(move |suffix| root.join(format!("{name}{suffix}")))
        })
        .find(|p| p.is_file())
}

pub fn probe(name: &str, args: &[&str]) -> Result<Vec<u8>, Failure> {
    let binary = locate(name).ok_or(Failure::Missing)?;
    let mut command = Command::new(binary);
    command.args(args).current_dir(std::env::temp_dir());
    run(command, Duration::from_secs(20), 256 * 1024)
}

pub fn run(mut command: Command, deadline: Duration, ceiling: usize) -> Result<Vec<u8>, Failure> {
    // CLI owns its own login; unrelated tokens from the host are not inherited.
    let env: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy().to_ascii_uppercase();
            matches!(
                key.as_str(),
                "PATH"
                    | "PATHEXT"
                    | "SYSTEMROOT"
                    | "SYSTEMDRIVE"
                    | "COMSPEC"
                    | "USERPROFILE"
                    | "HOMEDRIVE"
                    | "HOMEPATH"
                    | "HOME"
                    | "APPDATA"
                    | "LOCALAPPDATA"
                    | "TEMP"
                    | "TMP"
                    | "TMPDIR"
                    | "LANG"
                    | "LC_ALL"
                    | "HTTP_PROXY"
                    | "HTTPS_PROXY"
                    | "ALL_PROXY"
                    | "NO_PROXY"
            )
        })
        .collect();
    command
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::proxy::environment(&mut command);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|_| Failure::Start)?;
    #[cfg(windows)]
    let _job = match crate::kiro_acp::Job::attach(&child) {
        Ok(job) => job,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Failure::Start);
        }
    };
    let (tx, rx) = mpsc::channel();
    for (index, stream) in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ]
    .into_iter()
    .enumerate()
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream.take(ceiling as u64 + 1).read_to_end(&mut bytes);
            let _ = tx.send((index, bytes));
        });
    }
    drop(tx);
    let started = Instant::now();
    let mut output: [Option<Vec<u8>>; 2] = [None, None];
    let result = loop {
        for (index, bytes) in rx.try_iter() {
            output[index] = Some(bytes);
        }
        if output.iter().flatten().any(|b| b.len() > ceiling) {
            break Err(Failure::TooMuchOutput);
        }
        if started.elapsed() >= deadline {
            break Err(Failure::TimedOut);
        }
        match child.try_wait() {
            Ok(Some(status)) if output.iter().all(Option::is_some) => {
                if status.success() {
                    break Ok(output[0].take().unwrap());
                }
                break Err(Failure::Exited(
                    String::from_utf8_lossy(output[1].as_ref().unwrap()).into_owned(),
                ));
            }
            Err(_) => break Err(Failure::Start),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "probe subprocess fixture"]
    fn overflow() {
        print!("{}", "x".repeat(10000));
    }
    #[test]
    #[ignore = "probe subprocess fixture"]
    fn timeout() {
        std::thread::sleep(Duration::from_secs(10));
    }
    #[test]
    #[ignore = "probe subprocess fixture"]
    fn ok() {
        println!("{{\"ok\":true}}");
    }
    fn command(mode: &str) -> Command {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args([
            "--ignored",
            "--exact",
            &format!("local_process::tests::{mode}"),
            "--nocapture",
        ]);
        c
    }
    #[test]
    fn probes_bound_output_and_time() {
        assert_eq!(
            run(command("overflow"), Duration::from_secs(5), 512),
            Err(Failure::TooMuchOutput)
        );
        assert_eq!(
            run(command("timeout"), Duration::from_millis(200), 512),
            Err(Failure::TimedOut)
        );
        assert!(
            String::from_utf8(run(command("ok"), Duration::from_secs(5), 2048).unwrap())
                .unwrap()
                .contains("\"ok\":true")
        );
    }
}
