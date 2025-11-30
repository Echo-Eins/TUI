use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::{PowerShellExecutor, LinuxSysMonitor};

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
    linux_sys: LinuxSysMonitor,
}

impl ProcessMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            linux_sys: LinuxSysMonitor::new(),
        })
    }

    pub async fn collect_data(&self) -> Result<ProcessData> {
        #[cfg(target_os = "linux")]
        {
            return self.collect_data_linux().await;
        }

        #[cfg(not(target_os = "linux"))]
        {
            return self.collect_data_windows().await;
        }
    }

    async fn collect_data_linux(&self) -> Result<ProcessData> {
        let linux_processes = self.linux_sys.get_processes()?;

        let processes: Vec<ProcessEntry> = linux_processes
            .into_iter()
            .map(|p| ProcessEntry {
                pid: p.pid,
                name: p.name,
                cpu_usage: 0.0,  // Will calculate later
                memory: p.memory,
                threads: p.threads,
                user: String::from("user"),
                command_line: p.cmdline,
                start_time: None,
                handle_count: 0,
                io_read_bytes: 0,
                io_write_bytes: 0,
            })
            .collect();

        Ok(ProcessData { processes })
    }

    async fn collect_data_windows(&self) -> Result<ProcessData> {
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
