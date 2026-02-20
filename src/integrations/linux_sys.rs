#![allow(dead_code)]

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

pub struct LinuxSysMonitor;

impl LinuxSysMonitor {
    pub fn new() -> Self {
        Self
    }

    // ========== CPU functions ==========

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
            if total > 0 { total } else { logical_count }
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
        if let Ok(content) = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq") {
            if let Ok(khz) = content.trim().parse::<f32>() {
                return Some(khz / 1000.0);
            }
        }
        // Try scaling_max_freq
        if let Ok(content) = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq") {
            if let Ok(khz) = content.trim().parse::<f32>() {
                return Some(khz / 1000.0);
            }
        }
        None
    }

    fn get_base_frequency_mhz(&self) -> Option<f32> {
        // Try base_frequency (in kHz)
        if let Ok(content) = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency") {
            if let Ok(khz) = content.trim().parse::<f32>() {
                return Some(khz / 1000.0);
            }
        }
        // Try cpuinfo_min_freq as a rough base
        if let Ok(content) = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_min_freq") {
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
            if name == "coretemp" || name == "k10temp" || name == "zenpower" || name == "cpu_thermal" {
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

    // ========== Memory functions ==========

    pub fn get_memory_info(&self) -> Result<MemoryInfo> {
        let content = fs::read_to_string("/proc/meminfo")?;
        let mut total = 0u64;
        let mut available = 0u64;
        let mut free = 0u64;
        let mut buffers = 0u64;
        let mut cached = 0u64;
        let mut active = 0u64;
        let mut inactive = 0u64;
        let mut dirty = 0u64;
        let mut shmem = 0u64;
        let mut sreclaimable = 0u64;
        let mut slab = 0u64;
        let mut commit_limit = 0u64;
        let mut committed_as = 0u64;
        let mut swap_total = 0u64;
        let mut swap_free = 0u64;
        let mut page_tables = 0u64;
        let mut bounce = 0u64;
        let mut mapped = 0u64;
        let mut kernel_stack = 0u64;

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }

            let value = parts[1].parse::<u64>().unwrap_or(0);

            match parts[0] {
                "MemTotal:" => total = value * 1024,
                "MemAvailable:" => available = value * 1024,
                "MemFree:" => free = value * 1024,
                "Buffers:" => buffers = value * 1024,
                "Cached:" => cached = value * 1024,
                "Active:" => active = value * 1024,
                "Inactive:" => inactive = value * 1024,
                "Dirty:" => dirty = value * 1024,
                "Shmem:" => shmem = value * 1024,
                "SReclaimable:" => sreclaimable = value * 1024,
                "Slab:" => slab = value * 1024,
                "CommitLimit:" => commit_limit = value * 1024,
                "Committed_AS:" => committed_as = value * 1024,
                "SwapTotal:" => swap_total = value * 1024,
                "SwapFree:" => swap_free = value * 1024,
                "PageTables:" => page_tables = value * 1024,
                "Bounce:" => bounce = value * 1024,
                "Mapped:" => mapped = value * 1024,
                "KernelStack:" => kernel_stack = value * 1024,
                _ => {}
            }
        }

        let used = total.saturating_sub(available);
        let swap_used = swap_total.saturating_sub(swap_free);

        Ok(MemoryInfo {
            total,
            used,
            available,
            free,
            buffers,
            cached,
            active,
            inactive,
            dirty,
            shmem,
            sreclaimable,
            slab,
            commit_limit,
            committed_as,
            swap_total,
            swap_free,
            swap_used,
            page_tables,
            bounce,
            mapped,
            kernel_stack,
        })
    }

    /// Detect and return zram device information
    pub fn get_zram_info(&self) -> Vec<ZramInfo> {
        let mut zram_devices = Vec::new();

        // Check /sys/block/zramN devices
        let block_dir = Path::new("/sys/block");
        if !block_dir.exists() {
            return zram_devices;
        }

        for entry in fs::read_dir(block_dir).into_iter().flatten().flatten() {
            let name = match entry.file_name().to_str() {
                Some(n) if n.starts_with("zram") => n.to_string(),
                _ => continue,
            };

            let path = entry.path();
            let disksize = fs::read_to_string(path.join("disksize"))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);

            if disksize == 0 {
                continue;
            }

            // mm_stat has: orig_data_size compr_data_size mem_used_total mem_limit ...
            let mm_stat = fs::read_to_string(path.join("mm_stat"))
                .unwrap_or_default();
            let mm_parts: Vec<u64> = mm_stat
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();

            let orig_data_size = mm_parts.get(0).copied().unwrap_or(0);
            let compr_data_size = mm_parts.get(1).copied().unwrap_or(0);
            let mem_used_total = mm_parts.get(2).copied().unwrap_or(0);

            let comp_algorithm = fs::read_to_string(path.join("comp_algorithm"))
                .unwrap_or_default()
                .trim()
                .to_string();

            // Extract the active algorithm (marked with [brackets])
            let active_algo = comp_algorithm
                .split_whitespace()
                .find(|s| s.starts_with('[') && s.ends_with(']'))
                .map(|s| s.trim_matches(|c| c == '[' || c == ']').to_string())
                .unwrap_or(comp_algorithm);

            let compression_ratio = if compr_data_size > 0 {
                orig_data_size as f64 / compr_data_size as f64
            } else {
                0.0
            };

            zram_devices.push(ZramInfo {
                name,
                disksize,
                orig_data_size,
                compr_data_size,
                mem_used_total,
                compression_ratio,
                algorithm: active_algo,
            });
        }

        zram_devices
    }

    /// Try to detect memory hardware type/speed from DMI
    pub fn get_memory_hardware_info(&self) -> MemoryHardwareInfo {
        // Try dmidecode if available (requires root)
        if let Some(info) = self.get_memory_from_dmidecode() {
            return info;
        }

        // Try /sys/devices/virtual/dmi
        if let Some(info) = self.get_memory_from_sysfs_dmi() {
            return info;
        }

        MemoryHardwareInfo {
            memory_type: "Unknown".to_string(),
            speed: "Unknown".to_string(),
            form_factor: None,
        }
    }

    fn get_memory_from_dmidecode(&self) -> Option<MemoryHardwareInfo> {
        let output = Command::new("dmidecode")
            .args(["-t", "memory"])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut mem_type = String::new();
        let mut speed = String::new();
        let mut form_factor = String::new();

        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Type:") && !trimmed.contains("Detail") && !trimmed.contains("Error") {
                let val = trimmed.strip_prefix("Type:")?.trim().to_string();
                if val != "Unknown" && !val.is_empty() {
                    mem_type = val;
                }
            } else if trimmed.starts_with("Configured Memory Speed:") || trimmed.starts_with("Speed:") {
                let val = trimmed.split(':').nth(1)?.trim().to_string();
                if val != "Unknown" && !val.is_empty() && speed.is_empty() {
                    speed = val;
                }
            } else if trimmed.starts_with("Form Factor:") {
                let val = trimmed.strip_prefix("Form Factor:")?.trim().to_string();
                if val != "Unknown" && !val.is_empty() {
                    form_factor = val;
                }
            }
        }

        if mem_type.is_empty() && speed.is_empty() {
            return None;
        }

        Some(MemoryHardwareInfo {
            memory_type: if mem_type.is_empty() { "Unknown".to_string() } else { mem_type },
            speed: if speed.is_empty() { "Unknown".to_string() } else { speed },
            form_factor: if form_factor.is_empty() { None } else { Some(form_factor) },
        })
    }

    fn get_memory_from_sysfs_dmi(&self) -> Option<MemoryHardwareInfo> {
        // Some info may be available through /sys/firmware/dmi/tables
        // but usually requires root. Return None for now.
        None
    }

    // ========== Disk functions ==========

    pub fn get_disk_info(&self) -> Result<Vec<DiskInfo>> {
        let output = Command::new("df")
            .args(["-B1", "-T"])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut disks = Vec::new();
        let mut seen_devices: HashMap<String, usize> = HashMap::new();

        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 7 {
                continue;
            }

            let filesystem = parts[0].to_string();
            let fs_type = parts[1].to_string();
            let total = parts[2].parse::<u64>().unwrap_or(0);
            let used = parts[3].parse::<u64>().unwrap_or(0);
            let available = parts[4].parse::<u64>().unwrap_or(0);
            let mount_point = parts[6].to_string();

            // Skip special/virtual filesystems
            if fs_type == "tmpfs"
                || fs_type == "devtmpfs"
                || fs_type == "squashfs"
                || fs_type == "overlay"
                || fs_type == "9p"
                || fs_type == "fuse.snapfuse"
                || mount_point.starts_with("/sys")
                || mount_point.starts_with("/proc")
                || mount_point.starts_with("/run")
                || mount_point.starts_with("/snap")
                || mount_point.starts_with("/dev/shm")
            {
                continue;
            }

            // For btrfs: same device can have multiple subvolumes showing
            // identical total/used/available. Track by device+total to deduplicate
            // the size, but keep all mount points.
            let device_key = format!("{}:{}", filesystem, total);

            disks.push(DiskInfo {
                name: filesystem.clone(),
                mount_point,
                total,
                used,
                available,
                fs_type: fs_type.clone(),
                is_primary_mount: !seen_devices.contains_key(&device_key),
            });

            seen_devices.entry(device_key).or_insert(disks.len() - 1);
        }

        Ok(disks)
    }

    pub fn get_block_devices(&self) -> Result<Vec<BlockDeviceInfo>> {
        let output = Command::new("lsblk")
            .args([
                "-b",
                "-J",
                "-o",
                "NAME,MODEL,TYPE,SIZE,ROTA,TRAN,FSTYPE,MOUNTPOINT,PKNAME,SERIAL,REV,LABEL,UUID",
            ])
            .output()
            .context("Failed to run lsblk")?;

        if !output.status.success() {
            anyhow::bail!("lsblk failed with status {}", output.status);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let root: LsblkRoot =
            serde_json::from_str(&stdout).context("Failed to parse lsblk json")?;

        let mut result = Vec::new();
        for device in root.blockdevices {
            Self::flatten_block_device(None, device, &mut result);
        }

        Ok(result)
    }

    fn flatten_block_device(
        parent: Option<String>,
        device: LsblkEntry,
        result: &mut Vec<BlockDeviceInfo>,
    ) {
        let name = device.name;
        let parent_name = device.pkname.or(parent);

        result.push(BlockDeviceInfo {
            name: name.clone(),
            model: device.model.unwrap_or_default(),
            dev_type: device.dev_type,
            size: device.size.unwrap_or(0),
            rotational: device.rota.unwrap_or(false),
            transport: device.tran.unwrap_or_default(),
            filesystem: device.fstype,
            mount_point: device.mountpoint,
            parent: parent_name.clone(),
            serial: device.serial,
            label: device.label,
            uuid: device.uuid,
        });

        if let Some(children) = device.children {
            for child in children {
                Self::flatten_block_device(Some(name.clone()), child, result);
            }
        }
    }

    /// Get btrfs-specific information if btrfs filesystem is present
    pub fn get_btrfs_info(&self) -> Vec<BtrfsInfo> {
        let mut results = Vec::new();

        // Try btrfs filesystem show
        let output = match Command::new("btrfs")
            .args(["filesystem", "show", "--raw"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return results,
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut current: Option<BtrfsInfo> = None;

        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Label:") {
                if let Some(fs) = current.take() {
                    results.push(fs);
                }

                let label = trimmed
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("none")
                    .trim_matches('\'')
                    .to_string();
                let uuid = trimmed
                    .split("uuid:")
                    .nth(1)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();

                current = Some(BtrfsInfo {
                    label: if label == "none" { String::new() } else { label },
                    uuid,
                    total_size: 0,
                    used: 0,
                    devices: Vec::new(),
                    subvolumes: Vec::new(),
                });
            } else if trimmed.starts_with("Total devices") {
                if let Some(ref mut fs) = current {
                    if let Some(size_str) = trimmed.split("size").nth(1) {
                        // Parse raw byte size
                        if let Ok(bytes) = size_str.trim().parse::<u64>() {
                            fs.total_size = bytes;
                        }
                    }
                }
            } else if trimmed.starts_with("devid") {
                if let Some(ref mut fs) = current {
                    let parts: Vec<&str> = trimmed.split_whitespace().collect();
                    // devid N size XXXX used YYYY path /dev/XXX
                    if let Some(path_idx) = parts.iter().position(|&p| p == "path") {
                        let device_path = parts.get(path_idx + 1).unwrap_or(&"").to_string();
                        fs.devices.push(device_path);
                    }
                }
            }
        }

        if let Some(fs) = current.take() {
            results.push(fs);
        }

        // Try to get subvolume info for each btrfs mount
        for info in &mut results {
            if let Some(mount) = self.find_btrfs_mount(&info.uuid) {
                if let Ok(output) = Command::new("btrfs")
                    .args(["subvolume", "list", "-a", &mount])
                    .output()
                {
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        for line in stdout.lines() {
                            // Format: ID xxx gen yyy top level zzz path <path>
                            if let Some(path_start) = line.find("path ") {
                                let subvol_path = line[path_start + 5..].trim().to_string();
                                info.subvolumes.push(subvol_path);
                            }
                        }
                    }
                }

                // Get usage info
                if let Ok(output) = Command::new("btrfs")
                    .args(["filesystem", "usage", "-b", &mount])
                    .output()
                {
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        for line in stdout.lines() {
                            let trimmed = line.trim();
                            if trimmed.starts_with("Used:") {
                                if let Some(val) = trimmed.split_whitespace().nth(1) {
                                    info.used = val.parse::<u64>().unwrap_or(0);
                                }
                            }
                        }
                    }
                }
            }
        }

        results
    }

    fn find_btrfs_mount(&self, uuid: &str) -> Option<String> {
        let content = fs::read_to_string("/proc/mounts").ok()?;
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && parts[2] == "btrfs" {
                // Check if this mount matches the UUID
                let dev = parts[0];
                // Try to read UUID from the device
                let uuid_path = format!("/dev/disk/by-uuid/{}", uuid);
                if let Ok(target) = fs::read_link(&uuid_path) {
                    let target_name = target.file_name()?.to_str()?;
                    if dev.contains(target_name) {
                        return Some(parts[1].to_string());
                    }
                }
                // Fallback: just return first btrfs mount
                return Some(parts[1].to_string());
            }
        }
        None
    }

    /// Get SMART data for a disk device using smartctl
    pub fn get_smart_data(&self, device_name: &str) -> Option<SmartData> {
        let device_path = format!("/dev/{}", device_name);
        let output = Command::new("smartctl")
            .args(["-a", "-j", &device_path])
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).ok()?;

        let temperature = json
            .get("temperature")
            .and_then(|t| t.get("current"))
            .and_then(|v| v.as_f64())
            .map(|t| t as f32);

        let power_on_hours = json
            .get("power_on_time")
            .and_then(|t| t.get("hours"))
            .and_then(|v| v.as_u64());

        let health = json
            .get("smart_status")
            .and_then(|s| s.get("passed"))
            .and_then(|v| v.as_bool())
            .map(|passed| if passed { "Healthy" } else { "Warning" }.to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        Some(SmartData {
            temperature,
            power_on_hours,
            health_status: health,
        })
    }

    pub fn get_disk_stats(&self) -> Result<Vec<RawDiskStats>> {
        let content = fs::read_to_string("/proc/diskstats")?;
        let mut stats = Vec::new();

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 14 {
                continue;
            }

            let name = parts[2].to_string();
            if name.starts_with("loop") || name.starts_with("ram") {
                continue;
            }

            stats.push(RawDiskStats {
                name,
                reads_completed: parts[3].parse().unwrap_or(0),
                sectors_read: parts[5].parse().unwrap_or(0),
                ms_reading: parts[6].parse().unwrap_or(0),
                writes_completed: parts[7].parse().unwrap_or(0),
                sectors_written: parts[9].parse().unwrap_or(0),
                ms_writing: parts[10].parse().unwrap_or(0),
                io_in_progress: parts[11].parse().unwrap_or(0),
                ms_doing_io: parts[12].parse().unwrap_or(0),
            });
        }

        Ok(stats)
    }

    pub fn get_process_io_samples(&self) -> Result<Vec<ProcessIoSample>> {
        let mut out = Vec::new();
        let now = Instant::now();

        for entry in fs::read_dir("/proc")? {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            let file_name = entry.file_name();
            let file_name = match file_name.to_str() {
                Some(v) => v,
                None => continue,
            };

            let pid = match file_name.parse::<u32>() {
                Ok(v) => v,
                Err(_) => continue,
            };

            let io_path = format!("/proc/{pid}/io");
            let io_content = match fs::read_to_string(io_path) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let mut read_bytes = 0u64;
            let mut write_bytes = 0u64;
            for line in io_content.lines() {
                if let Some(value) = line.strip_prefix("read_bytes:") {
                    read_bytes = value.trim().parse().unwrap_or(0);
                } else if let Some(value) = line.strip_prefix("write_bytes:") {
                    write_bytes = value.trim().parse().unwrap_or(0);
                }
            }

            let name = fs::read_to_string(format!("/proc/{pid}/comm"))
                .map(|v| v.trim().to_string())
                .unwrap_or_else(|_| "unknown".to_string());

            out.push(ProcessIoSample {
                pid,
                name,
                read_bytes,
                write_bytes,
                timestamp: now,
            });
        }

        Ok(out)
    }

    // ========== Network functions ==========

    pub fn get_network_stats(&self) -> Result<Vec<NetworkInterface>> {
        let content = fs::read_to_string("/proc/net/dev")?;
        let mut interfaces = Vec::new();

        for line in content.lines().skip(2) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let name = parts[0].trim_end_matches(':').to_string();

            if name == "lo" {
                continue;
            }

            let rx_bytes = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let rx_packets = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let tx_bytes = parts.get(9).and_then(|s| s.parse().ok()).unwrap_or(0);
            let tx_packets = parts.get(10).and_then(|s| s.parse().ok()).unwrap_or(0);

            interfaces.push(NetworkInterface {
                name,
                rx_bytes,
                rx_packets,
                tx_bytes,
                tx_packets,
            });
        }

        Ok(interfaces)
    }

    // ========== Process functions ==========

    pub fn get_processes(&self) -> Result<Vec<ProcessInfo>> {
        let mut processes = Vec::new();

        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let path = entry.path();
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                if let Ok(pid) = filename.parse::<u32>() {
                    if let Ok(process) = self.get_process_info(pid) {
                        processes.push(process);
                    }
                }
            }
        }

        Ok(processes)
    }

    fn get_process_info(&self, pid: u32) -> Result<ProcessInfo> {
        let stat_path = format!("/proc/{}/stat", pid);
        let cmdline_path = format!("/proc/{}/cmdline", pid);
        let status_path = format!("/proc/{}/status", pid);

        let stat = fs::read_to_string(&stat_path)?;

        // Extract name from stat (it's in parentheses)
        // Handle names with spaces/parens by finding last ')'
        let name = if let Some(start) = stat.find('(') {
            if let Some(end) = stat.rfind(')') {
                stat[start + 1..end].to_string()
            } else {
                String::from("unknown")
            }
        } else {
            String::from("unknown")
        };

        // Read cmdline
        let cmdline = fs::read_to_string(&cmdline_path)
            .ok()
            .map(|s| s.replace('\0', " ").trim().to_string())
            .filter(|s| !s.is_empty());

        // Parse values from stat - fields after the closing paren
        let after_name = stat.rfind(')').map(|i| &stat[i + 2..]).unwrap_or("");
        let stat_fields: Vec<&str> = after_name.split_whitespace().collect();

        // Field 17 (0-indexed from after name) = num_threads
        let threads = stat_fields.get(17).and_then(|s| s.parse().ok()).unwrap_or(1);

        // Get CPU times: utime (field 11) and stime (field 12) from after name
        let utime = stat_fields.get(11).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let stime = stat_fields.get(12).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let cpu_ticks = utime + stime;

        // Read memory from statm
        let statm_path = format!("/proc/{}/statm", pid);
        let memory = if let Ok(statm) = fs::read_to_string(&statm_path) {
            let pages: Vec<u64> = statm
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            pages.get(1).unwrap_or(&0) * 4096 // RSS in pages * page size
        } else {
            0
        };

        // Read UID from status for user info
        let uid = if let Ok(status) = fs::read_to_string(&status_path) {
            status.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0)
        } else {
            0
        };

        // Resolve UID to username
        let user = self.uid_to_username(uid);

        Ok(ProcessInfo {
            pid,
            name,
            cmdline,
            threads,
            memory,
            cpu_ticks,
            uid,
            user,
        })
    }

    fn uid_to_username(&self, uid: u32) -> String {
        if uid == 0 {
            return "root".to_string();
        }

        // Try reading /etc/passwd
        if let Ok(content) = fs::read_to_string("/etc/passwd") {
            for line in content.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 {
                    if let Ok(file_uid) = parts[2].parse::<u32>() {
                        if file_uid == uid {
                            return parts[0].to_string();
                        }
                    }
                }
            }
        }

        uid.to_string()
    }
}

