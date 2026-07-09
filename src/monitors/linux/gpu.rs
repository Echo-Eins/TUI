use crate::monitors::traits::*;
use crate::monitors::types::*;
use crate::utils::process::run_command_with_timeout;
use anyhow::Result;
use std::time::Duration;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const NVIDIA_GPU_QUERY: &str = concat!(
    "--query-gpu=",
    "index,uuid,name,pci.bus_id,temperature.gpu,utilization.gpu,utilization.memory,",
    "memory.used,memory.total,power.draw,power.limit,power.default_limit,fan.speed,",
    "clocks.current.graphics,clocks.current.memory,driver_version"
);

pub struct LinuxGpuMonitor {}

#[derive(Debug, PartialEq)]
struct NvidiaGpuRow {
    index: u32,
    uuid: String,
    name: String,
    bus_id: String,
    temperature: Option<f32>,
    utilization_gpu: Option<f32>,
    memory_used_mib: Option<u64>,
    memory_total_mib: Option<u64>,
    power_draw: Option<f32>,
    power_limit: Option<f32>,
    power_default_limit: Option<f32>,
    fan_speed: Option<f32>,
    graphics_clock: Option<u32>,
    memory_clock: Option<u32>,
    driver_version: String,
}

impl LinuxGpuMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {})
    }

    async fn get_nvidia_smi_linux(&self) -> Result<GpuData> {
        let output = self.run_nvidia_smi([NVIDIA_GPU_QUERY, "--format=csv,noheader,nounits"])?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut rows = Self::parse_gpu_rows(&stdout)?;
        rows.sort_by_key(|row| row.index);
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("nvidia-smi returned no GPU rows"))?;

        let memory_used = row.memory_used_mib.unwrap_or(0).saturating_mul(1024 * 1024);
        let memory_total = row
            .memory_total_mib
            .unwrap_or(0)
            .saturating_mul(1024 * 1024);
        let effective_power_limit = row
            .power_limit
            .filter(|value| *value > 0.0)
            .or_else(|| row.power_default_limit.filter(|value| *value > 0.0))
            .or_else(|| self.get_nvidia_power_limit_from_standard_output())
            .unwrap_or(0.0);

        let cuda_version = self
            .get_nvidia_cuda_version()
            .unwrap_or_else(|| "N/A".to_string());

        let processes = self
            .get_gpu_processes_linux(row.index, &row.uuid)
            .await
            .unwrap_or_default();

        Ok(GpuData {
            name: row.name,
            gpu_index: row.index,
            utilization: row.utilization_gpu.unwrap_or(0.0).clamp(0.0, 100.0),
            memory_used,
            memory_total,
            temperature: row.temperature.unwrap_or(0.0),
            power_usage: row.power_draw.unwrap_or(0.0),
            power_limit: effective_power_limit,
            fan_speed: row.fan_speed.unwrap_or(-1.0),
            clock_speed: row.graphics_clock.unwrap_or(0),
            memory_clock: row.memory_clock.unwrap_or(0),
            driver_version: row.driver_version,
            bus_id: row.bus_id,
            cuda_version,
            processes,
        })
    }

    fn parse_gpu_rows(output: &str) -> Result<Vec<NvidiaGpuRow>> {
        let mut rows = Vec::new();
        for (line_number, line) in output.trim_start_matches('\u{feff}').lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let fields = parse_csv_line(line);
            if fields.len() != 16 {
                anyhow::bail!(
                    "invalid nvidia-smi row {}: expected 16 fields, got {}",
                    line_number + 1,
                    fields.len()
                );
            }
            let index = parse_optional_u32(&fields[0]).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid nvidia-smi GPU index on row {}: {:?}",
                    line_number + 1,
                    fields[0]
                )
            })?;
            rows.push(NvidiaGpuRow {
                index,
                uuid: fields[1].trim().to_string(),
                name: fields[2].trim().to_string(),
                bus_id: fields[3].trim().to_string(),
                temperature: parse_optional_f32(&fields[4]),
                utilization_gpu: parse_optional_f32(&fields[5]),
                memory_used_mib: parse_optional_u64(&fields[7]),
                memory_total_mib: parse_optional_u64(&fields[8]),
                power_draw: parse_optional_f32(&fields[9]),
                power_limit: parse_optional_f32(&fields[10]),
                power_default_limit: parse_optional_f32(&fields[11]),
                fan_speed: parse_optional_f32(&fields[12]),
                graphics_clock: parse_optional_u32(&fields[13]),
                memory_clock: parse_optional_u32(&fields[14]),
                driver_version: fields[15].trim().to_string(),
            });
        }
        Ok(rows)
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

    async fn get_gpu_processes_linux(
        &self,
        selected_gpu_index: u32,
        selected_gpu_uuid: &str,
    ) -> Result<Vec<GpuProcessInfo>> {
        let mut processes = Vec::new();

        if let Ok(output) = self.run_nvidia_smi(vec![
            "--query-compute-apps=gpu_uuid,pid,process_name,used_memory",
            "--format=csv,noheader,nounits",
        ]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts = parse_csv_line(line);
                if parts.len() >= 4 && parts[0].trim() == selected_gpu_uuid {
                    let pid = parts[1].parse::<u32>().unwrap_or(0);
                    if pid == 0 {
                        continue;
                    }
                    let name = parts[2].rsplit('/').next().unwrap_or(&parts[2]).to_string();
                    let vram = parse_optional_u64(&parts[3])
                        .unwrap_or(0)
                        .saturating_mul(1024 * 1024);
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

        if let Ok(output) = self.run_nvidia_smi(vec![
            "--query-graphics-apps=gpu_uuid,pid,process_name,used_memory",
            "--format=csv,noheader,nounits",
        ]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts = parse_csv_line(line);
                if parts.len() >= 4 && parts[0].trim() == selected_gpu_uuid {
                    let pid = parts[1].parse::<u32>().unwrap_or(0);
                    if pid == 0 {
                        continue;
                    }
                    let vram = parse_optional_u64(&parts[3])
                        .unwrap_or(0)
                        .saturating_mul(1024 * 1024);
                    if let Some(existing) = processes.iter_mut().find(|process| process.pid == pid)
                    {
                        existing.process_type = "Compute+Graphics".to_string();
                        existing.vram = existing.vram.max(vram);
                    } else {
                        let name = parts[2].rsplit('/').next().unwrap_or(&parts[2]).to_string();
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

        // pmon column layouts vary by driver. Only its stable leading columns
        // (gpu, pid, type, sm) are used to enrich already identified processes.
        if let Ok(output) = self.run_nvidia_smi(vec!["pmon", "-s", "u", "-c", "1"]) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 4
                    || line.trim_start().starts_with('#')
                    || parse_optional_u32(parts[0]) != Some(selected_gpu_index)
                {
                    continue;
                }
                let Some(pid) = parse_optional_u32(parts[1]) else {
                    continue;
                };
                let Some(existing) = processes.iter_mut().find(|process| process.pid == pid) else {
                    continue;
                };
                if let Some(usage) = parse_optional_f32(parts[3]) {
                    existing.gpu_usage = usage.clamp(0.0, 100.0);
                }
            }
        }

        processes.sort_by(|a, b| b.vram.cmp(&a.vram));

        Ok(processes)
    }

    fn run_nvidia_smi<I, S>(&self, args: I) -> Result<std::process::Output>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = run_command_with_timeout("nvidia-smi", args, COMMAND_TIMEOUT);
        match output {
            Ok(out) if out.status.success() => Ok(out),
            Ok(out) => anyhow::bail!("nvidia-smi exited with status {}", out.status),
            Err(e) => anyhow::bail!("nvidia-smi command not found: {}", e),
        }
    }
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(field.trim().to_string());
                field.clear();
            }
            _ => field.push(ch),
        }
    }
    fields.push(field.trim().to_string());
    fields
}

