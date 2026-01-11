use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;
use crate::utils::parse_json_array;

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
    pub bus_id: String,
    pub cuda_version: String,
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
}

impl GpuMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    pub async fn collect_data(&self) -> Result<GpuData> {
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
    async fn collect_data_linux(&self) -> Result<GpuData> {
        // Try nvidia-smi directly (for NVIDIA GPUs)
        if let Ok(nvidia_data) = self.get_nvidia_smi_linux().await {
            return Ok(nvidia_data);
        }

        // Fallback to stub data if no GPU found
        Ok(self.get_stub_gpu_data())
    }

    async fn collect_data_windows(&self) -> Result<GpuData> {
        // Try nvidia-smi first (for NVIDIA GPUs)
        if let Ok(nvidia_data) = self.get_nvidia_smi_data().await {
            return Ok(nvidia_data);
        }

        // Fallback to WMI/perf counters
        self.get_wmi_gpu_data().await
    }

    async fn get_nvidia_smi_data(&self) -> Result<GpuData> {
        let script = r#"
            $nvidiaPath = $null
            $cmd = Get-Command nvidia-smi -ErrorAction SilentlyContinue
            if ($cmd) {
                $nvidiaPath = $cmd.Source
            } elseif (Test-Path 'C:\Windows\System32\nvidia-smi.exe') {
                $nvidiaPath = 'C:\Windows\System32\nvidia-smi.exe'
            } elseif (Test-Path 'C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe') {
                $nvidiaPath = 'C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe'
            }

            if (-not $nvidiaPath) {
                throw "nvidia-smi not found"
            }

            function Parse-Float($value, $default) {
                if ($null -eq $value) { return [float]$default }
                $v = $value.ToString().Trim()
                if ($v -eq '' -or $v -eq 'N/A' -or $v -eq '[N/A]') { return [float]$default }
                $out = 0.0
                if ([double]::TryParse($v, [ref]$out)) { return [float]$out }
                return [float]$default
            }

            function Parse-UInt64($value, $default) {
                if ($null -eq $value) { return [uint64]$default }
                $v = $value.ToString().Trim()
                if ($v -eq '' -or $v -eq 'N/A' -or $v -eq '[N/A]') { return [uint64]$default }
                $out = 0.0
                if ([double]::TryParse($v, [ref]$out)) { return [uint64]$out }
                return [uint64]$default
            }

            $cudaVersion = "N/A"
            $header = & $nvidiaPath
            if ($header) {
                $line = $header | Where-Object { $_ -match 'CUDA Version' } | Select-Object -First 1
                if ($line -match 'CUDA Version:\s*([0-9\.]+)') {
                    $cudaVersion = $Matches[1]
                }
            }

            $raw = & $nvidiaPath --query-gpu=name,pci.bus_id,temperature.gpu,utilization.gpu,utilization.memory,memory.used,memory.total,power.draw,power.limit,fan.speed,clocks.current.graphics,clocks.current.memory,driver_version --format=csv,noheader,nounits
            $lines = $raw -split "`n" | Where-Object { $_ -match '\S' }
            if (-not $lines) {
                throw "nvidia-smi returned empty output"
            }

            $rows = foreach ($line in $lines) {
                $parts = $line.Split(',') | ForEach-Object { $_.Trim() }
                if ($parts.Count -lt 13) { continue }
                [PSCustomObject]@{
                    Name = $parts[0]
                    BusId = $parts[1]
                    Temperature = Parse-Float $parts[2] 0.0
                    UtilizationGpu = Parse-Float $parts[3] 0.0
                    UtilizationMemory = Parse-Float $parts[4] 0.0
                    MemoryUsed = (Parse-UInt64 $parts[5] 0) * 1MB
                    MemoryTotal = (Parse-UInt64 $parts[6] 0) * 1MB
                    PowerDraw = Parse-Float $parts[7] 0.0
                    PowerLimit = Parse-Float $parts[8] 0.0
                    FanSpeed = Parse-Float $parts[9] -1.0
                    ClockGraphics = [uint32](Parse-UInt64 $parts[10] 0)
                    ClockMemory = [uint32](Parse-UInt64 $parts[11] 0)
                    DriverVersion = $parts[12]
                    CudaVersion = $cudaVersion
                }
            }

            $best = $rows | Sort-Object -Property MemoryTotal -Descending | Select-Object -First 1
            if (-not $best) {
                throw "nvidia-smi parsing failed"
            }

            $best | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        let info: NvidiaSmiData = serde_json::from_str(trimmed)
            .context("Failed to parse nvidia-smi data")?;

        let processes = self.get_gpu_processes().await.unwrap_or_default();

        let memory_total = info.MemoryTotal;
        let memory_used = info.MemoryUsed;
        let memory_used = if memory_total > 0 {
            memory_used.min(memory_total)
        } else {
            memory_used
        };

        Ok(GpuData {
            name: info.Name,
            utilization: info.UtilizationGpu.clamp(0.0, 100.0),
            memory_used,
            memory_total,
            temperature: info.Temperature,
            power_usage: info.PowerDraw,
            power_limit: info.PowerLimit,
            fan_speed: info.FanSpeed,
            clock_speed: info.ClockGraphics,
            memory_clock: info.ClockMemory,
            driver_version: info.DriverVersion,
            bus_id: info.BusId,
            cuda_version: info.CudaVersion,
            processes,
        })
    }

