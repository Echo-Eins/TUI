use crate::integrations::PowerShellExecutor;
use anyhow::{Context, Result};
use log::warn;
#[cfg(feature = "nvidia")]
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
#[cfg(feature = "nvidia")]
use nvml_wrapper::enums::device::UsedGpuMemory;
#[cfg(feature = "nvidia")]
use nvml_wrapper::struct_wrappers::device::ProcessInfo;
#[cfg(feature = "nvidia")]
use nvml_wrapper::Nvml;
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, System};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuData {
    pub name: String,
    pub utilization: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub temperature: f32,
    pub power_usage: f32,
    pub power_limit: f32,
    pub fan_speed: f32,
    pub clock_speed: u32,
    pub memory_clock: u32,
    pub driver_version: String,
    pub processes: Vec<GpuProcessInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuProcessInfo {
    pub pid: u32,
    pub name: String,
    pub gpu_usage: f32,
    pub vram: u64,
    pub process_type: String,
}

pub struct GpuMonitor {
    ps: PowerShellExecutor,
    prefer_nvml: bool,
}

impl GpuMonitor {
    pub fn new(ps: PowerShellExecutor, prefer_nvml: bool) -> Result<Self> {
        Ok(Self { ps, prefer_nvml })
    }

    pub async fn collect_data(&self) -> Result<GpuData> {
        #[cfg(feature = "nvidia")]
        if self.prefer_nvml {
            match self.get_nvml_data() {
                Ok(data) => return Ok(data),
                Err(e) => warn!("NVML telemetry unavailable, falling back: {}", e),
            }
        }

        if let Ok(nvidia_data) = self.get_nvidia_smi_data().await {
            return Ok(nvidia_data);
        }

        let gpu_info = self.get_windows_gpu_counters().await?;
        Ok(gpu_info)
    }

    async fn get_nvidia_smi_data(&self) -> Result<GpuData> {
        let script = r#"
            if (Get-Command nvidia-smi -ErrorAction SilentlyContinue) {
                $output = nvidia-smi --query-gpu=name,temperature.gpu,utilization.gpu,utilization.memory,memory.used,memory.total,power.draw,power.limit,fan.speed,clocks.current.graphics,clocks.current.memory,driver_version --format=csv,noheader,nounits
                $parts = $output.Split(',').Trim()

                [PSCustomObject]@{
                    Name = $parts[0]
                    Temperature = [float]$parts[1]
                    UtilizationGpu = [float]$parts[2]
                    UtilizationMemory = [float]$parts[3]
                    MemoryUsed = [uint64]($parts[4]) * 1MB
                    MemoryTotal = [uint64]($parts[5]) * 1MB
                    PowerDraw = [float]$parts[6]
                    PowerLimit = [float]$parts[7]
                    FanSpeed = if ($parts[8] -ne '[N/A]') { [float]$parts[8] } else { 0.0 }
                    ClockGraphics = [uint32]$parts[9]
                    ClockMemory = [uint32]$parts[10]
                    DriverVersion = $parts[11]
                } | ConvertTo-Json
            } else {
                throw "nvidia-smi not found"
            }
        "#;

        let output = self.ps.execute_uncached(script).await?;
        let info: NvidiaSmiData =
            serde_json::from_str(&output).context("Failed to parse nvidia-smi data")?;

        let processes = self.get_gpu_processes().await.unwrap_or_default();

        Ok(GpuData {
            name: info.Name,
            utilization: info.UtilizationGpu,
            memory_used: info.MemoryUsed,
            memory_total: info.MemoryTotal,
            temperature: info.Temperature,
            power_usage: info.PowerDraw,
            power_limit: info.PowerLimit,
            fan_speed: info.FanSpeed,
            clock_speed: info.ClockGraphics,
            memory_clock: info.ClockMemory,
            driver_version: info.DriverVersion,
            processes,
        })
    }

    async fn get_windows_gpu_counters(&self) -> Result<GpuData> {
        let script = r#"
            $gpu = Get-CimInstance Win32_VideoController | Select-Object -First 1

            $engineCounters = Get-Counter '\\GPU Engine(*)\\Utilization Percentage' -ErrorAction SilentlyContinue
            $utilization = if ($engineCounters.CounterSamples) {
                ($engineCounters.CounterSamples | Measure-Object -Property CookedValue -Sum).Sum
            } else {
                0
            }

            $memoryUsedCounter = Get-Counter '\\GPU Adapter Memory(*)\\Dedicated Usage' -ErrorAction SilentlyContinue
            $memoryLimitCounter = Get-Counter '\\GPU Adapter Memory(*)\\Dedicated Limit' -ErrorAction SilentlyContinue

            $memoryUsed = if ($memoryUsedCounter.CounterSamples) {
                ($memoryUsedCounter.CounterSamples | Measure-Object -Property CookedValue -Sum).Sum * 1MB
            } else {
                0
            }

            $memoryTotal = if ($memoryLimitCounter.CounterSamples) {
                ($memoryLimitCounter.CounterSamples | Measure-Object -Property CookedValue -Sum).Sum * 1MB
            } else {
                $gpu.AdapterRAM
            }

            $processSamples = Get-Counter '\\GPU Process Memory(*)\\Dedicated Usage' -ErrorAction SilentlyContinue
            $processes = @()

            if ($processSamples.CounterSamples) {
                foreach ($sample in $processSamples.CounterSamples) {
                    if ($sample.InstanceName -match 'pid_(\d+)_') {
                        $pid = [uint32]$matches[1]
                        $procName = (Get-Process -Id $pid -ErrorAction SilentlyContinue).ProcessName
                        $processes += [PSCustomObject]@{
                            Pid = $pid
                            Name = if ($procName) { $procName } else { "" }
                            Vram = [uint64]$sample.CookedValue * 1MB
                        }
                    }
                }
            }

            [PSCustomObject]@{
                Name = $gpu.Name
                DriverVersion = $gpu.DriverVersion
                Utilization = [float]$utilization
                MemoryUsed = [uint64]$memoryUsed
                MemoryTotal = [uint64]$memoryTotal
                Processes = $processes
            } | ConvertTo-Json -Depth 4
        "#;

        let output = self.ps.execute_uncached(script).await?;
        let info: WindowsGpuCounters = serde_json::from_str(&output)
            .context("Failed to parse Windows GPU performance data")?;

        let processes = info
            .Processes
            .unwrap_or_default()
            .into_iter()
            .map(|p| GpuProcessInfo {
                pid: p.Pid,
                name: p.Name.unwrap_or_default(),
                gpu_usage: 0.0,
                vram: p.Vram,
                process_type: "Unknown".to_string(),
            })
            .collect();

        Ok(GpuData {
            name: info.Name,
            utilization: info.Utilization,
            memory_used: info.MemoryUsed,
            memory_total: info.MemoryTotal,
            temperature: 0.0,
            power_usage: 0.0,
            power_limit: 0.0,
            fan_speed: 0.0,
            clock_speed: 0,
            memory_clock: 0,
            driver_version: info.DriverVersion,
            processes,
        })
    }

    #[cfg(feature = "nvidia")]
    fn get_nvml_data(&self) -> Result<GpuData> {
        let nvml = Nvml::init().context("Failed to initialize NVML")?;
        let device = nvml
            .device_by_index(0)
            .context("Failed to access primary GPU via NVML")?;

        let utilization = device
            .utilization_rates()
            .context("Failed to read GPU utilization from NVML")?;
        let memory = device
            .memory_info()
            .context("Failed to read GPU memory from NVML")?;

        let temperature = device.temperature(TemperatureSensor::Gpu).unwrap_or(0) as f32;
        let power_usage = device.power_usage().unwrap_or(0) as f32 / 1000.0;
        let power_limit = device.enforced_power_limit().unwrap_or(0) as f32 / 1000.0;
        let fan_speed = device.fan_speed(0).unwrap_or(0) as f32;
        let clock_speed = device.clock_info(Clock::Graphics).unwrap_or(0);
        let memory_clock = device.clock_info(Clock::Memory).unwrap_or(0);
        let driver_version = nvml.sys_driver_version().unwrap_or_else(|_| "".to_string());
        let name = device.name().unwrap_or_else(|_| "Unknown GPU".to_string());

        let mut system = System::new_all();
        system.refresh_processes();

        let mut processes = Vec::new();
        if let Ok(compute_processes) = device.running_compute_processes() {
            processes.extend(self.map_nvml_processes(compute_processes, "Compute", &mut system));
        }

        if let Ok(graphics_processes) = device.running_graphics_processes() {
            processes.extend(self.map_nvml_processes(graphics_processes, "Graphics", &mut system));
        }

        Ok(GpuData {
            name,
            utilization: utilization.gpu as f32,
            memory_used: memory.used,
            memory_total: memory.total,
            temperature,
            power_usage,
            power_limit,
            fan_speed,
            clock_speed,
            memory_clock,
            driver_version,
            processes,
        })
    }

    #[cfg(feature = "nvidia")]
    fn map_nvml_processes(
        &self,
        processes: Vec<ProcessInfo>,
        process_type: &str,
        system: &mut System,
    ) -> Vec<GpuProcessInfo> {
        processes
            .into_iter()
            .map(|p| {
                let pid = p.pid;
                let name = system
                    .process(Pid::from_u32(pid))
                    .map(|proc| proc.name().to_string())
                    .unwrap_or_default();

                let vram = match p.used_gpu_memory {
                    UsedGpuMemory::Used(bytes) => bytes,
                    UsedGpuMemory::Unavailable => 0,
                };

                GpuProcessInfo {
                    pid,
                    name,
                    gpu_usage: 0.0,
                    vram,
                    process_type: process_type.to_string(),
                }
            })
            .collect()
    }

    async fn get_gpu_processes(&self) -> Result<Vec<GpuProcessInfo>> {
        let script = r#"
            if (Get-Command nvidia-smi -ErrorAction SilentlyContinue) {
                nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv,noheader,nounits | ForEach-Object {
                    $parts = $_.Split(',').Trim()
                    [PSCustomObject]@{
                        Pid = [uint32]$parts[0]
                        Name = $parts[1]
                        Vram = [uint64]($parts[2]) * 1MB
                    }
                } | ConvertTo-Json
            } else {
                "[]"
            }
        "#;

        let output = self.ps.execute_uncached(script).await?;
        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let processes: Vec<GpuProcessSample> = serde_json::from_str(&output).unwrap_or_default();

        Ok(processes
            .into_iter()
            .map(|p| GpuProcessInfo {
                pid: p.Pid,
                name: p.Name,
                gpu_usage: 0.0, // Not available via nvidia-smi compute-apps
                vram: p.Vram,
                process_type: "Compute".to_string(),
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct NvidiaSmiData {
    Name: String,
    Temperature: f32,
    UtilizationGpu: f32,
    UtilizationMemory: f32,
    MemoryUsed: u64,
    MemoryTotal: u64,
    PowerDraw: f32,
    PowerLimit: f32,
    FanSpeed: f32,
    ClockGraphics: u32,
    ClockMemory: u32,
    DriverVersion: String,
}

#[derive(Debug, Deserialize)]
struct GpuProcessSample {
    Pid: u32,
    Name: String,
    Vram: u64,
}

#[derive(Debug, Deserialize)]
struct WindowsGpuCounters {
    Name: String,
    DriverVersion: String,
    Utilization: f32,
    MemoryUsed: u64,
    MemoryTotal: u64,
    Processes: Option<Vec<WindowsGpuProcess>>,
}

#[derive(Debug, Deserialize)]
struct WindowsGpuProcess {
    Pid: u32,
    Name: Option<String>,
    Vram: u64,
}
