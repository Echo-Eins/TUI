use anyhow::{Context, Result};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct CacheEntry {
    value: String,
    timestamp: Instant,
}

pub struct PowerShellExecutor {
    executable: String,
    timeout: Duration,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    cache_ttl: Duration,
    use_cache: bool,
}

impl PowerShellExecutor {
    pub fn new(
        executable: String,
        timeout_seconds: u64,
        cache_ttl_seconds: u64,
        use_cache: bool,
    ) -> Self {
        Self {
            executable,
            timeout: Duration::from_secs(timeout_seconds),
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl: Duration::from_secs(cache_ttl_seconds),
            use_cache,
        }
    }

    pub async fn execute(&self, command: &str) -> Result<String> {
        self.execute_inner(command, true).await
    }

    pub async fn execute_uncached(&self, command: &str) -> Result<String> {
        self.execute_inner(command, false).await
    }

    async fn execute_inner(&self, command: &str, allow_cache: bool) -> Result<String> {
        let should_use_cache = self.use_cache && allow_cache && self.cache_ttl > Duration::ZERO;

        // Check cache
        if should_use_cache {
            let cache = self.cache.read();
            if let Some(entry) = cache.get(command) {
                if entry.timestamp.elapsed() < self.cache_ttl {
                    return Ok(entry.value.clone());
                }
            }
        }

        // Execute command
        let output = tokio::task::spawn_blocking({
            let executable = self.executable.clone();
            let command = command.to_string();
            move || {
                Command::new(&executable)
                    .args(&["-NoProfile", "-NonInteractive", "-Command", &command])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
            }
        })
        .await?
        .context("Failed to execute PowerShell command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("PowerShell command failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        // Update cache
        if should_use_cache {
            let mut cache = self.cache.write();
            cache.insert(
                command.to_string(),
                CacheEntry {
                    value: stdout.clone(),
                    timestamp: Instant::now(),
                },
            );
        }

        Ok(stdout)
    }

    pub fn clear_cache(&self) {
        self.cache.write().clear();
    }
}

impl Clone for PowerShellExecutor {
    fn clone(&self) -> Self {
        Self {
            executable: self.executable.clone(),
            timeout: self.timeout,
            cache: Arc::clone(&self.cache),
            cache_ttl: self.cache_ttl,
            use_cache: self.use_cache,
        }
    }
}
