use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::utils::json::parse_json_array;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaData {
    pub available: bool,
    pub models: Vec<OllamaModel>,
    pub running_models: Vec<RunningModel>,
    pub activity_log: Vec<ActivityLogEntry>,
    pub chat_logs: Vec<ChatLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub size_bytes: u64,
    pub size_display: String,
    pub params_value: Option<f64>,
    pub params_unit: Option<char>,
    pub params_display: String,
    pub modified: String,
    pub parameters: Option<String>,
    pub quantization: Option<String>,
    pub family: Option<String>,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningModel {
    pub name: String,
    pub size_bytes: u64,
    pub size_display: String,
    pub gpu_memory_mb: Option<u64>,
    pub gpu_memory_display: String,
    pub params_value: Option<f64>,
    pub params_unit: Option<char>,
    pub params_display: String,
    pub processor: String, // "100% GPU" or "CPU/GPU split"
    pub until: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityLogEntry {
    pub timestamp: u64,
    pub action: String,
    pub details: String,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatLogEntry {
    pub model: String,
    pub ended_at: u64,
    pub ended_at_display: String,
    pub path: String,
    pub last_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatLogMetadata {
    pub model: String,
    pub ended_at: u64,
    pub ended_at_display: String,
    pub paused_at: Option<u64>,
    pub paused_at_display: Option<String>,
    pub last_user_prompt: String,
    pub message_count: usize,
    pub total_turns: usize,
}

pub struct OllamaClient {
    ollama_path: String,
}

impl OllamaClient {
    pub fn new(ollama_path: Option<String>) -> Result<Self> {
        let path = ollama_path.unwrap_or_else(|| "ollama".to_string());
        Ok(Self { ollama_path: path })
    }

    pub async fn collect_data(&mut self) -> Result<OllamaData> {
        let available = self.check_availability().await;

        if !available {
            return Ok(OllamaData {
                available: false,
                models: Vec::new(),
                running_models: Vec::new(),
                activity_log: Vec::new(),
                chat_logs: Vec::new(),
            });
        }

        let models = self.list_models().await.unwrap_or_default();
        let running_models = self.list_running().await.unwrap_or_default();
        let chat_logs = self.list_chat_logs().unwrap_or_default();

        Ok(OllamaData {
            available: true,
            models,
            running_models,
            activity_log: Vec::new(),
            chat_logs,
        })
    }

    pub async fn check_availability(&self) -> bool {
        match Command::new(&self.ollama_path).arg("--version").output() {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<OllamaModel>> {
        let output = Command::new(&self.ollama_path)
            .arg("list")
            .output()
            .context("Failed to execute ollama list")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_model_list(&stdout)
    }

    fn parse_model_list(&self, output: &str) -> Result<Vec<OllamaModel>> {
        let mut models = Vec::new();
        let mut lines = output.lines().filter(|line| !line.trim().is_empty());

        let header = match lines.next() {
            Some(line) => line,
            None => return Ok(models),
        };
        let headers = split_columns(header);
        let name_idx = find_column(&headers, "NAME").unwrap_or(0);
        let size_idx = find_column(&headers, "SIZE").unwrap_or(2);
        let modified_idx = find_column(&headers, "MODIFIED").unwrap_or(3);

        for line in lines {
            let cols = split_columns(line);
            if cols.is_empty() || cols.len() <= name_idx {
                continue;
            }

            let name = cols[name_idx].trim().to_string();
            if name.is_empty() {
                continue;
            }

            let size_raw = cols.get(size_idx).map(String::as_str).unwrap_or("-");
            let (size_display, size_bytes) = self.normalize_size(size_raw);

            let modified = if let Some(value) = cols.get(modified_idx) {
                value.trim().to_string()
            } else if cols.len() > size_idx + 1 {
                cols[size_idx + 1..].join(" ").trim().to_string()
            } else {
                String::new()
            };

            let (params_value, params_unit, params_display) =
                parse_model_params_from_name(&name);

            models.push(OllamaModel {
                name,
                size_bytes,
                size_display,
                params_value,
                params_unit,
                params_display,
                modified,
                parameters: None,
                quantization: None,
                family: None,
                format: None,
            });
        }

        Ok(models)
    }
    pub async fn list_running(&self) -> Result<Vec<RunningModel>> {
        let output = Command::new(&self.ollama_path)
            .arg("ps")
            .output()
            .context("Failed to execute ollama ps")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut running = self.parse_running_models(&stdout)?;
        self.attach_gpu_memory(&mut running);
        Ok(running)
    }

    fn parse_running_models(&self, output: &str) -> Result<Vec<RunningModel>> {
        let mut running = Vec::new();
        let mut lines = output.lines().filter(|line| !line.trim().is_empty());

        let header = match lines.next() {
            Some(line) => line,
            None => return Ok(running),
        };
        let headers = split_columns(header);
        let name_idx = find_column(&headers, "NAME").unwrap_or(0);
        let size_idx = find_column(&headers, "SIZE").unwrap_or(2);
        let processor_idx = find_column(&headers, "PROCESSOR");
        let until_idx = find_column(&headers, "UNTIL");

        for line in lines {
            let cols = split_columns(line);
            if cols.is_empty() || cols.len() <= name_idx {
                continue;
            }

            let name = cols[name_idx].trim().to_string();
            if name.is_empty() {
                continue;
            }

            let size_raw = cols.get(size_idx).map(String::as_str).unwrap_or("-");
            let (size_display, size_bytes) = self.normalize_size(size_raw);

            let processor = processor_idx
                .and_then(|idx| cols.get(idx))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Unknown".to_string());

            let until = until_idx
                .and_then(|idx| cols.get(idx))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty() && value != "-");

            let (params_value, params_unit, params_display) =
                parse_model_params_from_name(&name);

            running.push(RunningModel {
                name,
                size_bytes,
                size_display,
                gpu_memory_mb: None,
                gpu_memory_display: "-".to_string(),
                params_value,
                params_unit,
                params_display,
                processor,
                until,
            });
        }

        Ok(running)
    }

    fn attach_gpu_memory(&self, running: &mut [RunningModel]) {
        let gpu_processes = self.query_nvidia_smi_processes();
        if gpu_processes.is_empty() {
            for model in running.iter_mut() {
                if is_cloud_model(&model.name) {
                    model.gpu_memory_display = "cloud".to_string();
                }
            }
            return;
        }

        let pids: Vec<u32> = gpu_processes.keys().copied().collect();
        let command_lines = self.query_process_command_lines(&pids);

        for model in running.iter_mut() {
            if is_cloud_model(&model.name) {
                model.gpu_memory_display = "cloud".to_string();
                model.gpu_memory_mb = None;
                continue;
            }

            let name_lower = model.name.to_ascii_lowercase();
            let mut total_mb = 0u64;
            for (pid, memory_mb) in gpu_processes.iter() {
                if let Some(command_line) = command_lines.get(pid) {
                    if command_line.to_ascii_lowercase().contains(&name_lower) {
                        total_mb += memory_mb;
                    }
                }
            }

            if total_mb > 0 {
                model.gpu_memory_mb = Some(total_mb);
                model.gpu_memory_display = format_mb_as_gb(total_mb);
            } else {
                model.gpu_memory_mb = None;
                model.gpu_memory_display = "-".to_string();
            }
        }
    }

    fn extract_last_prompt_from_path(&self, path: &PathBuf) -> Option<String> {
        let content = fs::read_to_string(path).ok()?;
        let lines: Vec<String> = content.lines().map(|line| line.to_string()).collect();
        extract_last_prompt_from_lines(&lines)
    }

    fn read_chat_metadata_from_path(&self, path: &PathBuf) -> Option<ChatLogMetadata> {
        let meta_path = chat_log_meta_path(path);
        let content = fs::read_to_string(meta_path).ok()?;
        toml::from_str::<ChatLogMetadata>(&content).ok()
    }

    fn query_nvidia_smi_processes(&self) -> HashMap<u32, u64> {
        let output = Command::new("nvidia-smi")
            .args([
                "--query-compute-apps=pid,used_memory",
                "--format=csv,noheader,nounits",
            ])
            .output();
        let Ok(output) = output else {
            return HashMap::new();
        };
        if !output.status.success() {
            return HashMap::new();
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut map = HashMap::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split(',').map(|part| part.trim()).collect();
            if parts.len() < 2 {
                continue;
            }
            let Ok(pid) = parts[0].parse::<u32>() else {
                continue;
            };
            let Ok(memory_mb) = parts[1].parse::<u64>() else {
                continue;
            };
            map.insert(pid, memory_mb);
        }
        map
    }

    fn query_process_command_lines(&self, pids: &[u32]) -> HashMap<u32, String> {
        if !cfg!(target_os = "windows") || pids.is_empty() {
            return HashMap::new();
        }

        let filter = pids
            .iter()
            .map(|pid| format!("ProcessId={pid}"))
            .collect::<Vec<_>>()
            .join(" or ");
        let command = format!(
            "Get-CimInstance Win32_Process -Filter \"{}\" | Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress",
            filter
        );

        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &command])
            .output();
        let Ok(output) = output else {
            return HashMap::new();
        };
        if !output.status.success() {
            return HashMap::new();
        }

        #[derive(Deserialize)]
        struct ProcessCommandLine {
            #[serde(rename = "ProcessId")]
            process_id: u32,
            #[serde(rename = "CommandLine")]
            command_line: Option<String>,
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let entries: Vec<ProcessCommandLine> = parse_json_array(&stdout).unwrap_or_default();
        entries
            .into_iter()
            .filter_map(|entry| {
                entry
                    .command_line
                    .map(|line| (entry.process_id, line))
            })
            .collect()
    }
    #[allow(dead_code)]
    pub async fn show_model(&self, model_name: &str) -> Result<String> {
        let output = Command::new(&self.ollama_path)
            .arg("show")
            .arg(model_name)
            .output()
            .context("Failed to execute ollama show")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to show model: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub async fn run_model(&self, model_name: &str, prompt: &str) -> Result<String> {
        let mut command = Command::new(&self.ollama_path);
        command.arg("run").arg(model_name);
        if !prompt.trim().is_empty() {
            command.arg(prompt);
        }
        let output = command.output().context("Failed to execute ollama run")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to run model: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub async fn stop_model(&self, model_name: &str) -> Result<()> {
        let output = Command::new(&self.ollama_path)
            .arg("stop")
            .arg(model_name)
            .output()
            .context("Failed to execute ollama stop")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to stop model: {}", stderr));
        }

        Ok(())
    }

    pub async fn remove_model(&self, model_name: &str) -> Result<()> {
        let output = Command::new(&self.ollama_path)
            .arg("rm")
            .arg(model_name)
            .output()
            .context("Failed to execute ollama rm")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to remove model: {}", stderr));
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn pull_model(&self, model_name: &str) -> Result<String> {
        let output = Command::new(&self.ollama_path)
            .arg("pull")
            .arg(model_name)
            .output()
            .context("Failed to execute ollama pull")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to pull model: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub fn list_chat_logs(&self) -> Result<Vec<ChatLogEntry>> {
        let dir = chat_log_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(&dir).context("Failed to read chat log directory")? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("log") {
                continue;
            }
            let stem = match path.file_stem().and_then(|name| name.to_str()) {
                Some(name) => name,
                None => continue,
            };

            let (ended_at_dt, model) = match parse_log_filename(stem) {
                Some((dt, model)) => (dt, model),
                None => {
                    let modified = entry
                        .metadata()
                        .and_then(|meta| meta.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    let dt: DateTime<Local> = DateTime::from(modified);
                    (dt, "Unknown".to_string())
                }
            };
            let metadata = self.read_chat_metadata_from_path(&path);
            let last_prompt = metadata
                .as_ref()
                .map(|meta| meta.last_user_prompt.clone())
                .filter(|prompt| !prompt.trim().is_empty())
                .or_else(|| self.extract_last_prompt_from_path(&path))
                .unwrap_or_else(String::new);

            entries.push(ChatLogEntry {
                model,
                ended_at: ended_at_dt.timestamp() as u64,
                ended_at_display: format_log_timestamp(ended_at_dt),
                path: path.to_string_lossy().to_string(),
                last_prompt,
            });
        }

        entries.sort_by(|a, b| b.ended_at.cmp(&a.ended_at));
        Ok(entries)
    }

    pub fn save_chat_log(&self, model_name: &str, content: &str) -> Result<ChatLogEntry> {
        self.save_chat_log_prefixed("", model_name, content)
    }

    pub fn save_chat_log_prefixed(
        &self,
        prefix: &str,
        model_name: &str,
        content: &str,
    ) -> Result<ChatLogEntry> {
        let now = Local::now();
        let dir = chat_log_dir();
        fs::create_dir_all(&dir).context("Failed to create chat log directory")?;

        let prefix = prefix.trim();
        let file_name = if prefix.is_empty() {
            build_log_filename(now, model_name)
        } else {
            build_log_filename_with_prefix(Some(prefix), now, model_name)
        };
        let path = dir.join(file_name);
        fs::write(&path, content).context("Failed to write chat log")?;

        Ok(ChatLogEntry {
            model: model_name.to_string(),
            ended_at: now.timestamp() as u64,
            ended_at_display: format_log_timestamp(now),
            path: path.to_string_lossy().to_string(),
            last_prompt: String::new(),
        })
    }

    pub fn write_chat_metadata(
        &self,
        log_path: &str,
        metadata: &ChatLogMetadata,
    ) -> Result<()> {
        let path = PathBuf::from(log_path);
        let meta_path = chat_log_meta_path(&path);
        let content =
            toml::to_string_pretty(metadata).context("Failed to serialize chat metadata")?;
        fs::write(&meta_path, content).context("Failed to write chat metadata")?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn execute_command(&self, command: &str) -> Result<String> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Err(anyhow::anyhow!("Empty command"));
        }

        let output = Command::new(&self.ollama_path)
            .args(&parts)
            .output()
            .context("Failed to execute ollama command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Command failed: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn parse_size_to_bytes(&self, size_str: &str) -> u64 {
        let parts: Vec<&str> = size_str.split_whitespace().collect();
        if parts.len() < 2 {
            return 0;
        }

        let value: f64 = parts[0].parse().unwrap_or(0.0);
        let unit = parts[1].to_uppercase();

        match unit.as_str() {
            "B" => value as u64,
            "KB" => (value * 1024.0) as u64,
            "MB" => (value * 1024.0 * 1024.0) as u64,
            "GB" => (value * 1024.0 * 1024.0 * 1024.0) as u64,
            "TB" => (value * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64,
            _ => 0,
        }
    }

    fn normalize_size(&self, raw: &str) -> (String, u64) {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "-" {
            return ("-".to_string(), 0);
        }
        let size_bytes = self.parse_size_to_bytes(trimmed);
        (trimmed.to_string(), size_bytes)
    }
    #[allow(dead_code)]
    pub fn add_log_entry(&mut self, action: String, details: String, success: bool) -> ActivityLogEntry {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        ActivityLogEntry {
            timestamp,
            action,
            details,
            success,
        }
    }
}

fn chat_log_dir() -> PathBuf {
    PathBuf::from("logs").join("ollama")
}

fn chat_log_meta_path(log_path: &PathBuf) -> PathBuf {
    log_path.with_extension("toml")
}

fn build_log_filename(timestamp: DateTime<Local>, model_name: &str) -> String {
    build_log_filename_with_prefix(None, timestamp, model_name)
}

fn build_log_filename_with_prefix(
    prefix: Option<&str>,
    timestamp: DateTime<Local>,
    model_name: &str,
) -> String {
    let ts = timestamp.format("%Y-%m-%d_%H-%M-%S");
    let encoded_model = encode_filename_component(model_name);
    let prefix = prefix.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    match prefix {
        Some(value) => format!("{}_{}__{}.log", value, ts, encoded_model),
        None => format!("{}__{}.log", ts, encoded_model),
    }
}

fn parse_log_filename(stem: &str) -> Option<(DateTime<Local>, String)> {
    let (timestamp_raw, model) = stem.split_once("__")?;
    let timestamp = if timestamp_raw.len() >= 21 {
        let bytes = timestamp_raw.as_bytes();
        if bytes.get(1) == Some(&b'_') && bytes[0].is_ascii_alphabetic() {
            &timestamp_raw[2..]
        } else {
            timestamp_raw
        }
    } else {
        timestamp_raw
    };
    let naive = NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d_%H-%M-%S").ok()?;
    let dt = Local
        .from_local_datetime(&naive)
        .single()
        .unwrap_or_else(|| Local.from_utc_datetime(&naive));
    let decoded_model = decode_filename_component(model);
    Some((dt, decoded_model))
}

fn encode_filename_component(input: &str) -> String {
    let mut out = String::new();
    for &byte in input.as_bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

fn decode_filename_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    out.push(value);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn format_log_timestamp(timestamp: DateTime<Local>) -> String {
    timestamp.format("%Y-%m-%d %H:%M").to_string()
}

fn is_cloud_model(model_name: &str) -> bool {
    model_name.to_ascii_lowercase().contains("cloud")
}

fn format_mb_as_gb(total_mb: u64) -> String {
    if total_mb < 1024 {
        return format!("{total_mb} MB");
    }
    let gb = total_mb as f64 / 1024.0;
    format!("{:.2} GB", gb)
}

fn parse_model_params_from_name(name: &str) -> (Option<f64>, Option<char>, String) {
    let chars: Vec<char> = name.chars().collect();
    for (idx, ch) in chars.iter().enumerate() {
        let unit = ch.to_ascii_uppercase();
        if !matches!(unit, 'M' | 'B' | 'T') {
            continue;
        }
        if idx == 0 {
            continue;
        }
        let mut start = idx;
        while start > 0 {
            let prev = chars[start - 1];
            if prev.is_ascii_digit() || prev == '.' {
                start -= 1;
            } else {
                break;
            }
        }
        if start == idx {
            continue;
        }
        let num_str: String = chars[start..idx].iter().collect();
        if let Ok(value) = num_str.parse::<f64>() {
            let display = format_param_display(value, unit);
            return (Some(value), Some(unit), display);
        }
    }
    (None, None, "-".to_string())
}

fn format_param_display(value: f64, unit: char) -> String {
    if (value.fract() - 0.0).abs() < f64::EPSILON {
        format!("{:.0}{}", value, unit)
    } else {
        let mut text = format!("{:.2}", value);
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        format!("{text}{unit}")
    }
}

fn extract_last_prompt_from_lines(lines: &[String]) -> Option<String> {
    const USER_PREFIXES: [&str; 3] = ["Запрос:", "Р—Р°РїСЂРѕСЃ:", "Request:"];
    const ASSIST_PREFIXES: [&str; 3] = ["Ответ:", "РћС‚РІРµС‚:", "Response:"];

    let mut current = String::new();
    let mut in_prompt = false;
    let mut last_prompt: Option<String> = None;

    for raw_line in lines {
        let line = raw_line.trim_end().trim_start_matches('\u{feff}');
        if let Some(prefix) = match_prefix(line, &USER_PREFIXES) {
            if in_prompt && !current.is_empty() {
                last_prompt = Some(current.trim_end().to_string());
            }
            current = line[prefix.len()..].trim_start().to_string();
            in_prompt = true;
            continue;
        }
        if match_prefix(line, &ASSIST_PREFIXES).is_some() {
            if in_prompt && !current.is_empty() {
                last_prompt = Some(current.trim_end().to_string());
            }
            current.clear();
            in_prompt = false;
            continue;
        }
        if in_prompt {
            let continuation = line.strip_prefix("  ").unwrap_or(line);
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(continuation);
        }
    }

    if in_prompt && !current.is_empty() {
        last_prompt = Some(current.trim_end().to_string());
    }

    last_prompt
}

fn match_prefix<'a>(line: &'a str, prefixes: &[&'a str]) -> Option<&'a str> {
    for prefix in prefixes {
        if line.starts_with(prefix) {
            return Some(prefix);
        }
    }
    None
}
fn split_columns(line: &str) -> Vec<String> {
    let mut columns = Vec::new();
    let mut buffer = String::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            let mut whitespace = 1;
            while let Some(next) = chars.peek() {
                if next.is_whitespace() {
                    whitespace += 1;
                    chars.next();
                } else {
                    break;
                }
            }
            if whitespace >= 2 {
                if !buffer.trim().is_empty() {
                    columns.push(buffer.trim().to_string());
                }
                buffer.clear();
                continue;
            }
            buffer.push(' ');
        } else {
            buffer.push(ch);
        }
    }

    if !buffer.trim().is_empty() {
        columns.push(buffer.trim().to_string());
    }

    columns
}

fn find_column(headers: &[String], name: &str) -> Option<usize> {
    headers
        .iter()
        .position(|header| header.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::OllamaClient;
    use super::{extract_last_prompt_from_lines, parse_model_params_from_name};

    #[test]
    fn parse_model_list_columns() {
        let client = OllamaClient {
            ollama_path: "ollama".to_string(),
        };
        let output = "\
NAME                          ID              SIZE      MODIFIED\n\
granite4:micro-h               076afb3855dc    1.9 GB    4 weeks ago\n\
nomic-embed-text:latest        0a109f422b47    274 MB    4 weeks ago\n\
gemini-3-pro-preview:latest    91a1db042ba1    -         5 weeks ago\n";

        let models = client.parse_model_list(output).expect("parse ok");
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].name, "granite4:micro-h");
        assert_eq!(models[0].size_display, "1.9 GB");
        assert!(models[0].size_bytes > 0);
        assert_eq!(models[0].modified, "4 weeks ago");
        assert_eq!(models[2].size_display, "-");
        assert_eq!(models[2].size_bytes, 0);
        assert_eq!(models[2].modified, "5 weeks ago");
    }

    #[test]
    fn parse_running_models_columns() {
        let client = OllamaClient {
            ollama_path: "ollama".to_string(),
        };
        let output = "\
NAME            ID              SIZE     PROCESSOR    CONTEXT    UNTIL\n\
llama3:latest    a80c4f17acd5    2.0 GB   100% GPU     4096       44 minutes from now\n\
qwen:latest      123456789abc    1.2 GB   CPU/GPU      2048       -\n";

        let running = client.parse_running_models(output).expect("parse ok");
        assert_eq!(running.len(), 2);
        assert_eq!(running[0].name, "llama3:latest");
        assert_eq!(running[0].size_display, "2.0 GB");
        assert_eq!(running[0].processor, "100% GPU");
        assert_eq!(running[0].until.as_deref(), Some("44 minutes from now"));
        assert_eq!(running[1].until, None);
    }

    #[test]
    fn parse_model_params_variants() {
        let (value, unit, display) = parse_model_params_from_name("llama3:70b");
        assert_eq!(value, Some(70.0));
        assert_eq!(unit, Some('B'));
        assert_eq!(display, "70B");

        let (value, unit, display) = parse_model_params_from_name("qwen2:1.5b");
        assert_eq!(value, Some(1.5));
        assert_eq!(unit, Some('B'));
        assert_eq!(display, "1.5B");

        let (value, unit, display) = parse_model_params_from_name("model-32m");
        assert_eq!(value, Some(32.0));
        assert_eq!(unit, Some('M'));
        assert_eq!(display, "32M");

        let (value, unit, display) = parse_model_params_from_name("no-params");
        assert_eq!(value, None);
        assert_eq!(unit, None);
        assert_eq!(display, "-");
    }

    #[test]
    fn extract_last_prompt_from_log_lines() {
        let lines = vec![
            "Запрос: First question".to_string(),
            "Ответ: First answer".to_string(),
            "Запрос: Second question".to_string(),
            "  with extra context".to_string(),
            "Ответ: Second answer".to_string(),
            "Запрос: Final question".to_string(),
        ];
        let prompt = extract_last_prompt_from_lines(&lines).expect("prompt");
        assert_eq!(prompt, "Final question");

        let lines = vec![
            "Запрос: Multiline".to_string(),
            "  prompt line two".to_string(),
            "Ответ: Done".to_string(),
        ];
        let prompt = extract_last_prompt_from_lines(&lines).expect("prompt");
        assert_eq!(prompt, "Multiline\nprompt line two");
    }
}




