use crate::monitors::traits::*;
use crate::monitors::types::*;
use anyhow::Result;

pub struct LinuxGpuMonitor {}

impl LinuxGpuMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {})
    }

    async fn get_nvidia_smi_linux(&self) -> Result<GpuData> {
        let output = self.run_nvidia_smi(vec![
            "--query-gpu=name,pci.bus_id,temperature.gpu,utilization.gpu,utilization.memory,memory.used,memory.total,power.draw,power.limit,power.default_limit,fan.speed,clocks.current.graphics,clocks.current.memory,driver_version",
            "--format=csv,noheader,nounits",
        ])?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split(',').map(|s| s.trim()).collect();

        if parts.len() < 14 {
            anyhow::bail!(
                "Invalid nvidia-smi output: expected 14 fields, got {}",
                parts.len()
            );
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
            self.get_nvidia_power_limit_from_standard_output()
                .unwrap_or(0.0)
        };

        let fan_speed = Self::parse_nvidia_float_or(parts[10], -1.0);
        let clock_graphics = parts[11].parse::<u32>().unwrap_or(0);
        let clock_memory = parts[12].parse::<u32>().unwrap_or(0);
        let driver_version = parts[13].to_string();

        // Parse CUDA version from standard nvidia-smi output
        let cuda_version = self
            .get_nvidia_cuda_version()
            .unwrap_or_else(|| "N/A".to_string());

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

    fn get_nvidia_cuda_version(&self) -> Option<String> {
        let output = self.run_nvidia_smi(Vec::<&str>::new()).ok()?;
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

    fn get_nvidia_power_limit_from_standard_output(&self) -> Option<f32> {
        let output = self.run_nvidia_smi(Vec::<&str>::new()).ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("W /") && line.contains('W') {
                let parts: Vec<&str> = line.split('/').collect();
                if parts.len() >= 2 {
                    let right = parts.last()?;
                    let watts_str = right
                        .trim()
                        .trim_end_matches('W')
                        .trim()
                        .trim_end_matches('|')
                        .trim();
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

    async fn get_gpu_processes_linux(&self) -> Result<Vec<GpuProcessInfo>> {
        let mut processes = Vec::new();

        // Primary method: nvidia-smi pmon (most reliable for per-process data)
        if let Ok(output) = self.run_nvidia_smi(vec!["pmon", "-s", "um", "-c", "1"]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let line = line.trim();
                // Skip header and comment lines (start with #)
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let parts: Vec<&str> = line.split_whitespace().collect();
                // Format: gpu pid type sm mem enc dec fb command
                // At minimum we need: gpu(0) pid(1) type(2) sm(3) mem(4) ... fb(7) command(8)
                if parts.len() >= 9 {
                    let pid = parts[1].parse::<u32>().unwrap_or(0);
                    if pid == 0 {
                        continue;
                    }

                    let proc_type = parts[2]; // C, G, or C+G
                    let sm_usage = parts[3].parse::<f32>().unwrap_or(-1.0);
                    let fb_mem_mb = parts[7].parse::<u64>().unwrap_or(0);
                    let command = parts[8..].join(" ");

                    // Get short process name from command path
                    let name = command.rsplit('/').next().unwrap_or(&command).to_string();

                    let process_type = match proc_type {
                        "C" => "Compute".to_string(),
                        "G" => "Graphics".to_string(),
                        "C+G" => "Compute+Graphics".to_string(),
                        _ => proc_type.to_string(),
                    };

                    // Merge duplicate PIDs (pmon may show multiple lines per GPU)
                    if let Some(existing) = processes
                        .iter_mut()
                        .find(|p: &&mut GpuProcessInfo| p.pid == pid)
                    {
                        if fb_mem_mb * 1024 * 1024 > existing.vram {
                            existing.vram = fb_mem_mb * 1024 * 1024;
                        }
                        if sm_usage > existing.gpu_usage {
                            existing.gpu_usage = sm_usage;
                        }
                        continue;
                    }

                    processes.push(GpuProcessInfo {
                        pid,
                        name,
                        gpu_usage: sm_usage,
                        vram: fb_mem_mb * 1024 * 1024,
                        process_type,
                    });
                }
            }
        }

        // Fallback: query-compute-apps + query-graphics-apps if pmon returned nothing
        if processes.is_empty() {
            // Get compute processes
            if let Ok(output) = self.run_nvidia_smi(vec![
                "--query-compute-apps=pid,process_name,used_memory",
                "--format=csv,noheader,nounits",
            ]) {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                    if parts.len() >= 3 {
                        let pid = parts[0].parse::<u32>().unwrap_or(0);
                        if pid == 0 {
                            continue;
                        }
                        let name = parts[1].rsplit('/').next().unwrap_or(parts[1]).to_string();
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
            if let Ok(output) = self.run_nvidia_smi(vec![
                "--query-graphics-apps=pid,process_name,used_memory",
                "--format=csv,noheader,nounits",
            ]) {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                    if parts.len() >= 3 {
                        let pid = parts[0].parse::<u32>().unwrap_or(0);
                        if pid == 0 {
                            continue;
                        }

                        if let Some(existing) = processes.iter_mut().find(|p| p.pid == pid) {
                            existing.process_type = "Compute+Graphics".to_string();
                            let new_vram = parts[2].parse::<u64>().unwrap_or(0) * 1024 * 1024;
                            if new_vram > existing.vram {
                                existing.vram = new_vram;
                            }
                            continue;
                        }

                        let name = parts[1].rsplit('/').next().unwrap_or(parts[1]).to_string();
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
        }

        // Sort by VRAM usage descending
        processes.sort_by(|a, b| b.vram.cmp(&a.vram));

        Ok(processes)
    }

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

impl GpuMonitorTrait for LinuxGpuMonitor {
    async fn collect_data(&self) -> Result<GpuData> {
        if let Ok(nvidia_data) = self.get_nvidia_smi_linux().await {
            return Ok(nvidia_data);
        }

        anyhow::bail!("No supported GPU detected (nvidia-smi not found or failed)")
    }
}
