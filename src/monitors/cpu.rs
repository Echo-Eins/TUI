use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::{PowerShellExecutor, LinuxSysMonitor};
use crate::utils::parse_json_array;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuData {
    pub name: String,
    pub overall_usage: f32,
    pub core_count: usize,
    pub thread_count: usize,
    pub core_usage: Vec<CoreUsage>,
    pub frequency: FrequencyInfo,
    pub power: PowerInfo,
    pub temperature: Option<f32>,
    pub top_processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreUsage {
    pub core_id: usize,
    pub usage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrequencyInfo {
    pub base_clock: f32,      // GHz
    pub avg_frequency: f32,   // GHz
    pub max_frequency: f32,   // GHz
    pub boost_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerInfo {
    pub current_power: f32,   // Watts
    pub max_power: f32,       // Watts (TDP)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub threads: usize,
    pub memory: u64,  // Bytes
}

pub struct CpuMonitor {
    ps: PowerShellExecutor,
    #[allow(dead_code)]
    linux_sys: LinuxSysMonitor,
    linux_prev_ticks: Mutex<HashMap<u32, u64>>,
    linux_prev_timestamp: Mutex<Option<Instant>>,
}

const CPU_INFO_SCRIPT: &str = r#"
    try {
        $cpu = Get-CimInstance Win32_Processor -ErrorAction Stop | Select-Object -First 1
        if ($cpu) {
            $cpu | ConvertTo-Json
        } else {
            [PSCustomObject]@{
                Name = "Unknown"
                MaxClockSpeed = 0
                CurrentClockSpeed = 0
                NumberOfCores = 0
                NumberOfLogicalProcessors = 0
                TDP = 65
            } | ConvertTo-Json
        }
    } catch {
        [PSCustomObject]@{
            Name = "Unknown"
            MaxClockSpeed = 0
            CurrentClockSpeed = 0
            NumberOfCores = 0
            NumberOfLogicalProcessors = 0
            TDP = 65
        } | ConvertTo-Json
    }
"#;

const CORE_USAGE_SCRIPT: &str = r#"
    try {
        $cores = Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -ErrorAction Stop |
            Where-Object { $_.Name -ne '_Total' }
        $result = foreach ($core in $cores) {
            [PSCustomObject]@{
                Core = $core.Name
                Usage = [double]$core.PercentProcessorTime
            }
        }
        $result | ConvertTo-Json
    } catch {
        "[]"
    }
"#;

const OVERALL_USAGE_SCRIPT: &str = r#"
    try {
        $total = Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -ErrorAction Stop |
            Where-Object { $_.Name -eq '_Total' } |
            Select-Object -First 1
        if ($total) { $total.PercentProcessorTime } else { 0 }
    } catch {
        0
    }
"#;

const TOP_PROCESSES_SCRIPT: &str = r#"
    try {
        $logical = (Get-CimInstance Win32_ComputerSystem -ErrorAction SilentlyContinue).NumberOfLogicalProcessors
        if (-not $logical -or $logical -le 0) { $logical = [Environment]::ProcessorCount }
        if (-not $logical -or $logical -le 0) { $logical = 1 }

        $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -ErrorAction Stop |
            Where-Object { $_.IDProcess -ne 0 -and $_.Name -ne '_Total' -and $_.Name -ne 'Idle' } |
            Sort-Object PercentProcessorTime -Descending |
            Select-Object -First 5

        $result = foreach ($entry in $perf) {
            $proc = Get-Process -Id $entry.IDProcess -ErrorAction SilentlyContinue
            [PSCustomObject]@{
                Id = [uint32]$entry.IDProcess
                ProcessName = if ($proc) { $proc.ProcessName } else { $entry.Name }
                CpuPercent = [double]$entry.PercentProcessorTime / [double]$logical
                Threads = if ($proc -and $proc.Threads) { $proc.Threads.Count } else { $null }
                Memory = if ($proc) { [uint64]$proc.WorkingSet64 } else { 0 }
            }
        }

        $result | ConvertTo-Json
    } catch {
        "[]"
    }
"#;

const PERF_INFO_SCRIPT: &str = r#"
    try {
        $perf = Get-CimInstance Win32_PerfFormattedData_Counters_ProcessorInformation -ErrorAction Stop
        $entries = $perf | Where-Object { $_.Name -notlike '*_Total' }
        if (-not $entries) { $entries = $perf }

        $avgFreq = ($entries | Measure-Object -Property ProcessorFrequency -Average).Average
        $maxFreq = ($entries | Measure-Object -Property ProcessorFrequency -Maximum).Maximum
        $avgPerf = ($entries | Measure-Object -Property PercentProcessorPerformance -Average).Average
        $avgUtil = ($entries | Measure-Object -Property PercentProcessorUtility -Average).Average

        [PSCustomObject]@{
            AvgFrequency = [double]$avgFreq
            MaxFrequency = [double]$maxFreq
            AvgPerformance = [double]$avgPerf
            AvgUtility = [double]$avgUtil
        } | ConvertTo-Json
    } catch {
        [PSCustomObject]@{
            AvgFrequency = 0
            MaxFrequency = 0
            AvgPerformance = 0
            AvgUtility = 0
        } | ConvertTo-Json
    }
"#;

const TEMPERATURE_SCRIPT: &str = r#"
    try {
        $temps = Get-CimInstance -Namespace "root/wmi" -ClassName MSAcpi_ThermalZoneTemperature -ErrorAction SilentlyContinue |
            Where-Object { $_.CurrentTemperature -gt 0 } |
            ForEach-Object { ($_.CurrentTemperature / 10) - 273.15 }
        if ($temps) {
            $max = ($temps | Measure-Object -Maximum).Maximum
            [math]::Round($max, 1)
        } else {
            ""
        }
    } catch {
        ""
    }
"#;

impl CpuMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            linux_sys: LinuxSysMonitor::new(),
            linux_prev_ticks: Mutex::new(HashMap::new()),
            linux_prev_timestamp: Mutex::new(None),
        })
    }

    pub async fn collect_data(&self) -> Result<CpuData> {
        #[cfg(target_os = "linux")]
        {
            self.collect_data_linux().await
        }

        #[cfg(not(target_os = "linux"))]
        {
            self.collect_data_windows().await
        }
    }

    #[allow(dead_code)]
    async fn collect_data_linux(&self) -> Result<CpuData> {
        let cpu_info = self.linux_sys.get_cpu_info()?;
        let overall_usage = self.linux_sys.get_cpu_usage()?;
        let core_usage_values = self.linux_sys.get_per_core_usage()?;

        let core_usage: Vec<CoreUsage> = core_usage_values
            .iter()
            .enumerate()
            .map(|(i, &usage)| CoreUsage {
                core_id: i,
                usage,
            })
            .collect();

        // Get per-core frequencies for avg calculation
        let per_core_freqs = self.linux_sys.get_per_core_frequencies();
        let avg_freq_mhz = if !per_core_freqs.is_empty() {
            per_core_freqs.iter().sum::<f32>() / per_core_freqs.len() as f32
        } else {
            cpu_info.current_frequency_mhz
        };

        // Determine boost state from sysfs (accurate), with frequency-based fallback
        let boost_active = match self.linux_sys.is_boost_enabled() {
            Some(enabled) => enabled,
            None => avg_freq_mhz > cpu_info.base_frequency_mhz * 1.05,
        };

        let frequency = FrequencyInfo {
            base_clock: cpu_info.base_frequency_mhz / 1000.0,
            avg_frequency: avg_freq_mhz / 1000.0,
            max_frequency: cpu_info.max_frequency_mhz / 1000.0,
            boost_active,
        };

        // Get real temperature
        let temperature = self.linux_sys.get_cpu_temperature();

        // Get power info from RAPL if available
        let (current_power, max_power) = match self.linux_sys.get_cpu_power() {
            Some((_, tdp)) => {
                // Estimate current power from usage and TDP
                let estimated = (overall_usage / 100.0) * tdp;
                (estimated, tdp)
            }
            None => {
                // Fallback: estimate with 65W TDP
                let tdp = 65.0;
                ((overall_usage / 100.0) * tdp, tdp)
            }
        };

        // Get all processes with CPU usage
        let top_processes = self.get_linux_processes()?;

        Ok(CpuData {
            name: cpu_info.name,
            overall_usage,
            core_count: cpu_info.core_count,
            thread_count: cpu_info.thread_count,
            core_usage,
            frequency,
            power: PowerInfo {
                current_power,
                max_power,
            },
            temperature,
            top_processes,
        })
    }

    fn get_linux_processes(&self) -> Result<Vec<ProcessInfo>> {
        let processes = self.linux_sys.get_processes()?;
        let now = Instant::now();

        let clock_ticks_per_sec = 100u64; // sysconf(_SC_CLK_TCK) is typically 100 on Linux
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get() as f64)
            .unwrap_or(1.0);

        let mut prev_ticks = self.linux_prev_ticks.lock();
        let mut prev_ts = self.linux_prev_timestamp.lock();

        let elapsed_secs = prev_ts
            .map(|t| now.saturating_duration_since(t).as_secs_f64())
            .unwrap_or(0.0);

        let mut result: Vec<ProcessInfo> = processes
            .iter()
            .map(|p| {
                let cpu_usage = if elapsed_secs > 0.0 {
                    if let Some(&prev) = prev_ticks.get(&p.pid) {
                        let delta_ticks = p.cpu_ticks.saturating_sub(prev);
                        let delta_secs = delta_ticks as f64 / clock_ticks_per_sec as f64;
                        let usage = (delta_secs / elapsed_secs / num_cpus) * 100.0;
                        (usage.clamp(0.0, 100.0)) as f32
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                ProcessInfo {
                    pid: p.pid,
                    name: p.name.clone(),
                    cpu_usage,
                    threads: p.threads,
                    memory: p.memory,
                }
            })
            .collect();

        // Update previous ticks
        prev_ticks.clear();
        for p in &processes {
            prev_ticks.insert(p.pid, p.cpu_ticks);
        }
        *prev_ts = Some(now);

        // Sort by CPU usage descending
        result.sort_by(|a, b| b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap_or(std::cmp::Ordering::Equal));

        Ok(result)
    }

    async fn collect_data_windows(&self) -> Result<CpuData> {
        let outputs = self
            .ps
            .execute_batch(&[
                CPU_INFO_SCRIPT,
                CORE_USAGE_SCRIPT,
                OVERALL_USAGE_SCRIPT,
                TOP_PROCESSES_SCRIPT,
                PERF_INFO_SCRIPT,
                TEMPERATURE_SCRIPT,
            ])
            .await
            .context("Failed to execute CPU monitor batch")?;

        let cpu_info = Self::parse_cpu_info(&outputs[0])?;
        let core_usage = Self::parse_core_usage(&outputs[1])?;
        let overall_usage = Self::parse_overall_usage(&outputs[2])?;
        let top_processes = Self::parse_top_processes(&outputs[3])?;
        let perf_info = Self::parse_perf_info(&outputs[4])?;
        let temperature = Self::parse_temperature(&outputs[5]).ok();
        let frequency = self.get_frequency_info(&cpu_info, &perf_info)?;
        let power = self.get_power_info(&cpu_info, overall_usage, &perf_info);
        let (core_count, thread_count) = self.get_core_counts(&cpu_info)?;

        Ok(CpuData {
            name: cpu_info.name,
            overall_usage,
            core_count,
            thread_count,
            core_usage,
            frequency,
            power,
            temperature,
            top_processes,
        })
    }

    fn parse_cpu_info(output: &str) -> Result<CpuInfoParsed> {
        let info: Win32Processor = serde_json::from_str(output)
            .context("Failed to parse CPU info")?;

        Ok(CpuInfoParsed {
            name: info.Name,
            max_clock_speed: info.MaxClockSpeed,
            current_clock_speed: info.CurrentClockSpeed,
            number_of_cores: info.NumberOfCores,
            number_of_logical_processors: info.NumberOfLogicalProcessors,
            tdp: info.TDP.unwrap_or(65.0),
        })
    }

    fn parse_overall_usage(output: &str) -> Result<f32> {
        let usage: f32 = output.trim().parse()
            .context("Failed to parse CPU usage")?;

        Ok(usage.min(100.0))
    }

    fn parse_perf_info(output: &str) -> Result<PerfInfo> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        serde_json::from_str(trimmed).context("Failed to parse CPU perf info")
    }

    fn parse_core_usage(output: &str) -> Result<Vec<CoreUsage>> {
        let cores: Vec<CoreSample> = parse_json_array(output)
            .context("Failed to parse core usage")?;
        if cores.is_empty() {
            return Ok(Vec::new());
        }

        Ok(cores
            .into_iter()
            .enumerate()
            .map(|(id, sample)| CoreUsage {
                core_id: sample.Core.parse::<usize>().unwrap_or(id),
                usage: sample.Usage.min(100.0),
            })
            .collect())
    }

    fn get_frequency_info(&self, cpu_info: &CpuInfoParsed, perf: &PerfInfo) -> Result<FrequencyInfo> {
        let base_mhz = cpu_info.max_clock_speed.max(1) as f32;
        let avg_mhz = perf
            .avg_frequency()
            .unwrap_or(cpu_info.current_clock_speed as f32)
            .max(0.0);
        let max_mhz = perf
            .max_frequency()
            .unwrap_or(cpu_info.max_clock_speed as f32)
            .max(base_mhz);
        let avg_perf = perf.avg_performance().unwrap_or(100.0);

        Ok(FrequencyInfo {
            base_clock: base_mhz / 1000.0,
            avg_frequency: avg_mhz / 1000.0,
            max_frequency: max_mhz / 1000.0,
            boost_active: avg_perf > 100.0 || avg_mhz > base_mhz * 1.05,
        })
    }

    fn get_power_info(&self, cpu_info: &CpuInfoParsed, overall_usage: f32, perf: &PerfInfo) -> PowerInfo {
        let util = perf
            .avg_utility()
            .unwrap_or(overall_usage)
            .clamp(0.0, 100.0);
        let current_power = (util / 100.0) * cpu_info.tdp;
        PowerInfo {
            current_power,
            max_power: cpu_info.tdp,
        }
    }

    fn parse_top_processes(output: &str) -> Result<Vec<ProcessInfo>> {
        let processes: Vec<ProcessSample> = parse_json_array(output)
            .context("Failed to parse top processes")?;
        if processes.is_empty() {
            return Ok(Vec::new());
        }

        Ok(processes
            .into_iter()
            .map(|p| ProcessInfo {
                pid: p.Id,
                name: p.ProcessName,
                cpu_usage: (p.CpuPercent.unwrap_or(0.0) as f32).min(100.0),
                threads: p.Threads.unwrap_or(1) as usize,
                memory: p.Memory.unwrap_or(0),
            })
            .collect())
    }

    fn parse_temperature(output: &str) -> Result<f32> {
        let trimmed = output.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
            anyhow::bail!("CPU temperature unavailable");
        }
        let temp: f32 = trimmed.parse().context("Failed to parse CPU temperature")?;
        Ok(temp)
    }

    fn get_core_counts(&self, cpu_info: &CpuInfoParsed) -> Result<(usize, usize)> {
        Ok((
            cpu_info.number_of_cores as usize,
            cpu_info.number_of_logical_processors as usize,
        ))
    }
}

// PowerShell data structures
#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct Win32Processor {
    Name: String,
    MaxClockSpeed: u32,
    CurrentClockSpeed: u32,
    NumberOfCores: u32,
    NumberOfLogicalProcessors: u32,
    TDP: Option<f32>,
}

#[derive(Debug)]
struct CpuInfoParsed {
    name: String,
    max_clock_speed: u32,
    current_clock_speed: u32,
    number_of_cores: u32,
    number_of_logical_processors: u32,
    tdp: f32,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct CoreSample {
    Core: String,
    Usage: f32,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct PerfInfo {
    AvgFrequency: Option<f32>,
    MaxFrequency: Option<f32>,
    AvgPerformance: Option<f32>,
    AvgUtility: Option<f32>,
}

impl PerfInfo {
    fn avg_frequency(&self) -> Option<f32> {
        self.AvgFrequency
    }
    fn max_frequency(&self) -> Option<f32> {
        self.MaxFrequency
    }
    fn avg_performance(&self) -> Option<f32> {
        self.AvgPerformance
    }
    fn avg_utility(&self) -> Option<f32> {
        self.AvgUtility
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
}
