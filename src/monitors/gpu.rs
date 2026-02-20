use crate::integrations::PowerShellExecutor;
use crate::utils::parse_json_array;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuData {
    pub name: String,
    pub gpu_index: u32,
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
        if let Ok(nvidia_data) = self.get_nvidia_smi_linux().await {
            return Ok(nvidia_data);
        }

        anyhow::bail!("No supported GPU detected (nvidia-smi not found or failed)")
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
                if ($v -eq '' -or $v -eq 'N/A' -or $v -eq '[N/A]' -or $v -eq '[Not Supported]' -or $v -eq 'Not Supported') { return [float]$default }
                $out = 0.0
                if ([double]::TryParse($v, [ref]$out)) { return [float]$out }
                return [float]$default
            }

            function Parse-UInt64($value, $default) {
                if ($null -eq $value) { return [uint64]$default }
                $v = $value.ToString().Trim()
                if ($v -eq '' -or $v -eq 'N/A' -or $v -eq '[N/A]' -or $v -eq '[Not Supported]' -or $v -eq 'Not Supported') { return [uint64]$default }
                $out = 0.0
                if ([double]::TryParse($v, [ref]$out)) { return [uint64]$out }
                return [uint64]$default
            }

            # Get standard nvidia-smi output for power draw fallback
            $standardOutput = & $nvidiaPath
            $cudaVersion = "N/A"
            $fallbackPowerDraw = 0.0
            $fallbackPowerLimit = 0.0
            $gpuIndex = 0

            if ($standardOutput) {
                # Extract CUDA version
                $line = $standardOutput | Where-Object { $_ -match 'CUDA Version' } | Select-Object -First 1
                if ($line -match 'CUDA Version:\s*([0-9\.]+)') {
                    $cudaVersion = $Matches[1]
                }

                # Extract power draw from the table (e.g., "32W / 137W")
                $powerLine = $standardOutput | Where-Object { $_ -match 'Pwr:Usage/Cap' } | Select-Object -First 1
                if ($powerLine -match '(\d+)W\s*/\s*(\d+)W') {
                    $fallbackPowerDraw = [float]$Matches[1]
                    $fallbackPowerLimit = [float]$Matches[2]
                }

                # Extract GPU index from Bus-Id line
                $busLine = $standardOutput | Where-Object { $_ -match 'Bus-Id' } | Select-Object -First 1
                if ($busLine -match '\|\s*(\d+)\s+') {
                    $gpuIndex = [int]$Matches[1]
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

                $powerDraw = Parse-Float $parts[7] 0.0
                $powerLimit = Parse-Float $parts[8] 0.0

                # Use fallback power values if query returned 0
                if ($powerDraw -eq 0.0 -and $fallbackPowerDraw -gt 0.0) {
                    $powerDraw = $fallbackPowerDraw
                }
                if ($powerLimit -eq 0.0 -and $fallbackPowerLimit -gt 0.0) {
                    $powerLimit = $fallbackPowerLimit
                }

                # Parse GPU index from bus_id (format: 00000000:01:00.0)
                $busId = $parts[1]
                $busIdIndex = 0
                if ($busId -match ':(\d+):') {
                    $busIdIndex = [int]$Matches[1]
                }

                [PSCustomObject]@{
                    Name = $parts[0]
                    BusId = $parts[1]
                    GpuIndex = $busIdIndex
                    Temperature = Parse-Float $parts[2] 0.0
                    UtilizationGpu = Parse-Float $parts[3] 0.0
                    UtilizationMemory = Parse-Float $parts[4] 0.0
                    MemoryUsed = (Parse-UInt64 $parts[5] 0) * 1MB
                    MemoryTotal = (Parse-UInt64 $parts[6] 0) * 1MB
                    PowerDraw = $powerDraw
                    PowerLimit = $powerLimit
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
        let info: NvidiaSmiData =
            serde_json::from_str(trimmed).context("Failed to parse nvidia-smi data")?;

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
            gpu_index: info.GpuIndex,
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
        let info: GpuInfo = serde_json::from_str(trimmed).context("Failed to parse GPU info")?;

        let processes = self.get_gpu_processes().await.unwrap_or_default();

        let utilization = info.Utilization.unwrap_or(0.0).clamp(0.0, 100.0);
        let memory_total = info.MemoryTotal.unwrap_or(0);
        let mut memory_used = info.MemoryUsed.unwrap_or(0);
        if memory_total > 0 {
            memory_used = memory_used.min(memory_total);
        }

        Ok(GpuData {
            name: info.Name,
            gpu_index: 0,
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
        let processes: Vec<GpuProcessSample> =
            parse_json_array(&output).context("Failed to parse GPU process list")?;
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
        let processes: Vec<GpuProcessSample> =
            parse_json_array(&output).context("Failed to parse GPU process list")?;
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
        let output = self.run_nvidia_smi([
            "--query-gpu=name,pci.bus_id,temperature.gpu,utilization.gpu,utilization.memory,memory.used,memory.total,power.draw,power.limit,power.default_limit,fan.speed,clocks.current.graphics,clocks.current.memory,driver_version",
            "--format=csv,noheader,nounits",
        ])?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split(',').map(|s| s.trim()).collect();

        if parts.len() < 14 {
            anyhow::bail!("Invalid nvidia-smi output: expected 14 fields, got {}", parts.len());
        }

        let name = parts[0].to_string();
        let bus_id = parts[1].to_string();
        let temperature = parts[2].parse::<f32>().unwrap_or(0.0);
        let utilization_gpu = parts[3].parse::<f32>().unwrap_or(0.0);
        let _utilization_memory = parts[4].parse::<f32>().unwrap_or(0.0);
        let memory_used = parts[5].parse::<u64>().unwrap_or(0) * 1024 * 1024;
        let memory_total = parts[6].parse::<u64>().unwrap_or(0) * 1024 * 1024;

        // Parse power values properly - handle [N/A], [Not Supported], etc.
        let power_draw = Self::parse_nvidia_float(parts[7]);
        let power_limit = Self::parse_nvidia_float(parts[8]);
        let power_default_limit = Self::parse_nvidia_float(parts[9]);

        // Use power_limit if available, then default_limit, then try standard output
        let effective_power_limit = if power_limit > 0.0 {
            power_limit
        } else if power_default_limit > 0.0 {
            power_default_limit
        } else {
            self.get_nvidia_power_limit_from_standard_output().unwrap_or(0.0)
        };

        let fan_speed = Self::parse_nvidia_float_or(parts[10], -1.0);
        let clock_graphics = parts[11].parse::<u32>().unwrap_or(0);
        let clock_memory = parts[12].parse::<u32>().unwrap_or(0);
        let driver_version = parts[13].to_string();

        // Parse CUDA version from standard nvidia-smi output
        let cuda_version = self.get_nvidia_cuda_version().unwrap_or_else(|| "N/A".to_string());

        // Extract GPU index from bus_id
        let gpu_index = bus_id
            .split(':')
            .nth(1)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);

        // Get all GPU processes (compute + graphics)
        let processes = self.get_gpu_processes_linux().await.unwrap_or_default();

        Ok(GpuData {
            name,
            gpu_index,
            utilization: utilization_gpu.clamp(0.0, 100.0),
            memory_used,
            memory_total,
            temperature,
            power_usage: power_draw,
            power_limit: effective_power_limit,
            fan_speed,
            clock_speed: clock_graphics,
            memory_clock: clock_memory,
            driver_version,
            bus_id,
            cuda_version,
            processes,
        })
    }

    fn parse_nvidia_float(s: &str) -> f32 {
        let trimmed = s.trim();
        if trimmed.is_empty()
            || trimmed == "N/A"
            || trimmed == "[N/A]"
            || trimmed == "[Not Supported]"
            || trimmed == "Not Supported"
        {
            return 0.0;
        }
        trimmed.parse::<f32>().unwrap_or(0.0)
    }

    fn parse_nvidia_float_or(s: &str, default: f32) -> f32 {
        let trimmed = s.trim();
        if trimmed.is_empty()
            || trimmed == "N/A"
            || trimmed == "[N/A]"
            || trimmed == "[Not Supported]"
            || trimmed == "Not Supported"
        {
            return default;
        }
        trimmed.parse::<f32>().unwrap_or(default)
    }

    #[allow(dead_code)]
    fn get_nvidia_cuda_version(&self) -> Option<String> {
        let output = self.run_nvidia_smi(std::iter::empty::<&str>()).ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("CUDA Version") {
                if let Some(idx) = line.find("CUDA Version:") {
                    let version = line[idx + 14..].trim().split_whitespace().next()?;
                    return Some(version.trim_end_matches('|').trim().to_string());
                }
            }
        }
        None
    }

    #[allow(dead_code)]
    fn get_nvidia_power_limit_from_standard_output(&self) -> Option<f32> {
        let output = self.run_nvidia_smi(std::iter::empty::<&str>()).ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("W /") && line.contains('W') {
                let parts: Vec<&str> = line.split('/').collect();
                if parts.len() >= 2 {
                    let right = parts.last()?;
                    let watts_str = right.trim().trim_end_matches('W').trim().trim_end_matches('|').trim();
                    if let Ok(watts) = watts_str.parse::<f32>() {
                        if watts > 0.0 && watts < 10000.0 {
                            return Some(watts);
                        }
                    }
                }
            }
        }
        None
    }

    #[allow(dead_code)]
    async fn get_gpu_processes_linux(&self) -> Result<Vec<GpuProcessInfo>> {
        let mut processes = Vec::new();

        // Get compute processes
        if let Ok(output) = self.run_nvidia_smi([
            "--query-compute-apps=pid,process_name,used_memory",
            "--format=csv,noheader,nounits",
        ]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                if parts.len() >= 3 {
                    let pid = parts[0].parse::<u32>().unwrap_or(0);
                    if pid == 0 { continue; }
                    let name = parts[1].to_string();
                    let vram = parts[2].parse::<u64>().unwrap_or(0) * 1024 * 1024;

                    processes.push(GpuProcessInfo {
                        pid,
                        name,
                        gpu_usage: -1.0,
                        vram,
                        process_type: "Compute".to_string(),
                    });
                }
            }
        }

        // Get graphics processes
        if let Ok(output) = self.run_nvidia_smi([
            "--query-graphics-apps=pid,process_name,used_memory",
            "--format=csv,noheader,nounits",
        ]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                if parts.len() >= 3 {
                    let pid = parts[0].parse::<u32>().unwrap_or(0);
                    if pid == 0 { continue; }

                    // If already in compute list, mark as both
                    if let Some(existing) = processes.iter_mut().find(|p| p.pid == pid) {
                        existing.process_type = "Compute+Graphics".to_string();
                        let new_vram = parts[2].parse::<u64>().unwrap_or(0) * 1024 * 1024;
                        if new_vram > existing.vram {
                            existing.vram = new_vram;
                        }
                        continue;
                    }

                    let name = parts[1].to_string();
                    let vram = parts[2].parse::<u64>().unwrap_or(0) * 1024 * 1024;

                    processes.push(GpuProcessInfo {
                        pid,
                        name,
                        gpu_usage: -1.0,
                        vram,
                        process_type: "Graphics".to_string(),
                    });
                }
            }
        }

        // Try to get per-process GPU utilization
        if let Ok(output) = self.run_nvidia_smi(["-q", "-d", "PIDS"]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut current_pid: Option<u32> = None;
            for line in stdout.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("Process ID") {
                    if let Some(val) = trimmed.split(':').nth(1) {
                        current_pid = val.trim().parse::<u32>().ok();
                    }
                } else if trimmed.starts_with("Sm Utilization") {
                    if let Some(pid) = current_pid {
                        if let Some(val) = trimmed.split(':').nth(1) {
                            let usage_str = val.trim().trim_end_matches('%').trim();
                            if let Ok(usage) = usage_str.parse::<f32>() {
                                if let Some(proc) = processes.iter_mut().find(|p| p.pid == pid) {
                                    proc.gpu_usage = usage;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Sort by VRAM usage descending
        processes.sort_by(|a, b| b.vram.cmp(&a.vram));

        Ok(processes)
    }

    #[allow(dead_code)]
    fn run_nvidia_smi<I, S>(&self, args: I) -> Result<std::process::Output>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = std::process::Command::new("nvidia-smi").args(args).output();
        match output {
            Ok(out) if out.status.success() => Ok(out),
            Ok(out) => anyhow::bail!("nvidia-smi exited with status {}", out.status),
            Err(e) => anyhow::bail!("nvidia-smi command not found: {}", e),
        }
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct NvidiaSmiData {
    Name: String,
    BusId: String,
    GpuIndex: u32,
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

//"""
//nvidia-smi output example:

//Fri Feb 20 03:55:07 2026
//+-----------------------------------------------------------------------------------------+
//| NVIDIA-SMI 590.48.01              Driver Version: 590.48.01      CUDA Version: 13.1     |
//+-----------------------------------------+------------------------+----------------------+
//| GPU  Name                 Persistence-M | Bus-Id          Disp.A | Volatile Uncorr. ECC |
//| Fan  Temp   Perf          Pwr:Usage/Cap |           Memory-Usage | GPU-Util  Compute M. |
//|                                         |                        |               MIG M. |
//|=========================================+========================+======================|
//|   0  NVIDIA GeForce RTX 3080 ...    Off |   00000000:01:00.0 Off |                  N/A |
//| N/A   41C    P8             12W /  115W |      12MiB /   8192MiB |      0%      Default |
//|                                         |                        |                  N/A |
//+-----------------------------------------+------------------------+----------------------+
//
//+-----------------------------------------------------------------------------------------+
//| Processes:                                                                              |
//|  GPU   GI   CI              PID   Type   Process name                        GPU Memory |
//|        ID   ID                                                               Usage      |
//|=========================================================================================|
//|    0   N/A  N/A            1636      G   /usr/bin/X                                4MiB |
//+-----------------------------------------------------------------------------------------+
//"""