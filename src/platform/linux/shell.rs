use crate::platform::executor::CommandExecutor;
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
    async fn execute(&self, cmd: &str) -> Result<String> {
        let (tx, mut rx) = mpsc::channel(32);
        
        let cmd_owned = cmd.to_string();
        
        // Run blocking PTY operations in a spawn_blocking task
        let handle = task::spawn_blocking(move || {
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
            drop(pair.slave); // Close the slave side in the parent

            let mut reader = pair.master.try_clone_reader().context("Failed to clone reader")?;
            
            // Read output in smaller chunks and send them over
            let mut buf = [0u8; 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx.blocking_send(text);
            }

            let _ = child.wait();
            Ok::<(), anyhow::Error>(())
        });
        
        let mut output = String::new();
        while let Some(chunk) = rx.recv().await {
            output.push_str(&chunk);
        }
        
        handle.await.context("Task panicked")??;
        
        Ok(output)
    }

    async fn execute_stream(&self, cmd: &str) -> Result<mpsc::Receiver<String>> {
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
                    let _ = tx.blocking_send(format!("Error: {}", e));
                    return;
                }
            };

            let mut command = CommandBuilder::new("bash");
            command.args(["-lc", &cmd_owned]);

            let mut child = match pair.slave.spawn_command(command) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.blocking_send(format!("Failed to spawn command: {}", e));
                    return;
                }
            };
            drop(pair.slave);

            let mut reader = match pair.master.try_clone_reader() {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.blocking_send(format!("Failed to clone reader: {}", e));
                    return;
                }
            };

            let mut buf = [0u8; 1024];
            let mut line_buffer = String::new();

            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                
                // Extremely simple line splitting without losing characters
                for c in text.chars() {
                    line_buffer.push(c);
                    if c == '\n' {
                        let _ = tx.blocking_send(line_buffer.clone());
                        line_buffer.clear();
                    }
                }
            }
            
            if !line_buffer.is_empty() {
                let _ = tx.blocking_send(line_buffer);
            }

            let _ = child.wait();
        });

        Ok(rx)
    }

    async fn suggest(&self, _input: &str) -> Result<Vec<String>> {
        // Implement compgen or history searching later
        Ok(Vec::new())
    }

    async fn validate(&self, cmd: &str) -> Result<bool> {
        // Use bash -n to test syntax validity
        let status = tokio::process::Command::new("bash")
            .args(["-n", "-c", cmd])
            .output()
            .await?
            .status;
        Ok(status.success())
    }
}
