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
    pub user: String,
    pub command_line: Option<String>,
    pub start_time: Option<String>,
    pub handle_count: u32,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
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
            Get-Process | Select-Object -Property `
                Id,
                ProcessName,
                CPU,
                Threads,
                @{Name='Memory';Expression={$_.WorkingSet64}},
                @{Name='User';Expression={
                    try {
                        $_.GetOwner().User
                    } catch {
                        'N/A'
                    }
                }},
                Path,
                StartTime,
                HandleCount,
                @{Name='IOReadBytes';Expression={
                    try {
                        $_.IO.ReadTransferCount
                    } catch {
                        0
                    }
                }},
                @{Name='IOWriteBytes';Expression={
                    try {
                        $_.IO.WriteTransferCount
                    } catch {
                        0
                    }
                }} | ConvertTo-Json
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
                user: p.User.unwrap_or_else(|| "N/A".to_string()),
                command_line: p.Path,
                start_time: p.StartTime,
                handle_count: p.HandleCount.unwrap_or(0),
                io_read_bytes: p.IOReadBytes.unwrap_or(0),
                io_write_bytes: p.IOWriteBytes.unwrap_or(0),
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
    User: Option<String>,
    Path: Option<String>,
    StartTime: Option<String>,
    HandleCount: Option<u32>,
    IOReadBytes: Option<u64>,
    IOWriteBytes: Option<u64>,
}
