use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessData {
    pub processes: Vec<ProcessEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory: u64,
    pub threads: usize,
}

pub struct ProcessMonitor {
    ps: PowerShellExecutor,
}

impl ProcessMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    pub async fn collect_data(&self) -> Result<ProcessData> {
        let processes = self.get_processes().await?;
        Ok(ProcessData { processes })
    }

    async fn get_processes(&self) -> Result<Vec<ProcessEntry>> {
        let script = r#"
            Get-Process | Sort-Object CPU -Descending | Select-Object -First 20 Id, ProcessName, CPU, Threads, @{Name='Memory';Expression={$_.WorkingSet64}} | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let processes: Vec<ProcessSample> = serde_json::from_str(&output).unwrap_or_default();

        Ok(processes
            .into_iter()
            .map(|p| ProcessEntry {
                pid: p.Id,
                name: p.ProcessName,
                cpu_usage: p.CPU.unwrap_or(0.0) as f32,
                memory: p.Memory.unwrap_or(0),
                threads: p.Threads.unwrap_or(1) as usize,
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct ProcessSample {
    Id: u32,
    ProcessName: String,
    CPU: Option<f64>,
    Threads: Option<u32>,
    Memory: Option<u64>,
}
