use super::LinuxSysMonitor;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

impl LinuxSysMonitor {
    pub fn get_cpu_usage(&self) -> Result<f32> {
        let stat1 = self.read_cpu_stat()?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        let stat2 = self.read_cpu_stat()?;

        let total_diff = stat2.total().saturating_sub(stat1.total());
        let idle_diff = stat2.idle.saturating_sub(stat1.idle);

        if total_diff == 0 {
            return Ok(0.0);
        }

        let usage = 100.0 * (1.0 - (idle_diff as f64 / total_diff as f64));
        Ok(usage.clamp(0.0, 100.0) as f32)
    }

    pub fn get_cpu_info(&self) -> Result<CpuInfo> {
        let content = fs::read_to_string("/proc/cpuinfo")?;
        let mut name = String::from("Unknown CPU");
        let mut logical_count = 0usize;
        let mut physical_ids = HashSet::new();
        let mut core_ids_per_physical: HashMap<String, HashSet<String>> = HashMap::new();
        let mut current_physical_id = String::new();
        let mut avg_mhz: Vec<f32> = Vec::new();
        let mut siblings: usize = 0;
        let mut cores_per_socket: usize = 0;

        for line in content.lines() {
            if line.starts_with("model name") {
                if let Some(value) = line.split(':').nth(1) {
                    name = value.trim().to_string();
                }
            } else if line.starts_with("processor") {
                logical_count += 1;
            } else if line.starts_with("physical id") {
                if let Some(value) = line.split(':').nth(1) {
                    current_physical_id = value.trim().to_string();
                    physical_ids.insert(current_physical_id.clone());
                }
            } else if line.starts_with("core id") {
                if let Some(value) = line.split(':').nth(1) {
                    let core_id = value.trim().to_string();
                    core_ids_per_physical
                        .entry(current_physical_id.clone())
                        .or_default()
                        .insert(core_id);
                }
            } else if line.starts_with("cpu MHz") {
                if let Some(value) = line.split(':').nth(1) {
                    if let Ok(freq) = value.trim().parse::<f32>() {
                        avg_mhz.push(freq);
                    }
                }
            } else if line.starts_with("siblings") {
                if let Some(value) = line.split(':').nth(1) {
                    if let Ok(s) = value.trim().parse::<usize>() {
                        siblings = s;
                    }
                }
            } else if line.starts_with("cpu cores") {
                if let Some(value) = line.split(':').nth(1) {
                    if let Ok(c) = value.trim().parse::<usize>() {
                        cores_per_socket = c;
                    }
                }
            }
        }

        // Calculate physical core count
        let num_sockets = physical_ids.len().max(1);
        let physical_cores = if cores_per_socket > 0 {
            cores_per_socket * num_sockets
        } else {
            // Fallback: count unique core_ids across all physical packages
            let total: usize = core_ids_per_physical.values().map(|s| s.len()).sum();
            if total > 0 {
                total
            } else {
                logical_count
            }
        };

        let thread_count = if logical_count > 0 {
            logical_count
        } else if siblings > 0 {
            siblings * num_sockets
        } else {
            physical_cores
        };

        // Average frequency from current MHz readings
        let current_mhz = if !avg_mhz.is_empty() {
            avg_mhz.iter().sum::<f32>() / avg_mhz.len() as f32
        } else {
            0.0
        };

        // Try to get max frequency from sysfs
        let max_mhz = self.get_max_frequency_mhz().unwrap_or(current_mhz);
        let base_mhz = self.get_base_frequency_mhz().unwrap_or(max_mhz);

        Ok(CpuInfo {
            name,
            core_count: physical_cores,
            thread_count,
            current_frequency_mhz: current_mhz,
            max_frequency_mhz: max_mhz,
            base_frequency_mhz: base_mhz,
        })
    }

    fn get_max_frequency_mhz(&self) -> Option<f32> {
        // Try cpuinfo_max_freq first (in kHz)
        if let Ok(content) =
            fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
        {
            if let Ok(khz) = content.trim().parse::<f32>() {
                return Some(khz / 1000.0);
            }
        }
        // Try scaling_max_freq
        if let Ok(content) =
            fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq")
        {
            if let Ok(khz) = content.trim().parse::<f32>() {
                return Some(khz / 1000.0);
            }
        }
        None
    }

