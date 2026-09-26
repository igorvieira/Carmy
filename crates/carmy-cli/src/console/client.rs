//! Runs the application's `console` command and exchanges `carmy-console/1` lines with it.
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{Receiver, Sender, channel},
    thread,
};

/// What the application process reports.
#[derive(Debug)]
pub enum Incoming {
    /// A protocol line (the `ready` event or a response).
    Line(Value),
    /// A line on stderr: build output while compiling, logs afterwards.
    Log(String),
    /// The process ended; `Client::exit_code` has its code.
    Exited,
}

pub struct Client {
    child: Child,
    stdin: ChildStdin,
}

impl Client {
    /// `cargo run --quiet -- console` in `project`, with stdout and stderr read on threads.
    pub fn spawn(project: &Path) -> std::io::Result<(Self, Receiver<Incoming>)> {
        let mut child = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .args(["run", "--quiet", "--", "console"])
            .current_dir(project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (sender, receiver) = channel();
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let lines = sender.clone();
        let out = thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let message = match serde_json::from_str(&line) {
                    Ok(value) => Incoming::Line(value),
                    Err(_) => Incoming::Log(line),
                };
                if lines.send(message).is_err() {
                    break;
                }
            }
        });
        let logs = sender.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if logs.send(Incoming::Log(line)).is_err() {
                    break;
                }
            }
        });
        let stdin = child.stdin.take().expect("piped");
        let client = Self { child, stdin };
        watch_exit(out, sender);
        Ok((client, receiver))
    }

    pub fn send(&mut self, request: &Value) -> std::io::Result<()> {
        writeln!(self.stdin, "{request}")?;
        self.stdin.flush()
    }

    pub fn exit_code(&mut self) -> Option<i32> {
        self.child.try_wait().ok().flatten().and_then(|s| s.code())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{{\"op\":\"exit\"}}");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reports `Exited` once stdout closes, which happens when the process ends.
fn watch_exit(stdout: thread::JoinHandle<()>, sender: Sender<Incoming>) {
    thread::spawn(move || {
        let _ = stdout.join();
        let _ = sender.send(Incoming::Exited);
    });
}