// ========== Data types ==========

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

#[derive(Debug)]
pub struct MemoryInfo {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub free: u64,
    pub buffers: u64,
    pub cached: u64,
    pub active: u64,
    pub inactive: u64,
    pub dirty: u64,
    pub shmem: u64,
    pub sreclaimable: u64,
    pub slab: u64,
    pub commit_limit: u64,
    pub committed_as: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub swap_used: u64,
    pub page_tables: u64,
    pub bounce: u64,
    pub mapped: u64,
    pub kernel_stack: u64,
}

#[derive(Debug, Clone)]
pub struct ZramInfo {
    pub name: String,
    pub disksize: u64,          // Uncompressed limit
    pub orig_data_size: u64,    // Original data stored
    pub compr_data_size: u64,   // Compressed data size
    pub mem_used_total: u64,    // Total memory used by zram (includes overhead)
    pub compression_ratio: f64,
    pub algorithm: String,
}

#[derive(Debug, Clone)]
pub struct MemoryHardwareInfo {
    pub memory_type: String,
    pub speed: String,
    pub form_factor: Option<String>,
}

#[derive(Debug)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub fs_type: String,
    pub is_primary_mount: bool,
}

#[derive(Debug, Clone)]
pub struct BlockDeviceInfo {
    pub name: String,
    pub model: String,
    pub dev_type: String,
    pub size: u64,
    pub rotational: bool,
    pub transport: String,
    pub filesystem: Option<String>,
    pub mount_point: Option<String>,
    pub parent: Option<String>,
    pub serial: Option<String>,
    pub label: Option<String>,
    pub uuid: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BtrfsInfo {
    pub label: String,
    pub uuid: String,
    pub total_size: u64,
    pub used: u64,
    pub devices: Vec<String>,
    pub subvolumes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SmartData {
    pub temperature: Option<f32>,
    pub power_on_hours: Option<u64>,
    pub health_status: String,
}

#[derive(Debug, Clone)]
pub struct RawDiskStats {
    pub name: String,
    pub reads_completed: u64,
    pub sectors_read: u64,
    pub ms_reading: u64,
    pub writes_completed: u64,
    pub sectors_written: u64,
    pub ms_writing: u64,
    pub io_in_progress: u64,
    pub ms_doing_io: u64,
}

#[derive(Debug, Clone)]
pub struct ProcessIoSample {
    pub pid: u32,
    pub name: String,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub timestamp: Instant,
}

#[derive(Debug, serde::Deserialize)]
struct LsblkRoot {
    blockdevices: Vec<LsblkEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct LsblkEntry {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "model")]
    model: Option<String>,
    #[serde(rename = "type")]
    dev_type: String,
    #[serde(rename = "size")]
    size: Option<u64>,
    #[serde(rename = "rota")]
    rota: Option<bool>,
    #[serde(rename = "tran")]
    tran: Option<String>,
    #[serde(rename = "fstype")]
    fstype: Option<String>,
    #[serde(rename = "mountpoint")]
    mountpoint: Option<String>,
    #[serde(rename = "pkname")]
    pkname: Option<String>,
    #[serde(rename = "children")]
    children: Option<Vec<LsblkEntry>>,
    #[serde(rename = "serial")]
    serial: Option<String>,
    #[serde(rename = "label")]
    label: Option<String>,
    #[serde(rename = "uuid")]
    uuid: Option<String>,
}

#[derive(Debug)]
pub struct NetworkInterface {
    pub name: String,
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
}

#[derive(Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cmdline: Option<String>,
    pub threads: usize,
    pub memory: u64,
    pub cpu_ticks: u64,
    pub uid: u32,
    pub user: String,
}
