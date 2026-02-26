use anyhow::{Context, Result};
use base64::Engine;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::process::{Command as StdCommand, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_LOG_CHARS: usize = 4096;
const PS_ENCODING_PREFIX: &str =
    "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n$OutputEncoding = [System.Text.Encoding]::UTF8\n";

struct LimitedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn encode_powershell_command(command: &str) -> String {
    let mut bytes = Vec::with_capacity(command.len().saturating_mul(2));
    for unit in command.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn sanitize_for_log(command: &str) -> String {
    let mut sanitized = command.replace('\r', "\\r").replace('\n', "\\n");
    if sanitized.len() > MAX_LOG_CHARS {
        sanitized.truncate(MAX_LOG_CHARS);
        sanitized.push_str("...");
    }
    sanitized
}

fn split_batch_output(output: &str, separator: &str, expected: usize) -> Result<Vec<String>> {
    let mut parts: Vec<&str> = output.split(separator).collect();
    if parts.len() < expected + 2 {
        anyhow::bail!("PowerShell batch output missing separators");
    }

    parts.remove(0);
    parts.pop();

    if parts.len() != expected {
        anyhow::bail!(
            "PowerShell batch output count mismatch: expected {}, got {}",
            expected,
            parts.len()
        );
    }

    Ok(parts.into_iter().map(|s| s.trim().to_string()).collect())
}

async fn read_limited<R>(mut reader: R, limit: usize) -> Result<LimitedOutput>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::with_capacity(limit.min(8192));
    let mut total = 0usize;
    let mut truncated = false;
    let mut chunk = [0u8; 8192];

    loop {
        let n = reader
            .read(&mut chunk)
            .await
            .context("Failed to read PowerShell output")?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n);
        if buf.len() < limit {
            let remaining = limit - buf.len();
            let to_copy = remaining.min(n);
            buf.extend_from_slice(&chunk[..to_copy]);
        }
        if total > limit {
            truncated = true;
        }
    }

    Ok(LimitedOutput {
        bytes: buf,
        truncated,
    })
}

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
    active_pid: Arc<RwLock<Option<u32>>>,
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
            active_pid: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn execute_batch(&self, commands: &[&str]) -> Result<Vec<String>> {
        if commands.is_empty() {
            return Ok(Vec::new());
        }

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_nanos();
        let separator = format!("__CODEX_PS_BATCH_{}__", stamp);
        let escaped_separator = separator.replace('\'', "''");

        let mut script = String::new();
        script.push_str(PS_ENCODING_PREFIX);
        script.push_str("$ErrorActionPreference = 'Continue'\n");
        script.push_str("$ProgressPreference = 'SilentlyContinue'\n");
        script.push_str("$WarningPreference = 'SilentlyContinue'\n");
        script.push_str(&format!("$__codex_sep = '{}'\n", escaped_separator));
        for command in commands {
            script.push_str("Write-Output $__codex_sep\n");
            script.push_str(command);
            script.push('\n');
        }
        script.push_str("Write-Output $__codex_sep\n");

        let output = self.execute(&script).await?;
        split_batch_output(&output, &separator, commands.len())
    }

    #[allow(dead_code)]
    pub fn clear_cache(&self) {
        self.cache.write().clear();
    }

    pub fn check_environment(executable: &str) -> PowerShellEnvironmentStatus {
        let version_check = StdCommand::new(executable)
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

        let module_check = StdCommand::new(executable)
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

use crate::platform::executor::{CommandExecutor, StreamMessage};
use crate::utils::ansi;
use crate::utils::utf8_buffer::Utf8AccumulationBuffer;
use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
impl CommandExecutor for PowerShellExecutor {
    fn name(&self) -> &str {
        "powershell"
    }

    async fn execute(&self, command: &str) -> Result<String> {
        let cache_key = command.to_string();
        // Check cache
        if self.cache_enabled {
            let cache = self.cache.read();
            if let Some(entry) = cache.get(&cache_key) {
                if entry.timestamp.elapsed() < self.cache_ttl {
                    return Ok(entry.value.clone());
                }
            }
        }

        let command = format!("{}{}", PS_ENCODING_PREFIX, command);

        log::debug!(
            "Executing PowerShell command: {}",
            sanitize_for_log(&command)
        );

        let encoded_command = encode_powershell_command(&command);
        let mut child = TokioCommand::new(&self.executable)
            .args(&[
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &encoded_command,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn PowerShell process")?;

        let stdout = child
            .stdout
            .take()
            .context("Failed to capture PowerShell stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("Failed to capture PowerShell stderr")?;

        let stdout_handle = tokio::spawn(read_limited(stdout, MAX_OUTPUT_BYTES));
        let stderr_handle = tokio::spawn(read_limited(stderr, MAX_OUTPUT_BYTES));

        let status = match timeout(self.timeout, child.wait()).await {
            Ok(result) => result.context("Failed to wait for PowerShell process")?,
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                stdout_handle.abort();
                stderr_handle.abort();
                anyhow::bail!(
                    "PowerShell command timed out after {}s",
                    self.timeout.as_secs()
                );
            }
        };

        let stdout = stdout_handle
            .await
            .context("Failed to join stdout reader")??;
        let stderr = stderr_handle
            .await
            .context("Failed to join stderr reader")??;

        if stdout.truncated {
            log::warn!("PowerShell stdout truncated to {} bytes", MAX_OUTPUT_BYTES);
        }
        if stderr.truncated {
            log::warn!("PowerShell stderr truncated to {} bytes", MAX_OUTPUT_BYTES);
        }

        let stdout_text = decode_output(&stdout.bytes);
        let stderr_text = decode_output(&stderr.bytes);

        if !stderr_text.trim().is_empty() {
            log::debug!(
                "PowerShell stderr: {}",
                sanitize_for_log(stderr_text.trim())
            );
        }

        if !status.success() {
            let message = if stderr_text.trim().is_empty() {
                "PowerShell command failed with empty stderr".to_string()
            } else {
                stderr_text.trim().to_string()
            };
            let code = status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "terminated".to_string());
            anyhow::bail!("PowerShell command failed (exit {}): {}", code, message);
        }

        let stdout = stdout_text;

        // Update cache
        if self.cache_enabled {
            let mut cache = self.cache.write();
            cache.insert(
                cache_key,
                CacheEntry {
                    value: stdout.clone(),
                    timestamp: Instant::now(),
                },
            );
        }

        Ok(stdout)
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
        let exec = self.executable.clone();
        let active_pid = Arc::clone(&self.active_pid);
        let cwd_owned = cwd.map(|s| s.to_string());
        let env_owned = env.cloned();
        let term_size = terminal_size;

        tokio::spawn(async move {
            let full_cmd = format!("{}{}", PS_ENCODING_PREFIX, cmd_owned);
            let encoded_command = encode_powershell_command(&full_cmd);
            let mut command = TokioCommand::new(&exec);
            command
                .args(&[
                    "-NoProfile",
                    "-NonInteractive",
                    "-EncodedCommand",
                    &encoded_command,
                ])
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
                        for c in text.chars() {
                            match c {
                                '\n' => {
                                    let clean = ansi::strip_ansi(&line_buffer);
                                    let _ = tx_clone.send(StreamMessage::Stdout(clean)).await;
                                    line_buffer.clear();
                                }
                                '\r' => {
                                    line_buffer.clear();
                                }
                                _ => {
                                    line_buffer.push(c);
                                }
                            }
                        }
                    }
                    let remaining = utf8_buf.flush();
                    if !remaining.is_empty() {
                        for c in remaining.chars() {
                            match c {
                                '\n' => {
                                    let clean = ansi::strip_ansi(&line_buffer);
                                    let _ = tx_clone.send(StreamMessage::Stdout(clean)).await;
                                    line_buffer.clear();
                                }
                                '\r' => { line_buffer.clear(); }
                                _ => { line_buffer.push(c); }
                            }
                        }
                    }
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
                        for c in text.chars() {
                            match c {
                                '\n' => {
                                    let clean = ansi::strip_ansi(&line_buffer);
                                    let _ = tx_clone.send(StreamMessage::Stderr(clean)).await;
                                    line_buffer.clear();
                                }
                                '\r' => { line_buffer.clear(); }
                                _ => { line_buffer.push(c); }
                            }
                        }
                    }
                    let remaining = utf8_buf.flush();
                    if !remaining.is_empty() {
                        for c in remaining.chars() {
                            match c {
                                '\n' => {
                                    let clean = ansi::strip_ansi(&line_buffer);
                                    let _ = tx_clone.send(StreamMessage::Stderr(clean)).await;
                                    line_buffer.clear();
                                }
                                '\r' => { line_buffer.clear(); }
                                _ => { line_buffer.push(c); }
                            }
                        }
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
        // Very basic validation - compile script block without executing
        let validation_cmd = format!(
            "$ErrorActionPreference = 'Stop'; [scriptblock]::Create('{}'); exit $LASTEXITCODE",
            cmd.replace('\'', "''")
        );
        let result = self.execute(&validation_cmd).await;
        Ok(result.is_ok())
    }

    async fn interrupt(&self) -> Result<()> {
        let pid = *self.active_pid.read();
        if let Some(pid) = pid {
            let status = TokioCommand::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status()
                .await
                .context("Failed to invoke taskkill")?;
            if !status.success() {
                anyhow::bail!("taskkill failed for PID {}", pid);
            }
        }
        Ok(())
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
            active_pid: Arc::clone(&self.active_pid),
        }
    }
}

pub struct PowerShellEnvironmentStatus {
    pub available: bool,
    pub missing_modules: Vec<String>,
}

fn decode_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(bytes, true);
    }

    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(bytes, false);
    }

    if bytes.iter().skip(1).take(8).any(|b| *b == 0) {
        let decoded = decode_utf16(bytes, true);
        if !decoded.is_empty() {
            return decoded;
        }
    }

    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }

    #[cfg(windows)]
    {
        if let Some(decoded) = decode_with_system_codepage(bytes) {
            return decoded;
        }
    }

    String::from_utf8_lossy(bytes).to_string()
}