    async fn get_wmi_gpu_data(&self) -> Result<GpuData> {
        let script = r#"
            $gpus = Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue
            $gpu = $gpus | Sort-Object AdapterRAM -Descending | Select-Object -First 1
            if (-not $gpu) {
                throw "No GPU detected"
            }

            $engine = Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -ErrorAction SilentlyContinue
            $util = if ($engine) {
                ($engine | Measure-Object -Property UtilizationPercentage -Maximum).Maximum
            } else {
                0
            }

            $procMem = Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUProcessMemory -ErrorAction SilentlyContinue
            $memUsed = if ($procMem) {
                ($procMem | Measure-Object -Property DedicatedUsage -Sum).Sum
            } else {
                0
            }

            $adapterMem = Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUAdapterMemory -ErrorAction SilentlyContinue |
                Sort-Object TotalDedicatedMemory -Descending | Select-Object -First 1
            $memTotal = if ($adapterMem -and $adapterMem.TotalDedicatedMemory) {
                [uint64]$adapterMem.TotalDedicatedMemory
            } else {
                [uint64]$gpu.AdapterRAM
            }

            [PSCustomObject]@{
                Name = $gpu.Name
                DriverVersion = $gpu.DriverVersion
                MemoryTotal = $memTotal
                MemoryUsed = [uint64]$memUsed
                Utilization = [float]$util
            } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        let info: GpuInfo = serde_json::from_str(trimmed)
            .context("Failed to parse GPU info")?;

        let processes = self.get_gpu_processes().await.unwrap_or_default();

        let utilization = info.Utilization.unwrap_or(0.0).clamp(0.0, 100.0);
        let memory_total = info.MemoryTotal.unwrap_or(0);
        let mut memory_used = info.MemoryUsed.unwrap_or(0);
        if memory_total > 0 {
            memory_used = memory_used.min(memory_total);
        }

        Ok(GpuData {
            name: info.Name,
            utilization,
            memory_used,
            memory_total,
            temperature: 0.0,
            power_usage: 0.0,
            power_limit: 0.0,
            fan_speed: -1.0,
            clock_speed: 0,
            memory_clock: 0,
            driver_version: info.DriverVersion,
            bus_id: "N/A".to_string(),
            cuda_version: "N/A".to_string(),
            processes,
        })
    }

    async fn get_gpu_processes(&self) -> Result<Vec<GpuProcessInfo>> {
        if let Ok(processes) = self.get_gpu_processes_wmi().await {
            if !processes.is_empty() {
                return Ok(processes);
            }
        }

        let script = r#"
            $nvidiaPath = $null
            $cmd = Get-Command nvidia-smi -ErrorAction SilentlyContinue
            if ($cmd) {
                $nvidiaPath = $cmd.Source
            } elseif (Test-Path 'C:\Windows\System32\nvidia-smi.exe') {
                $nvidiaPath = 'C:\Windows\System32\nvidia-smi.exe'
            } elseif (Test-Path 'C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe') {
                $nvidiaPath = 'C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe'
            }

            if ($nvidiaPath) {
                & $nvidiaPath --query-compute-apps=pid,process_name,used_memory --format=csv,noheader,nounits | ForEach-Object {
                    $parts = $_.Split(',') | ForEach-Object { $_.Trim() }
                    if ($parts.Count -lt 3) { return }
                    [PSCustomObject]@{
                        Pid = [uint32]$parts[0]
                        Name = $parts[1]
                        Vram = [uint64]($parts[2]) * 1MB
                        GpuUsage = -1.0
                        Type = "Compute"
                    }
                } | ConvertTo-Json
            } else {
                "[]"
            }
        "#;

        let output = self.ps.execute(script).await?;
        let processes: Vec<GpuProcessSample> = parse_json_array(&output)
            .context("Failed to parse GPU process list")?;
        if processes.is_empty() {
            return Ok(Vec::new());
        }

        Ok(processes
            .into_iter()
            .map(|p| GpuProcessInfo {
                pid: p.Pid,
                name: p.Name,
                gpu_usage: if p.GpuUsage < 0.0 { -1.0 } else { p.GpuUsage },
                vram: p.Vram,
                process_type: if p.Type.trim().is_empty() {
                    "Compute".to_string()
                } else {
                    p.Type
                },
            })
            .collect())
    }