    fn get_base_frequency_mhz(&self) -> Option<f32> {
        // Try base_frequency (in kHz)
        if let Ok(content) =
            fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency")
        {
            if let Ok(khz) = content.trim().parse::<f32>() {
                return Some(khz / 1000.0);
            }
        }
        // Try cpuinfo_min_freq as a rough base
        if let Ok(content) =
            fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_min_freq")
        {
            if let Ok(khz) = content.trim().parse::<f32>() {
                return Some(khz / 1000.0);
            }
        }
        None
    }

    /// Get per-core usage by reading individual cpuN lines from /proc/stat
    pub fn get_per_core_usage(&self) -> Result<Vec<f32>> {
        let stat1 = self.read_all_cpu_stats()?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        let stat2 = self.read_all_cpu_stats()?;

        // Build a HashMap from stat2 for quick lookup
        let stat2_map: HashMap<String, CpuStat> = stat2.into_iter().collect();

        let mut result = Vec::new();
        for (name, s1) in &stat1 {
            if name == "cpu" {
                continue; // Skip the total line
            }
            if let Some(s2) = stat2_map.get(name) {
                let total_diff = s2.total().saturating_sub(s1.total());
                let idle_diff = s2.idle.saturating_sub(s1.idle);
                let usage: f64 = if total_diff > 0 {
                    100.0 * (1.0 - (idle_diff as f64 / total_diff as f64))
                } else {
                    0.0
                };
                result.push(usage.clamp(0.0, 100.0) as f32);
            }
        }

        if result.is_empty() {
            let usage = self.get_cpu_usage()?;
            let info = self.get_cpu_info()?;
            result = vec![usage; info.thread_count];
        }

        Ok(result)
    }

    fn read_all_cpu_stats(&self) -> Result<Vec<(String, CpuStat)>> {
        let content = fs::read_to_string("/proc/stat")?;
        let mut stats = Vec::new();

        for line in content.lines() {
            if !line.starts_with("cpu") {
                continue;
            }

            let mut parts = line.split_whitespace();
            let name = parts.next().unwrap_or("").to_string();
            let values: Vec<u64> = parts.filter_map(|s| s.parse().ok()).collect();

            stats.push((
                name,
                CpuStat {
                    user: *values.get(0).unwrap_or(&0),
                    nice: *values.get(1).unwrap_or(&0),
                    system: *values.get(2).unwrap_or(&0),
                    idle: *values.get(3).unwrap_or(&0),
                    iowait: *values.get(4).unwrap_or(&0),
                    irq: *values.get(5).unwrap_or(&0),
                    softirq: *values.get(6).unwrap_or(&0),
                },
            ));
        }

        Ok(stats)
    }

    fn read_cpu_stat(&self) -> Result<CpuStat> {
        let content = fs::read_to_string("/proc/stat")?;
        let line = content.lines().next().context("Empty /proc/stat")?;

        let values: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();

        Ok(CpuStat {
            user: *values.get(0).unwrap_or(&0),
            nice: *values.get(1).unwrap_or(&0),
            system: *values.get(2).unwrap_or(&0),
            idle: *values.get(3).unwrap_or(&0),
            iowait: *values.get(4).unwrap_or(&0),
            irq: *values.get(5).unwrap_or(&0),
            softirq: *values.get(6).unwrap_or(&0),
        })
    }

    /// Read CPU temperature from hwmon or thermal zones
    pub fn get_cpu_temperature(&self) -> Option<f32> {
        // Try hwmon first (more accurate)
        if let Some(temp) = self.get_temperature_from_hwmon() {
            return Some(temp);
        }
        // Fallback to thermal zones
        self.get_temperature_from_thermal_zone()
    }

    fn get_temperature_from_hwmon(&self) -> Option<f32> {
        let hwmon_dir = Path::new("/sys/class/hwmon");
        if !hwmon_dir.exists() {
            return None;
        }

        for entry in fs::read_dir(hwmon_dir).ok()?.flatten() {
            let path = entry.path();
            // Check if this is a CPU temperature sensor
            let name = fs::read_to_string(path.join("name")).unwrap_or_default();
            let name = name.trim();
            if name == "coretemp"
                || name == "k10temp"
                || name == "zenpower"
                || name == "cpu_thermal"
            {
                // Read temp1_input (in millidegrees)
                for i in 1..=16 {
                    let temp_path = path.join(format!("temp{}_input", i));
                    if let Ok(content) = fs::read_to_string(&temp_path) {
                        if let Ok(millideg) = content.trim().parse::<f32>() {
                            return Some(millideg / 1000.0);
                        }
                    }
                }
            }
        }
        None
    }

