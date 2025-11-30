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
    cache_enabled: bool,
}

impl PowerShellExecutor {
    /// Creates a new executor. Set `use_cache` to false or `cache_ttl_seconds` to 0 to disable
    /// caching for scenarios that require very frequent refreshes.
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
            cache_enabled: use_cache && cache_ttl_seconds > 0,
        }
    }

    pub async fn execute(&self, command: &str) -> Result<String> {
        // Check cache
        if self.cache_enabled {
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
        if self.cache_enabled {
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

    pub fn check_environment(executable: &str) -> PowerShellEnvironmentStatus {
        let version_check = Command::new(executable)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$PSVersionTable.PSVersion.ToString()",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        if version_check.is_err() || !version_check.as_ref().map(|s| s.success()).unwrap_or(false) {
            return PowerShellEnvironmentStatus {
                available: false,
                missing_modules: Vec::new(),
            };
        }

        let required_modules = vec!["CimCmdlets", "Microsoft.PowerShell.Management"];
        let module_script = format!(
            "{}{}{}{}",
            "$required = @('",
            required_modules.join("','"),
            "');",
            "$missing = $required | Where-Object { -not (Get-Module -ListAvailable $_) };$missing"
        );

        let module_check = Command::new(executable)
            .args(["-NoProfile", "-NonInteractive", "-Command", &module_script])
            .output();

        let missing_modules = match module_check {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(String::from)
                    .collect::<Vec<_>>()
            }
            _ => required_modules.iter().map(|s| s.to_string()).collect(),
        };

        PowerShellEnvironmentStatus {
            available: true,
            missing_modules,
        }
    }
}

impl Clone for PowerShellExecutor {
    fn clone(&self) -> Self {
        Self {
            executable: self.executable.clone(),
            timeout: self.timeout,
            cache: Arc::clone(&self.cache),
            cache_ttl: self.cache_ttl,
            cache_enabled: self.cache_enabled,
        }
    }
}

pub struct PowerShellEnvironmentStatus {
    pub available: bool,
    pub missing_modules: Vec<String>,
}
