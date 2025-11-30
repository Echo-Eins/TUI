use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::{PowerShellExecutor, LinuxSysMonitor};

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
    linux_sys: LinuxSysMonitor,
}

impl CpuMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            linux_sys: LinuxSysMonitor::new(),
        })
    }

    pub async fn collect_data(&self) -> Result<CpuData> {
        // Check if we're on Linux - use linux_sys, otherwise use PowerShell
        #[cfg(target_os = "linux")]
        {
            self.collect_data_linux().await
        }

        #[cfg(not(target_os = "linux"))]
        {
            self.collect_data_windows().await
        }
    }

    async fn collect_data_linux(&self) -> Result<CpuData> {
        let cpu_info = self.linux_sys.get_cpu_info()?;
        let overall_usage = self.linux_sys.get_cpu_usage()?;
        let core_usage_values = self.linux_sys.get_core_usage()?;

        let core_usage: Vec<CoreUsage> = core_usage_values
            .iter()
            .enumerate()
            .map(|(i, &usage)| CoreUsage {
                core_id: i,
                usage,
            })
            .collect();

        let frequency = FrequencyInfo {
            base_clock: cpu_info.frequency_mhz / 1000.0,
            avg_frequency: cpu_info.frequency_mhz / 1000.0,
            max_frequency: cpu_info.frequency_mhz / 1000.0,
            boost_active: false,
        };

        Ok(CpuData {
            name: cpu_info.name,
            overall_usage,
            core_count: cpu_info.core_count,
            thread_count: cpu_info.core_count,
            core_usage,
            frequency,
            power: PowerInfo {
                current_power: (overall_usage / 100.0) * 65.0,  // Assume 65W TDP
                max_power: 65.0,
            },
            temperature: Some(50.0),  // Placeholder
            top_processes: Vec::new(),  // Will implement later
        })
    }

    async fn collect_data_windows(&self) -> Result<CpuData> {
        // Get CPU info
        let cpu_info = self.get_cpu_info().await?;

        // Get per-core usage
        let core_usage = self.get_core_usage().await?;

        // Get overall CPU usage
        let overall_usage = self.get_overall_usage().await?;

        // Get frequency info
        let frequency = self.get_frequency_info(&cpu_info).await?;

        // Get top processes
        let top_processes = self.get_top_processes().await?;

        // Try to get temperature
        let temperature = self.get_temperature().await.ok();

        // Get core counts
        let (core_count, thread_count) = self.get_core_counts(&cpu_info)?;

        Ok(CpuData {
            name: cpu_info.name,
            overall_usage,
            core_count,
            thread_count,
            core_usage,
            frequency,
            power: PowerInfo {
                current_power: (overall_usage / 100.0) * cpu_info.tdp,
                max_power: cpu_info.tdp,
            },
            temperature,
            top_processes,
        })
    }

    async fn get_cpu_info(&self) -> Result<CpuInfo> {
        let script = r#"
            Get-CimInstance Win32_Processor | Select-Object -First 1 | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        let info: Win32Processor = serde_json::from_str(&output)
            .context("Failed to parse CPU info")?;

        Ok(CpuInfo {
            name: info.Name,
            max_clock_speed: info.MaxClockSpeed,
            current_clock_speed: info.CurrentClockSpeed,
            number_of_cores: info.NumberOfCores,
            number_of_logical_processors: info.NumberOfLogicalProcessors,
            tdp: info.TDP.unwrap_or(65.0), // Default TDP if not available
        })
    }

    async fn get_overall_usage(&self) -> Result<f32> {
        let script = r#"
            (Get-Counter '\Processor(_Total)\% Processor Time').CounterSamples[0].CookedValue
        "#;

        let output = self.ps.execute(script).await?;
        let usage: f32 = output.trim().parse()
            .context("Failed to parse CPU usage")?;

        Ok(usage.min(100.0))
    }

    async fn get_core_usage(&self) -> Result<Vec<CoreUsage>> {
        let script = r#"
            $cores = Get-Counter '\Processor(*)\% Processor Time' |
                     Select-Object -ExpandProperty CounterSamples |
                     Where-Object { $_.InstanceName -ne '_total' }
            $result = @()
            foreach ($core in $cores) {
                $result += [PSCustomObject]@{
                    Core = $core.InstanceName
                    Usage = $core.CookedValue
                }
            }
            ConvertTo-Json @($result)
        "#;

        let output = self.ps.execute(script).await?;

        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let cores: Vec<CoreSample> = serde_json::from_str(&output)
            .unwrap_or_default();

        Ok(cores
            .into_iter()
            .enumerate()
            .map(|(id, sample)| CoreUsage {
                core_id: id,
                usage: sample.Usage.min(100.0),
            })
            .collect())
    }

    async fn get_frequency_info(&self, cpu_info: &CpuInfo) -> Result<FrequencyInfo> {
        let current_mhz = cpu_info.current_clock_speed as f32;
        let max_mhz = cpu_info.max_clock_speed as f32;

        Ok(FrequencyInfo {
            base_clock: max_mhz / 1000.0,
            avg_frequency: current_mhz / 1000.0,
            max_frequency: max_mhz / 1000.0 * 1.2, // Estimated boost
            boost_active: current_mhz > max_mhz * 0.95,
        })
    }

    async fn get_top_processes(&self) -> Result<Vec<ProcessInfo>> {
        let script = r#"
            Get-Process |
            Where-Object { $_.CPU -gt 0 } |
            Sort-Object CPU -Descending |
            Select-Object -First 5 Id, ProcessName, CPU, Threads, @{Name='Memory';Expression={$_.WorkingSet64}} |
            ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;

        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let processes: Vec<ProcessSample> = serde_json::from_str(&output)
            .unwrap_or_default();

        Ok(processes
            .into_iter()
            .map(|p| ProcessInfo {
                pid: p.Id,
                name: p.ProcessName,
                cpu_usage: p.CPU.unwrap_or(0.0) as f32 / 10.0, // Normalize
                threads: p.Threads.unwrap_or(1) as usize,
                memory: p.Memory.unwrap_or(0),
            })
            .collect())
    }

    async fn get_temperature(&self) -> Result<f32> {
        // Try to get temperature from WMI thermal zone
        let script = r#"
            try {
                $temp = Get-CimInstance -Namespace "root/wmi" -ClassName MSAcpi_ThermalZoneTemperature -ErrorAction SilentlyContinue |
                        Select-Object -First 1 -ExpandProperty CurrentTemperature
                if ($temp) {
                    # Convert from tenths of Kelvin to Celsius
                    $celsius = ($temp / 10) - 273.15
                    [math]::Round($celsius, 1)
                } else {
                    # Fallback: estimate based on typical idle temps
                    45.0
                }
            } catch {
                45.0
            }
        "#;

        let output = self.ps.execute(script).await?;
        let temp: f32 = output.trim().parse()
            .unwrap_or(45.0);

        Ok(temp)
    }

    fn get_core_counts(&self, cpu_info: &CpuInfo) -> Result<(usize, usize)> {
        Ok((
            cpu_info.number_of_cores as usize,
            cpu_info.number_of_logical_processors as usize,
        ))
    }
}

// PowerShell data structures
#[derive(Debug, Deserialize)]
struct Win32Processor {
    Name: String,
    MaxClockSpeed: u32,
    CurrentClockSpeed: u32,
    NumberOfCores: u32,
    NumberOfLogicalProcessors: u32,
    TDP: Option<f32>,
}

#[derive(Debug)]
struct CpuInfo {
    name: String,
    max_clock_speed: u32,
    current_clock_speed: u32,
    number_of_cores: u32,
    number_of_logical_processors: u32,
    tdp: f32,
}

#[derive(Debug, Deserialize)]
struct CoreSample {
    Usage: f32,
}

#[derive(Debug, Deserialize)]
struct ProcessSample {
    Id: u32,
    ProcessName: String,
    CPU: Option<f64>,
    Threads: Option<u32>,
    Memory: Option<u64>,
}
