use crate::platform::executor::{CommandExecutor, StreamMessage};
use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::Read;
use tokio::sync::mpsc;
use tokio::task;

pub struct LinuxCommandExecutor;

impl LinuxCommandExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl CommandExecutor for LinuxCommandExecutor {
    fn name(&self) -> &str {
        "bash"
    }

    async fn execute(&self, cmd: &str) -> Result<String> {
        let cmd_owned = cmd.to_string();

        let output = task::spawn_blocking(move || {
            let pty_system = NativePtySystem::default();
            let pair = pty_system.openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }).context("Failed to open PTY")?;

            let mut command = CommandBuilder::new("bash");
            command.args(["-lc", &cmd_owned]);

            let mut child = pair.slave.spawn_command(command).context("Failed to spawn command")?;
            drop(pair.slave);

            let mut reader = pair.master.try_clone_reader().context("Failed to clone reader")?;

            let mut buf = [0u8; 4096];
            let mut output = String::new();
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 { break; }
                output.push_str(&String::from_utf8_lossy(&buf[..n]));
            }

            let _ = child.wait();
            Ok::<String, anyhow::Error>(output)
        }).await.context("Task panicked")??;

        Ok(output)
    }

    async fn execute_stream(&self, cmd: &str) -> Result<mpsc::Receiver<StreamMessage>> {
        let (tx, rx) = mpsc::channel(100);
        let cmd_owned = cmd.to_string();

        task::spawn_blocking(move || {
            let pty_system = NativePtySystem::default();
            let pair = match pty_system.openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.blocking_send(StreamMessage::Stderr(format!("Error: {}", e)));
                    let _ = tx.blocking_send(StreamMessage::Exit(1));
                    return;
                }
            };

            let mut command = CommandBuilder::new("bash");
            command.args(["-lc", &cmd_owned]);

            let mut child = match pair.slave.spawn_command(command) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.blocking_send(StreamMessage::Stderr(format!("Failed to spawn: {}", e)));
                    let _ = tx.blocking_send(StreamMessage::Exit(1));
                    return;
                }
            };
            drop(pair.slave);

            let mut reader = match pair.master.try_clone_reader() {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.blocking_send(StreamMessage::Stderr(format!("Reader error: {}", e)));
                    let _ = tx.blocking_send(StreamMessage::Exit(1));
                    return;
                }
            };

            let mut buf = [0u8; 1024];
            let mut line_buffer = String::new();

            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 { break; }

                let text = String::from_utf8_lossy(&buf[..n]).to_string();

                for c in text.chars() {
                    line_buffer.push(c);
                    if c == '\n' {
                        // PTY merges stdout/stderr; treat as Stdout for now
                        let _ = tx.blocking_send(StreamMessage::Stdout(line_buffer.clone()));
                        line_buffer.clear();
                    }
                }
            }

            if !line_buffer.is_empty() {
                let _ = tx.blocking_send(StreamMessage::Stdout(line_buffer));
            }

            // Get exit code
            let exit_code = match child.wait() {
                Ok(status) => {
                    if status.success() { 0 } else {
                        // portable-pty ExitStatus doesn't always give code, default to 1
                        1
                    }
                }
                Err(_) => 1,
            };

            let _ = tx.blocking_send(StreamMessage::Exit(exit_code));
        });

        Ok(rx)
    }

    async fn suggest(&self, _input: &str) -> Result<Vec<String>> {
        // Implement compgen or history searching later
        Ok(Vec::new())
    }

    async fn validate(&self, cmd: &str) -> Result<bool> {
        let status = tokio::process::Command::new("bash")
            .args(["-n", "-c", cmd])
            .output()
            .await?
            .status;
        Ok(status.success())
    }
}
