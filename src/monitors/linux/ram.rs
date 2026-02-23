use crate::integrations::LinuxSysMonitor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use anyhow::Result;

pub struct LinuxRamMonitor {
    linux_sys: LinuxSysMonitor,
}

impl LinuxRamMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            linux_sys: LinuxSysMonitor::new(),
        })
    }
}

impl RamMonitorTrait for LinuxRamMonitor {
    async fn collect_data(&self) -> Result<RamData> {
        let mem_info = self.linux_sys.get_memory_info()?;

        // Get processes sorted by memory
        let mut processes = self.linux_sys.get_processes().unwrap_or_default();
        processes.sort_by(|a, b| b.memory.cmp(&a.memory));
        let top_processes: Vec<ProcessMemoryInfo> = processes
            .into_iter()
            .filter(|p| p.memory > 0)
            .map(|proc| ProcessMemoryInfo {
                pid: proc.pid,
                name: proc.name,
                working_set: proc.memory,
                private_bytes: proc.memory,
            })
            .collect();

        // Committed memory: use values from /proc/meminfo
        let commit_limit = if mem_info.commit_limit > 0 {
            mem_info.commit_limit
        } else {
            // CommitLimit = total RAM + total swap
            mem_info.total + mem_info.swap_total
        };
        let committed = if mem_info.committed_as > 0 {
            mem_info.committed_as
        } else {
            mem_info.used + mem_info.swap_used
        };
        let commit_percent = if commit_limit > 0 {
            (committed as f64 / commit_limit as f64) * 100.0
        } else {
            0.0
        };

        // Memory breakdown for Linux:
        // in_use = total - available (actual memory actively in use by apps)
        // cached = Cached + SReclaimable (file cache, reclaimable)
        // standby = Buffers (analogous to standby, can be reclaimed)
        // modified = Dirty pages (waiting to be written to disk)
        let total_cached = mem_info.cached + mem_info.sreclaimable;
        let in_use = mem_info.total.saturating_sub(mem_info.available);

        // Swap/pagefile info
        let mut pagefiles = Vec::new();
        if mem_info.swap_total > 0 {
            pagefiles.push(PagefileInfo {
                name: "swap".to_string(),
                total_size: mem_info.swap_total,
                current_usage: mem_info.swap_used,
                peak_usage: mem_info.swap_used,
                usage_percent: if mem_info.swap_total > 0 {
                    (mem_info.swap_used as f64 / mem_info.swap_total as f64) * 100.0
                } else {
                    0.0
                },
            });
        }

        // Detect zram devices
        let zram_infos = self.linux_sys.get_zram_info();
        let zram_devices: Vec<ZramDeviceInfo> = zram_infos
            .iter()
            .map(|z| ZramDeviceInfo {
                name: z.name.clone(),
                disksize: z.disksize,
                orig_data_size: z.orig_data_size,
                compr_data_size: z.compr_data_size,
                mem_used_total: z.mem_used_total,
                compression_ratio: z.compression_ratio,
                algorithm: z.algorithm.clone(),
            })
            .collect();

        // Add zram as additional "swap" entries in pagefiles
        for z in &zram_infos {
            pagefiles.push(PagefileInfo {
                name: format!("zram:{}", z.name),
                total_size: z.disksize,
                current_usage: z.orig_data_size,
                peak_usage: z.orig_data_size,
                usage_percent: if z.disksize > 0 {
                    (z.orig_data_size as f64 / z.disksize as f64) * 100.0
                } else {
                    0.0
                },
            });
        }

        let total_pagefile_size: u64 = pagefiles.iter().map(|p| p.total_size).sum();
        let total_pagefile_used: u64 = pagefiles.iter().map(|p| p.current_usage).sum();

        // Get memory hardware info (type, speed)
        let hw_info = self.linux_sys.get_memory_hardware_info();

        Ok(RamData {
            total: mem_info.total,
            used: mem_info.used,
            available: mem_info.available,
            cached: total_cached,
            free: mem_info.free,
            speed: hw_info.speed,
            type_name: hw_info.memory_type,
            in_use,
            standby: mem_info.buffers,
            modified: mem_info.dirty,
            committed,
            commit_limit,
            commit_percent,
            top_processes,
            pagefiles,
            total_pagefile_size,
            total_pagefile_used,
            zram_devices,
        })
    }
}