fn numeric_token(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("n/a")
        || trimmed.eq_ignore_ascii_case("[n/a]")
        || trimmed.eq_ignore_ascii_case("not supported")
        || trimmed.eq_ignore_ascii_case("[not supported]")
        || trimmed == "-"
    {
        return None;
    }
    trimmed.split_whitespace().next()
}

fn parse_optional_f32(value: &str) -> Option<f32> {
    numeric_token(value)?
        .trim_end_matches('%')
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_optional_u64(value: &str) -> Option<u64> {
    numeric_token(value)?.parse::<u64>().ok()
}

fn parse_optional_u32(value: &str) -> Option<u32> {
    numeric_token(value)?.parse::<u32>().ok()
}

impl GpuMonitorTrait for LinuxGpuMonitor {
    async fn collect_data(&self) -> Result<GpuData> {
        if let Ok(nvidia_data) = self.get_nvidia_smi_linux().await {
            return Ok(nvidia_data);
        }

        anyhow::bail!("No supported GPU detected (nvidia-smi not found or failed)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_gpus_and_optional_values() {
        let output = concat!(
            "1, GPU-b, \"NVIDIA RTX, Special\", 00000000:02:00.0, 61, 80 %, 10, ",
            "4096 MiB, 8192 MiB, 175.5 W, 220 W, 220 W, 45 %, 1905 MHz, 7001 MHz, 555.42\n",
            "0, GPU-a, NVIDIA A100, 00000000:01:00.0, N/A, 12, 0, ",
            "1024, 40960, [Not Supported], N/A, 250, N/A, 1410, 1215, 550.90\n"
        );

        let rows = LinuxGpuMonitor::parse_gpu_rows(output).expect("valid rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].index, 1);
        assert_eq!(rows[0].name, "NVIDIA RTX, Special");
        assert_eq!(rows[0].memory_used_mib, Some(4096));
        assert_eq!(rows[0].utilization_gpu, Some(80.0));
        assert_eq!(rows[1].temperature, None);
        assert_eq!(rows[1].power_default_limit, Some(250.0));
        assert_eq!(rows[1].fan_speed, None);
    }

    #[test]
    fn rejects_truncated_gpu_rows() {
        let error = LinuxGpuMonitor::parse_gpu_rows("0, GPU-a, NVIDIA GPU")
            .expect_err("truncated row must fail");
        assert!(error.to_string().contains("expected 16 fields"));
    }

    #[test]
    fn parses_quoted_csv_and_missing_numeric_values() {
        assert_eq!(
            parse_csv_line("1,\"GPU, Workstation\",\"quoted \"\"name\"\"\""),
            vec!["1", "GPU, Workstation", "quoted \"name\""]
        );
        assert_eq!(parse_optional_f32("[Not Supported]"), None);
        assert_eq!(parse_optional_f32("42.5 %"), Some(42.5));
        assert_eq!(parse_optional_u64("8192 MiB"), Some(8192));
    }
}