fn decode_utf16(bytes: &[u8], le: bool) -> String {
    let mut u16_buf = Vec::with_capacity(bytes.len() / 2);
    let mut idx = 0;
    while idx + 1 < bytes.len() {
        let pair = [bytes[idx], bytes[idx + 1]];
        let value = if le {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        };
        u16_buf.push(value);
        idx += 2;
    }
    String::from_utf16_lossy(&u16_buf)
}

#[cfg(windows)]
fn decode_with_system_codepage(bytes: &[u8]) -> Option<String> {
    use windows_sys::Win32::Globalization::{GetACP, GetOEMCP};
    use windows_sys::Win32::System::Console::GetConsoleOutputCP;

    let console_cp = unsafe { GetConsoleOutputCP() };
    let oem_cp = unsafe { GetOEMCP() };
    let ansi_cp = unsafe { GetACP() };

    for cp in [console_cp, oem_cp, ansi_cp] {
        if let Some(encoding) = encoding_for_codepage(cp) {
            let (text, _, _) = encoding.decode(bytes);
            if !text.is_empty() {
                return Some(text.into_owned());
            }
        }
    }

    None
}

#[cfg(windows)]
fn encoding_for_codepage(codepage: u32) -> Option<&'static encoding_rs::Encoding> {
    match codepage {
        65001 => Some(encoding_rs::UTF_8),
        866 => Some(encoding_rs::IBM866),
        1251 => Some(encoding_rs::WINDOWS_1251),
        1252 => Some(encoding_rs::WINDOWS_1252),
        932 => Some(encoding_rs::SHIFT_JIS),
        936 => Some(encoding_rs::GBK),
        949 => Some(encoding_rs::EUC_KR),
        950 => Some(encoding_rs::BIG5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::split_batch_output;

    #[test]
    fn split_batch_output_ok() {
        let sep = "__SEP__";
        let output = format!("{sep}\nfirst\n{sep}\nsecond\n{sep}\n");
        let parts = split_batch_output(&output, sep, 2).expect("split ok");
        assert_eq!(parts, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn split_batch_output_missing() {
        let sep = "__SEP__";
        let output = "no separators here";
        let err = split_batch_output(output, sep, 1).unwrap_err();
        assert!(
            err.to_string().contains("missing separators"),
            "unexpected error: {err}"
        );
    }
}
