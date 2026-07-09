use super::LinuxSysMonitor;
use crate::utils::process::run_command_with_timeout;
use anyhow::Result;
use std::fs;
use std::path::Path;
use std::time::Duration;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

impl LinuxSysMonitor {
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
            let mm_stat = fs::read_to_string(path.join("mm_stat")).unwrap_or_default();
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
                .find(|s| s.starts_with('['))
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
        let output =
            run_command_with_timeout("dmidecode", ["-t", "memory"], COMMAND_TIMEOUT).ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut mem_type = String::new();
        let mut speed = String::new();
        let mut form_factor = String::new();

        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Type:")
                && !trimmed.contains("Detail")
                && !trimmed.contains("Error")
            {
                let val = trimmed.strip_prefix("Type:")?.trim().to_string();
                if val != "Unknown" && !val.is_empty() {
                    mem_type = val;
                }
            } else if trimmed.starts_with("Configured Memory Speed:")
                || trimmed.starts_with("Speed:")
            {
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
            memory_type: if mem_type.is_empty() {
                "Unknown".to_string()
            } else {
                mem_type
            },
            speed: if speed.is_empty() {
                "Unknown".to_string()
            } else {
                speed
            },
            form_factor: if form_factor.is_empty() {
                None
            } else {
                Some(form_factor)
            },
        })
    }

    fn get_memory_from_sysfs_dmi(&self) -> Option<MemoryHardwareInfo> {
        // Some info may be available through /sys/firmware/dmi/tables
        // but usually requires root. Return None for now.
        None
    }
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
    pub disksize: u64,        // Uncompressed limit
    pub orig_data_size: u64,  // Original data stored
    pub compr_data_size: u64, // Compressed data size
    pub mem_used_total: u64,  // Total memory used by zram (includes overhead)
    pub compression_ratio: f64,
    pub algorithm: String,
}

#[derive(Debug, Clone)]
pub struct MemoryHardwareInfo {
    pub memory_type: String,
    pub speed: String,
    pub form_factor: Option<String>,
}
