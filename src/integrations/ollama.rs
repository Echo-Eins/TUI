use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaData {
    pub available: bool,
    pub models: Vec<OllamaModel>,
    pub running_models: Vec<RunningModel>,
    pub activity_log: Vec<ActivityLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub size_bytes: u64,
    pub size_display: String,
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
            });
        }

        let models = self.list_models().await.unwrap_or_default();
        let running_models = self.list_running().await.unwrap_or_default();

        Ok(OllamaData {
            available: true,
            models,
            running_models,
            activity_log: Vec::new(),
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
        let lines: Vec<&str> = output.lines().collect();

        // Skip header line if present
        for line in lines.iter().skip(1) {
            if line.trim().is_empty() {
                continue;
            }

            // Parse line format: "NAME                            ID              SIZE      MODIFIED"
            // Example: "llama3.2:latest                 a80c4f17acd5    2.0 GB    3 weeks ago"
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let name = parts[0].to_string();
                let size_str = format!("{} {}", parts.get(parts.len() - 3).unwrap_or(&""), parts.get(parts.len() - 2).unwrap_or(&""));
                let size_bytes = self.parse_size_to_bytes(&size_str);

                // Modified is the last 2-3 words
                let modified = parts[parts.len() - 2..].join(" ");

                models.push(OllamaModel {
                    name: name.clone(),
                    size_bytes,
                    size_display: size_str,
                    modified,
                    parameters: None,
                    quantization: None,
                    family: None,
                    format: None,
                });
            }
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
        self.parse_running_models(&stdout)
    }

    fn parse_running_models(&self, output: &str) -> Result<Vec<RunningModel>> {
        let mut running = Vec::new();
        let lines: Vec<&str> = output.lines().collect();

        // Skip header line if present
        for line in lines.iter().skip(1) {
            if line.trim().is_empty() {
                continue;
            }

            // Parse line format: "NAME            ID          SIZE     PROCESSOR       UNTIL"
            // Example: "llama3.2:latest a80c4f17acd5 2.0 GB   100% GPU        4 minutes from now"
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let name = parts[0].to_string();
                let size_str = format!("{} {}", parts.get(2).unwrap_or(&""), parts.get(3).unwrap_or(&""));
                let size_bytes = self.parse_size_to_bytes(&size_str);

                // Find processor info (contains "GPU" or "CPU")
                let processor_idx = parts.iter().position(|&p| p.contains("GPU") || p.contains("CPU"));
                let processor = if let Some(idx) = processor_idx {
                    if idx > 0 {
                        format!("{} {}", parts[idx - 1], parts[idx])
                    } else {
                        parts[idx].to_string()
                    }
                } else {
                    "Unknown".to_string()
                };

                // Until is remaining parts
                let until = if parts.len() > 5 {
                    Some(parts[5..].join(" "))
                } else {
                    None
                };

                running.push(RunningModel {
                    name,
                    size_bytes,
                    size_display: size_str,
                    processor,
                    until,
                });
            }
        }

        Ok(running)
    }

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

    pub async fn run_model(&self, model_name: &str) -> Result<String> {
        // Note: ollama run is interactive, so we just start it in background
        // This is more suitable for triggering via command input
        let output = Command::new(&self.ollama_path)
            .arg("run")
            .arg(model_name)
            .arg("--help")
            .output()
            .context("Failed to execute ollama run")?;

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
