use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::{PowerShellExecutor, LinuxSysMonitor};
use crate::utils::parse_json_array;

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
    #[allow(dead_code)]
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

    #[allow(dead_code)]
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
            $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -ErrorAction SilentlyContinue |
                Where-Object { $_.IDProcess -ne 0 -and $_.Name -ne '_Total' -and $_.Name -ne 'Idle' }

            $cpuById = @{}
            foreach ($p in $perf) {
                $cpuById[$p.IDProcess] = $p.PercentProcessorTime
            }

            $cimProcs = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue
            $cimById = @{}
            foreach ($proc in $cimProcs) {
                $cimById[$proc.ProcessId] = $proc
            }

            Get-Process | ForEach-Object {
                $cpu = $cpuById[$_.Id]
                $cim = $cimById[$_.Id]

                $user = 'N/A'
                if ($cim) {
                    try {
                        $owner = Invoke-CimMethod -InputObject $cim -MethodName GetOwner -ErrorAction Stop
                        if ($owner -and $owner.User) { $user = $owner.User }
                    } catch {}
                }

                $path = $null
                if ($cim -and $cim.ExecutablePath) {
                    $path = $cim.ExecutablePath
                } elseif ($_.Path) {
                    $path = $_.Path
                }

                $startTime = $null
                try {
                    if ($_.StartTime) { $startTime = $_.StartTime.ToString('o') }
                } catch {}

                $ioRead = 0
                $ioWrite = 0
                try {
                    if ($_.IO) {
                        $ioRead = $_.IO.ReadTransferCount
                        $ioWrite = $_.IO.WriteTransferCount
                    }
                } catch {}

                [PSCustomObject]@{
                    Id = $_.Id
                    ProcessName = $_.ProcessName
                    CpuPercent = if ($null -ne $cpu) { [double]$cpu } else { 0.0 }
                    Threads = if ($_.Threads) { $_.Threads.Count } else { 0 }
                    Memory = [uint64]$_.WorkingSet64
                    User = $user
                    Path = $path
                    StartTime = $startTime
                    HandleCount = $_.HandleCount
                    IOReadBytes = [uint64]$ioRead
                    IOWriteBytes = [uint64]$ioWrite
                }
            } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        let processes: Vec<ProcessSample> = parse_json_array(&output)
            .context("Failed to parse process list")?;
        if processes.is_empty() {
            return Ok(Vec::new());
        }

        Ok(processes
            .into_iter()
            .map(|p| ProcessEntry {
                pid: p.Id,
                name: p.ProcessName,
                cpu_usage: p.CpuPercent.unwrap_or(0.0) as f32,
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
#[allow(non_snake_case)]
struct ProcessSample {
    Id: u32,
    ProcessName: String,
    CpuPercent: Option<f64>,
    Threads: Option<u32>,
    Memory: Option<u64>,
    User: Option<String>,
    Path: Option<String>,
    StartTime: Option<String>,
    HandleCount: Option<u32>,
    IOReadBytes: Option<u64>,
    IOWriteBytes: Option<u64>,
}
