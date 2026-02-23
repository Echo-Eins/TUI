use crate::platform::executor::{CommandExecutor, StreamMessage};
use anyhow::{Context, Result};
use parking_lot::RwLock;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

pub struct LinuxCommandExecutor {
    active_pid: Arc<RwLock<Option<u32>>>,
}

impl LinuxCommandExecutor {
    pub fn new() -> Self {
        Self {
            active_pid: Arc::new(RwLock::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl CommandExecutor for LinuxCommandExecutor {
    fn name(&self) -> &str {
        "bash"
    }

    async fn execute(&self, cmd: &str) -> Result<String> {
        let output = Command::new("bash")
            .args(["-lc", cmd])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to spawn bash command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let code = output.status.code().unwrap_or(1);
            anyhow::bail!("bash command failed (exit {}): {}", code, stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn execute_stream(&self, cmd: &str) -> Result<mpsc::Receiver<StreamMessage>> {
        let (tx, rx) = mpsc::channel(100);
        let cmd_owned = cmd.to_string();
        let active_pid = Arc::clone(&self.active_pid);

        tokio::spawn(async move {
            let mut child = match Command::new("bash")
                .args(["-lc", &cmd_owned])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx
                        .send(StreamMessage::Stderr(format!("Failed to spawn: {}", e)))
                        .await;
                    let _ = tx.send(StreamMessage::Exit(1)).await;
                    return;
                }
            };

            *active_pid.write() = child.id();

            if let Some(mut stdout) = child.stdout.take() {
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let mut line_buffer = String::new();
                    while let Ok(n) = stdout.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        let text = String::from_utf8_lossy(&buf[..n]).to_string();
                        for c in text.chars() {
                            line_buffer.push(c);
                            if c == '\n' {
                                let _ = tx_clone
                                    .send(StreamMessage::Stdout(line_buffer.clone()))
                                    .await;
                                line_buffer.clear();
                            }
                        }
                    }
                    if !line_buffer.is_empty() {
                        let _ = tx_clone.send(StreamMessage::Stdout(line_buffer)).await;
                    }
                });
            }

            if let Some(mut stderr) = child.stderr.take() {
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let mut line_buffer = String::new();
                    while let Ok(n) = stderr.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        let text = String::from_utf8_lossy(&buf[..n]).to_string();
                        for c in text.chars() {
                            line_buffer.push(c);
                            if c == '\n' {
                                let _ = tx_clone
                                    .send(StreamMessage::Stderr(line_buffer.clone()))
                                    .await;
                                line_buffer.clear();
                            }
                        }
                    }
                    if !line_buffer.is_empty() {
                        let _ = tx_clone.send(StreamMessage::Stderr(line_buffer)).await;
                    }
                });
            }

            let exit_code = match child.wait().await {
                Ok(status) => status.code().unwrap_or(1),
                Err(_) => 1,
            };
            *active_pid.write() = None;
            let _ = tx.send(StreamMessage::Exit(exit_code)).await;
        });

        Ok(rx)
    }

    async fn suggest(&self, _input: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn validate(&self, cmd: &str) -> Result<bool> {
        let status = Command::new("bash")
            .args(["-n", "-c", cmd])
            .status()
            .await?;
        Ok(status.success())
    }

    async fn interrupt(&self) -> Result<()> {
        let pid = *self.active_pid.read();
        if let Some(pid) = pid {
            let pid_str = pid.to_string();
            let status = Command::new("kill")
                .args(["-INT", &pid_str])
                .status()
                .await
                .context("Failed to invoke kill -INT")?;
            if !status.success() {
                anyhow::bail!("Failed to send SIGINT to PID {}", pid);
            }
        }
        Ok(())
    }
}
