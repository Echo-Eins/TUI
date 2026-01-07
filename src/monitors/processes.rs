use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::{PowerShellExecutor, LinuxSysMonitor};
use crate::utils::parse_json_array;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::Instant;

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
    last_cpu_times: Mutex<HashMap<u32, f64>>,
    last_timestamp: Mutex<Option<Instant>>,
}

impl ProcessMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            linux_sys: LinuxSysMonitor::new(),
            last_cpu_times: Mutex::new(HashMap::new()),
            last_timestamp: Mutex::new(None),
        })
    }

    pub async fn collect_data(&mut self) -> Result<ProcessData> {
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

    async fn collect_data_windows(&mut self) -> Result<ProcessData> {
        let samples = self.get_process_samples().await?;
        let processes = self.build_process_entries(samples);
        Ok(ProcessData { processes })
    }

    async fn get_process_samples(&self) -> Result<Vec<ProcessSample>> {
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
                    CpuTimeSeconds = if ($null -ne $_.CPU) { [double]$_.CPU } else { 0.0 }
                    Threads = if ($_.Threads) { $_.Threads.Count } else { 0 }
                    Memory = [uint64]$_.WorkingSet64
                    User = $user
                    SessionId = $_.SessionId
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
        Ok(processes)
    }
}

impl ProcessMonitor {
    fn build_process_entries(&self, samples: Vec<ProcessSample>) -> Vec<ProcessEntry> {
        if samples.is_empty() {
            return Vec::new();
        }

        let now = Instant::now();
        let cpu_count = std::thread::available_parallelism()
            .map(|count| count.get() as f64)
            .unwrap_or(1.0)
            .max(1.0);

        let mut last_timestamp = self.last_timestamp.lock();
        let mut last_cpu_times = self.last_cpu_times.lock();
        let time_delta = last_timestamp
            .as_ref()
            .map(|t| now.duration_since(*t).as_secs_f64())
            .unwrap_or(0.0);

        let mut entries = Vec::with_capacity(samples.len());
        let mut current_cpu_times = HashMap::new();

        for sample in samples {
            let cpu_time = sample.CpuTimeSeconds.unwrap_or(0.0);
            current_cpu_times.insert(sample.Id, cpu_time);

            let mut cpu_usage = sample.CpuPercent.unwrap_or(0.0);
            if time_delta > 0.0 {
                if let Some(prev) = last_cpu_times.get(&sample.Id) {
                    let delta = (cpu_time - prev).max(0.0);
                    let computed = (delta / time_delta) * 100.0 / cpu_count;
                    if computed.is_finite() {
                        cpu_usage = computed;
                    }
                }
            }

            if !cpu_usage.is_finite() || cpu_usage < 0.0 {
                cpu_usage = 0.0;
            }
            if cpu_usage > 100.0 {
                cpu_usage = 100.0;
            }

            let user = normalize_user(sample.User, sample.SessionId);

            entries.push(ProcessEntry {
                pid: sample.Id,
                name: sample.ProcessName,
                cpu_usage: cpu_usage as f32,
                memory: sample.Memory.unwrap_or(0),
                threads: sample.Threads.unwrap_or(1) as usize,
                user,
                command_line: sample.Path,
                start_time: sample.StartTime,
                handle_count: sample.HandleCount.unwrap_or(0),
                io_read_bytes: sample.IOReadBytes.unwrap_or(0),
                io_write_bytes: sample.IOWriteBytes.unwrap_or(0),
            });
        }

        *last_timestamp = Some(now);
        *last_cpu_times = current_cpu_times;

        entries
    }
}

fn normalize_user(user: Option<String>, session_id: Option<u32>) -> String {
    if let Some(value) = user {
        let trimmed = value.trim();
        if !trimmed.is_empty() && trimmed != "N/A" {
            return trimmed.to_string();
        }
    }

    if session_id == Some(0) {
        "SYSTEM".to_string()
    } else {
        "USER".to_string()
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct ProcessSample {
    Id: u32,
    ProcessName: String,
    CpuPercent: Option<f64>,
    CpuTimeSeconds: Option<f64>,
    Threads: Option<u32>,
    Memory: Option<u64>,
    User: Option<String>,
    SessionId: Option<u32>,
    Path: Option<String>,
    StartTime: Option<String>,
    HandleCount: Option<u32>,
    IOReadBytes: Option<u64>,
    IOWriteBytes: Option<u64>,
}