    fn get_temperature_from_thermal_zone(&self) -> Option<f32> {
        let thermal_dir = Path::new("/sys/class/thermal");
        if !thermal_dir.exists() {
            return None;
        }

        let mut max_temp: Option<f32> = None;
        for entry in fs::read_dir(thermal_dir).ok()?.flatten() {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with("thermal_zone") {
                continue;
            }

            // Check type for CPU-related zones
            let zone_type = fs::read_to_string(path.join("type")).unwrap_or_default();
            let zone_type = zone_type.trim();
            let is_cpu = zone_type.contains("cpu")
                || zone_type.contains("CPU")
                || zone_type.contains("x86_pkg")
                || zone_type.contains("acpitz")
                || zone_type.contains("coretemp");

            if is_cpu || zone_type.is_empty() {
                if let Ok(content) = fs::read_to_string(path.join("temp")) {
                    if let Ok(millideg) = content.trim().parse::<f32>() {
                        let temp = millideg / 1000.0;
                        if temp > 0.0 && temp < 150.0 {
                            max_temp = Some(max_temp.map_or(temp, |m: f32| m.max(temp)));
                        }
                    }
                }
            }
        }
        max_temp
    }

    /// Read CPU power consumption from RAPL (Running Average Power Limit)
    pub fn get_cpu_power(&self) -> Option<(f32, f32)> {
        // Try reading RAPL power data
        let rapl_dir = Path::new("/sys/class/powercap");
        if !rapl_dir.exists() {
            return None;
        }

        for entry in fs::read_dir(rapl_dir).ok()?.flatten() {
            let path = entry.path();
            let name = fs::read_to_string(path.join("name")).unwrap_or_default();
            if name.trim() == "package-0" {
                let energy_uj = fs::read_to_string(path.join("energy_uj"))
                    .ok()?
                    .trim()
                    .parse::<u64>()
                    .ok()?;
                let max_power_uw = fs::read_to_string(path.join("constraint_0_max_power_uw"))
                    .ok()
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .unwrap_or(0.0);

                let tdp = max_power_uw / 1_000_000.0; // Convert uW to W
                                                      // Energy is in microjoules; we'd need two samples to compute power
                                                      // For now, return 0 for current (will be computed by the monitor)
                let _ = energy_uj;
                return Some((0.0, if tdp > 0.0 { tdp } else { 65.0 }));
            }
        }
        None
    }

    /// Check if CPU boost/turbo is enabled via sysfs
    pub fn is_boost_enabled(&self) -> Option<bool> {
        // AMD/generic: /sys/devices/system/cpu/cpufreq/boost (1=enabled)
        if let Ok(content) = fs::read_to_string("/sys/devices/system/cpu/cpufreq/boost") {
            if let Ok(val) = content.trim().parse::<u8>() {
                return Some(val == 1);
            }
        }
        // Intel pstate: /sys/devices/system/cpu/intel_pstate/no_turbo (0=enabled, 1=disabled)
        if let Ok(content) = fs::read_to_string("/sys/devices/system/cpu/intel_pstate/no_turbo") {
            if let Ok(val) = content.trim().parse::<u8>() {
                return Some(val == 0);
            }
        }
        None
    }

    /// Get per-core frequencies from sysfs
    pub fn get_per_core_frequencies(&self) -> Vec<f32> {
        let mut freqs = Vec::new();
        let cpu_dir = Path::new("/sys/devices/system/cpu");
        for i in 0..256 {
            let path = cpu_dir.join(format!("cpu{}/cpufreq/scaling_cur_freq", i));
            match fs::read_to_string(&path) {
                Ok(content) => {
                    if let Ok(khz) = content.trim().parse::<f32>() {
                        freqs.push(khz / 1000.0); // kHz to MHz
                    }
                }
                Err(_) => break,
            }
        }
        freqs
    }
}

#[derive(Debug)]
struct CpuStat {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
}

impl CpuStat {
    fn total(&self) -> u64 {
        self.user + self.nice + self.system + self.idle + self.iowait + self.irq + self.softirq
    }
}

#[derive(Debug)]
pub struct CpuInfo {
    pub name: String,
    pub core_count: usize,
    pub thread_count: usize,
    pub current_frequency_mhz: f32,
    pub max_frequency_mhz: f32,
    pub base_frequency_mhz: f32,
}