    async fn get_gpu_processes_wmi(&self) -> Result<Vec<GpuProcessInfo>> {
        let script = r#"
            $items = Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUProcessMemory -ErrorAction SilentlyContinue
            if (-not $items) {
                "[]"
                return
            }

            $byPid = @{}
            foreach ($item in $items) {
                if ($item.Name -match '^pid_(\d+)_') {
                    $pid = [int]$matches[1]
                    if (-not $byPid.ContainsKey($pid)) {
                        $byPid[$pid] = [uint64]0
                    }
                    $byPid[$pid] += [uint64]$item.DedicatedUsage
                }
            }

            $engine = Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -ErrorAction SilentlyContinue
            $gpuByPid = @{}
            $typeByPid = @{}
            $typeUtilByPid = @{}
            if ($engine) {
                foreach ($item in $engine) {
                    if ($item.Name -match '^pid_(\d+)_') {
                        $pid = [int]$matches[1]
                        $util = [float]$item.UtilizationPercentage
                        if (-not $gpuByPid.ContainsKey($pid)) { $gpuByPid[$pid] = 0.0 }
                        $gpuByPid[$pid] += $util

                        $etype = "Unknown"
                        if ($item.Name -match 'engtype_3D' -or $item.Name -match 'engtype_Graphics') {
                            $etype = "Graphics"
                        } elseif ($item.Name -match 'engtype_Compute') {
                            $etype = "Compute"
                        } elseif ($item.Name -match 'engtype_Copy') {
                            $etype = "Copy"
                        }
                        if (-not $typeUtilByPid.ContainsKey($pid) -or $util -gt $typeUtilByPid[$pid]) {
                            $typeUtilByPid[$pid] = $util
                            $typeByPid[$pid] = $etype
                        }
                    }
                }
            }

            if ($byPid.Count -eq 0 -and $gpuByPid.Count -eq 0) {
                "[]"
                return
            }

            $allPids = @($byPid.Keys + $gpuByPid.Keys) | Sort-Object -Unique
            $procMap = @{}
            try {
                Get-Process -Id $allPids -ErrorAction SilentlyContinue | ForEach-Object {
                    $procMap[$_.Id] = $_.ProcessName
                }
            } catch {}

            $result = foreach ($pid in $allPids) {
                $vram = if ($byPid.ContainsKey($pid)) { [uint64]$byPid[$pid] } else { [uint64]0 }
                $gpu = if ($gpuByPid.ContainsKey($pid)) { [float]$gpuByPid[$pid] } else { -1.0 }
                $ptype = if ($typeByPid.ContainsKey($pid)) { $typeByPid[$pid] } else { "Unknown" }
                [PSCustomObject]@{
                    Pid = [uint32]$pid
                    Name = if ($procMap.ContainsKey($pid)) { $procMap[$pid] } else { "PID $pid" }
                    Vram = $vram
                    GpuUsage = $gpu
                    Type = $ptype
                }
            } | Sort-Object -Property Vram -Descending | Select-Object -First 50

            $result | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        let processes: Vec<GpuProcessSample> = parse_json_array(&output)
            .context("Failed to parse GPU process list")?;
        if processes.is_empty() {
            return Ok(Vec::new());
        }

        Ok(processes
            .into_iter()
            .map(|p| GpuProcessInfo {
                pid: p.Pid,
                name: p.Name,
                gpu_usage: if p.GpuUsage < 0.0 { -1.0 } else { p.GpuUsage },
                vram: p.Vram,
                process_type: if p.Type.trim().is_empty() {
                    "Unknown".to_string()
                } else {
                    p.Type
                },
            })
            .collect())
    }

