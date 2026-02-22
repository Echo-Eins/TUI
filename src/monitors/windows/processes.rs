use crate::platform::executor::CommandExecutor;
use anyhow::{Context, Result};
use crate::integrations::PowerShellExecutor;
use crate::utils::parse_json_array;
use crate::monitors::types::*;
use crate::monitors::traits::*;
use serde::Deserialize;

pub struct WindowsProcessMonitor {
    ps: PowerShellExecutor,
}

const PROCESSES_SCRIPT: &str = r#"
    try {
        $logical = (Get-CimInstance Win32_ComputerSystem -ErrorAction SilentlyContinue).NumberOfLogicalProcessors
        if (-not $logical -or $logical -le 0) { $logical = [Environment]::ProcessorCount }
        if (-not $logical -or $logical -le 0) { $logical = 1 }

        $perfData = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -ErrorAction SilentlyContinue |
            Where-Object { $_.IDProcess -ne 0 -and $_.Name -ne '_Total' -and $_.Name -ne 'Idle' }

        $wmiProcs = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue
        $wmiMap = @{}
        if ($wmiProcs) {
            foreach ($wp in $wmiProcs) {
                $wmiMap[$wp.ProcessId] = $wp
            }
        }

        $allProcs = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne 0 -and $_.ProcessName -ne 'Idle' }

        $result = foreach ($proc in $allProcs) {
            $pidVal = $proc.Id
            $perf = if ($perfData) {
                $perfData | Where-Object { $_.IDProcess -eq $pidVal } | Select-Object -First 1
            } else { $null }

            $wmi = if ($wmiMap.ContainsKey($pidVal)) { $wmiMap[$pidVal] } else { $null }

            $cpu = 0.0
            $ioRead = [uint64]0
            $ioWrite = [uint64]0

            if ($perf) {
                $cpu = ([double]$perf.PercentProcessorTime) / $logical
                $ioRead = [uint64]$perf.IOReadBytesPersec
                $ioWrite = [uint64]$perf.IOWriteBytesPersec
            }

            $user = "Unknown"
            $cmd = $null
            if ($wmi) {
                $cmd = $wmi.CommandLine
                try {
                    $owner = Invoke-CimMethod -InputObject $wmi -MethodName GetOwner -ErrorAction SilentlyContinue
                    if ($owner -and $owner.User) {
                        $user = if ($owner.Domain) { "$($owner.Domain)\$($owner.User)" } else { $owner.User }
                    }
                } catch {}
            }

            [PSCustomObject]@{
                Pid = [uint32]$pidVal
                Name = $proc.ProcessName
                CpuUsage = [double]$cpu
                Memory = [uint64]$proc.WorkingSet64
                Threads = if ($proc.Threads) { $proc.Threads.Count } else { 0 }
                User = $user
                CommandLine = $cmd
                StartTime = if (Get-Member -InputObject $proc -Name "StartTime" -MemberType Properties) {
                    try { $proc.StartTime.ToString("o") } catch { $null }
                } else { $null }
                HandleCount = if ($proc.HandleCount) { [uint32]$proc.HandleCount } else { [uint32]0 }
                IoReadBytes = $ioRead
                IoWriteBytes = $ioWrite
            }
        }

        $result | Sort-Object CpuUsage -Descending | Select-Object -First 100 | ConvertTo-Json -Depth 2
    } catch {
        "[]"
    }
"#;

impl WindowsProcessMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }
}

impl ProcessMonitorTrait for WindowsProcessMonitor {
    async fn collect_data(&self) -> Result<ProcessData> {
        let output = self
            .ps
            .execute(PROCESSES_SCRIPT)
            .await
            .context("Failed to execute process monitor script")?;

        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(ProcessData {
                processes: Vec::new(),
            });
        }

        let samples: Vec<ProcessSampleWindows> = if trimmed.starts_with('[') {
            parse_json_array(trimmed).context("Failed to parse process list array")?
        } else {
            let single: ProcessSampleWindows =
                serde_json::from_str(trimmed).context("Failed to parse single process")?;
            vec![single]
        };

        let processes = samples
            .into_iter()
            .map(|s| ProcessEntry {
                pid: s.Pid,
                name: s.Name,
                cpu_usage: s.CpuUsage.clamp(0.0, 100.0) as f32,
                memory: s.Memory,
                threads: s.Threads as usize,
                user: s.User.unwrap_or_else(|| "Unknown".to_string()),
                command_line: s.CommandLine,
                start_time: s.StartTime,
                handle_count: s.HandleCount,
                io_read_bytes: s.IoReadBytes,
                io_write_bytes: s.IoWriteBytes,
            })
            .collect();

        Ok(ProcessData { processes })
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct ProcessSampleWindows {
    Pid: u32,
    Name: String,
    CpuUsage: f64,
    Memory: u64,
    Threads: u32,
    User: Option<String>,
    CommandLine: Option<String>,
    StartTime: Option<String>,
    HandleCount: u32,
    IoReadBytes: u64,
    IoWriteBytes: u64,
}

