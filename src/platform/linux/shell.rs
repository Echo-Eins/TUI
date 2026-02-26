use crate::platform::executor::{CommandExecutor, StreamMessage};
use crate::utils::ansi;
use crate::utils::utf8_buffer::Utf8AccumulationBuffer;
use anyhow::{Context, Result};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

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

/// Process a raw text chunk from stdout/stderr and send complete lines through the channel.
///
/// Handles:
/// - Line buffering (split on `\n`)
/// - Carriage return `\r` (resets current line — simulates terminal overwrite for progress bars)
/// - Trailing `\n` stripping on each line
/// - ANSI escape code stripping
async fn process_output_lines(
    text: &str,
    line_buffer: &mut String,
    tx: &mpsc::Sender<StreamMessage>,
    make_msg: fn(String) -> StreamMessage,
) {
    for c in text.chars() {
        match c {
            '\n' => {
                // Strip ANSI escape codes and send the completed line
                let clean = ansi::strip_ansi(line_buffer);
                let _ = tx.send(make_msg(clean)).await;
                line_buffer.clear();
            }
            '\r' => {
                // Carriage return: reset line buffer (overwrite behavior)
                line_buffer.clear();
            }
            _ => {
                line_buffer.push(c);
            }
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
            .args(["-c", cmd])
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

    async fn execute_stream(
        &self,
        cmd: &str,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
        terminal_size: Option<(u16, u16)>,
    ) -> Result<mpsc::Receiver<StreamMessage>> {
        let (tx, rx) = mpsc::channel(100);
        let cmd_owned = cmd.to_string();
        let active_pid = Arc::clone(&self.active_pid);
        let cwd_owned = cwd.map(|s| s.to_string());
        let env_owned = env.cloned();
        let term_size = terminal_size;

        tokio::spawn(async move {
            let mut command = Command::new("bash");
            command
                .args(["-c", &cmd_owned])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // Set working directory if provided
            if let Some(ref cwd) = cwd_owned {
                command.current_dir(cwd);
            }

            // Set accumulated session environment variables
            if let Some(ref env) = env_owned {
                for (key, value) in env {
                    command.env(key, value);
                }
            }

            // Set terminal size hints for commands that format output
            if let Some((cols, rows)) = term_size {
                command.env("COLUMNS", cols.to_string());
                command.env("LINES", rows.to_string());
                command.env("TERM", "dumb");
            }

            // Create a new process group so we can kill the entire tree
            #[cfg(unix)]
            unsafe {
                command.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }

            let mut child = match command.spawn() {
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

            // Spawn stdout reader task
            let stdout_handle = if let Some(mut stdout) = child.stdout.take() {
                let tx_clone = tx.clone();
                Some(tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let mut line_buffer = String::new();
                    let mut utf8_buf = Utf8AccumulationBuffer::new();
                    while let Ok(n) = stdout.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        let text = utf8_buf.push(&buf[..n]);
                        process_output_lines(
                            &text,
                            &mut line_buffer,
                            &tx_clone,
                            StreamMessage::Stdout,
                        )
                        .await;
                    }
                    // Flush any remaining UTF-8 bytes
                    let remaining = utf8_buf.flush();
                    if !remaining.is_empty() {
                        process_output_lines(
                            &remaining,
                            &mut line_buffer,
                            &tx_clone,
                            StreamMessage::Stdout,
                        )
                        .await;
                    }
                    // Flush any remaining incomplete line
                    if !line_buffer.is_empty() {
                        let clean = ansi::strip_ansi(&line_buffer);
                        let _ = tx_clone.send(StreamMessage::Stdout(clean)).await;
                    }
                }))
            } else {
                None
            };

            // Spawn stderr reader task
            let stderr_handle = if let Some(mut stderr) = child.stderr.take() {
                let tx_clone = tx.clone();
                Some(tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let mut line_buffer = String::new();
                    let mut utf8_buf = Utf8AccumulationBuffer::new();
                    while let Ok(n) = stderr.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        let text = utf8_buf.push(&buf[..n]);
                        process_output_lines(
                            &text,
                            &mut line_buffer,
                            &tx_clone,
                            StreamMessage::Stderr,
                        )
                        .await;
                    }
                    let remaining = utf8_buf.flush();
                    if !remaining.is_empty() {
                        process_output_lines(
                            &remaining,
                            &mut line_buffer,
                            &tx_clone,
                            StreamMessage::Stderr,
                        )
                        .await;
                    }
                    if !line_buffer.is_empty() {
                        let clean = ansi::strip_ansi(&line_buffer);
                        let _ = tx_clone.send(StreamMessage::Stderr(clean)).await;
                    }
                }))
            } else {
                None
            };

            // Wait for the child process to exit
            let exit_code = match child.wait().await {
                Ok(status) => status.code().unwrap_or(1),
                Err(_) => 1,
            };

            // CRITICAL: Wait for stdout/stderr tasks to finish BEFORE sending Exit.
            // This prevents the race condition where Exit arrives before all output is flushed.
            if let Some(handle) = stdout_handle {
                let _ = handle.await;
            }
            if let Some(handle) = stderr_handle {
                let _ = handle.await;
            }

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
            // Send SIGINT to the entire process group (negative PID)
            // This ensures child processes spawned by the command are also interrupted
            let pgid = format!("-{}", pid);
            let status = Command::new("kill")
                .args(["-INT", &pgid])
                .status()
                .await
                .context("Failed to invoke kill -INT on process group")?;
            if !status.success() {
                // Fallback: try killing just the PID directly
                let status = Command::new("kill")
                    .args(["-INT", &pid.to_string()])
                    .status()
                    .await
                    .context("Failed to invoke kill -INT")?;
                if !status.success() {
                    anyhow::bail!("Failed to send SIGINT to PID {}", pid);
                }
            }
        }
        Ok(())
    }
}