    // Linux-specific nvidia-smi implementation
    #[allow(dead_code)]
    async fn get_nvidia_smi_linux(&self) -> Result<GpuData> {
        use std::process::Command;

        let output = Command::new("nvidia-smi")
            .args(&[
                "--query-gpu=name,temperature.gpu,utilization.gpu,utilization.memory,memory.used,memory.total,power.draw,power.limit,fan.speed,clocks.current.graphics,clocks.current.memory,driver_version",
                "--format=csv,noheader,nounits"
            ])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split(',').map(|s| s.trim()).collect();

        if parts.len() < 12 {
            anyhow::bail!("Invalid nvidia-smi output");
        }

        let name = parts[0].to_string();
        let temperature = parts[1].parse::<f32>().unwrap_or(0.0);
        let utilization_gpu = parts[2].parse::<f32>().unwrap_or(0.0);
        let _utilization_memory = parts[3].parse::<f32>().unwrap_or(0.0);
        let memory_used = parts[4].parse::<u64>().unwrap_or(0) * 1024 * 1024; // MB to bytes
        let memory_total = parts[5].parse::<u64>().unwrap_or(0) * 1024 * 1024; // MB to bytes
        let power_draw = parts[6].parse::<f32>().unwrap_or(0.0);
        let power_limit = parts[7].parse::<f32>().unwrap_or(300.0);
        let fan_speed = if parts[8] == "[N/A]" || parts[8] == "N/A" {
            -1.0
        } else {
            parts[8].parse::<f32>().unwrap_or(-1.0)
        };
        let clock_graphics = parts[9].parse::<u32>().unwrap_or(0);
        let clock_memory = parts[10].parse::<u32>().unwrap_or(0);
        let driver_version = parts[11].to_string();

        // Get GPU processes
        let processes = self.get_gpu_processes_linux().await.unwrap_or_default();

        Ok(GpuData {
            name,
            utilization: utilization_gpu,
            memory_used,
            memory_total,
            temperature,
            power_usage: power_draw,
            power_limit,
            fan_speed,
            clock_speed: clock_graphics,
            memory_clock: clock_memory,
            driver_version,
            bus_id: "N/A".to_string(),
            cuda_version: "N/A".to_string(),
            processes,
        })
    }

    #[allow(dead_code)]
    async fn get_gpu_processes_linux(&self) -> Result<Vec<GpuProcessInfo>> {
        use std::process::Command;

        let output = Command::new("nvidia-smi")
            .args(&[
                "--query-compute-apps=pid,process_name,used_memory",
                "--format=csv,noheader,nounits"
            ])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut processes = Vec::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() >= 3 {
                let pid = parts[0].parse::<u32>().unwrap_or(0);
                let name = parts[1].to_string();
                let vram = parts[2].parse::<u64>().unwrap_or(0) * 1024 * 1024; // MB to bytes

                processes.push(GpuProcessInfo {
                    pid,
                    name,
                    gpu_usage: 0.0,
                    vram,
                    process_type: "Compute".to_string(),
                });
            }
        }

        Ok(processes)
    }

    #[allow(dead_code)]
    fn get_stub_gpu_data(&self) -> GpuData {
        GpuData {
            name: "No GPU detected".to_string(),
            utilization: 0.0,
            memory_used: 0,
            memory_total: 0,
            temperature: 0.0,
            power_usage: 0.0,
            power_limit: 0.0,
            fan_speed: -1.0,
            clock_speed: 0,
            memory_clock: 0,
            driver_version: "N/A".to_string(),
            bus_id: "N/A".to_string(),
            cuda_version: "N/A".to_string(),
            processes: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct NvidiaSmiData {
    Name: String,
    BusId: String,
    Temperature: f32,
    UtilizationGpu: f32,
    #[allow(dead_code)]
    UtilizationMemory: f32,
    MemoryUsed: u64,
    MemoryTotal: u64,
    PowerDraw: f32,
    PowerLimit: f32,
    FanSpeed: f32,
    ClockGraphics: u32,
    ClockMemory: u32,
    DriverVersion: String,
    CudaVersion: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct GpuProcessSample {
    Pid: u32,
    Name: String,
    Vram: u64,
    #[serde(default)]
    GpuUsage: f32,
    #[serde(default)]
    Type: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct GpuInfo {
    Name: String,
    DriverVersion: String,
    MemoryTotal: Option<u64>,
    MemoryUsed: Option<u64>,
    Utilization: Option<f32>,
}
