use anyhow::Result;
use std::process::{Command, Stdio};
use tokio::time::Duration;

pub struct PowerShellExecutor {
    executable: String,
    timeout: Duration,
}

impl PowerShellExecutor {
    pub fn new(executable: String, timeout_seconds: u64) -> Self {
        Self {
            executable,
            timeout: Duration::from_secs(timeout_seconds),
        }
    }

    pub async fn execute(&self, command: &str) -> Result<String> {
        let output = Command::new(&self.executable)
            .args(&["-NoProfile", "-NonInteractive", "-Command", command])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout)
    }

    pub async fn execute_script(&self, script: &str) -> Result<String> {
        let output = Command::new(&self.executable)
            .args(&["-NoProfile", "-NonInteractive", "-Command", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout)
    }
}
