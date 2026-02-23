use crate::integrations::LinuxSysMonitor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use anyhow::Result;
use std::fs;

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

/// Parsed entry from /proc/swaps
struct SwapEntry {
    filename: String,
    size_kb: u64,
    used_kb: u64,
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

        // Parse /proc/swaps to separate zram swap from regular (disk) swap.
        // /proc/meminfo SwapTotal includes zram if it's configured as swap,
        // so we must not double-count it.
        let swap_entries = parse_proc_swaps();
        let zram_names: Vec<String> = zram_infos.iter().map(|z| z.name.clone()).collect();

        let mut disk_swap_total: u64 = 0;
        let mut disk_swap_used: u64 = 0;
        let mut zram_swap_total: u64 = 0;

        for entry in &swap_entries {
            let is_zram = zram_names.iter().any(|z| entry.filename.contains(z));
            if is_zram {
                zram_swap_total += entry.size_kb * 1024;
            } else {
                disk_swap_total += entry.size_kb * 1024;
                disk_swap_used += entry.used_kb * 1024;
            }
        }

        // If /proc/swaps was not readable, fall back to /proc/meminfo values
        // but subtract known zram disksizes
        if swap_entries.is_empty() && mem_info.swap_total > 0 {
            let total_zram_disksize: u64 = zram_infos.iter().map(|z| z.disksize).sum();
            disk_swap_total = mem_info.swap_total.saturating_sub(total_zram_disksize);
            disk_swap_used = mem_info.swap_used;
            zram_swap_total = total_zram_disksize;
        }

        // Committed memory calculation.
        // The kernel's CommitLimit includes zram in SwapTotal, but zram uses physical
        // RAM (it's just compressed), so it shouldn't count as additional backing store.
        // Correct commit_limit = RAM + disk-only swap (excluding zram).
        let commit_limit = mem_info.total + disk_swap_total;
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

        // Swap section: only real disk swap, zram shown separately in zram section.
        let mut pagefiles = Vec::new();
        if disk_swap_total > 0 {
            pagefiles.push(PagefileInfo {
                name: "Swap".to_string(),
                total_size: disk_swap_total,
                current_usage: disk_swap_used,
                peak_usage: disk_swap_used,
                usage_percent: if disk_swap_total > 0 {
                    (disk_swap_used as f64 / disk_swap_total as f64) * 100.0
                } else {
                    0.0
                },
            });
        }

        // Show zram as separate swap entries only if they are active as swap
        // (with usage data from /proc/swaps or from zram mm_stat).
        // These are shown alongside disk swap for the total picture.
        for z in &zram_infos {
            if z.orig_data_size > 0 || zram_swap_total > 0 {
                pagefiles.push(PagefileInfo {
                    name: format!("zram ({})", z.algorithm),
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
        }

        // For total swap stats: disk swap + zram actual RAM usage (not disksize).
        // zram's real memory cost is mem_used_total (the compressed data + overhead
        // in physical RAM), not disksize (the virtual uncompressed size).
        let total_zram_mem_used: u64 = zram_infos.iter().map(|z| z.mem_used_total).sum();
        let total_pagefile_size = disk_swap_total
            + zram_infos.iter().map(|z| z.disksize).sum::<u64>();
        let total_pagefile_used = disk_swap_used
            + zram_infos.iter().map(|z| z.orig_data_size).sum::<u64>();

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

/// Parse /proc/swaps to identify individual swap devices and their types.
/// Format:
///   Filename            Type        Size        Used    Priority
///   /dev/sda2           partition   15728640    12345   -2
///   /dev/zram0          partition   8388608     5678    100
fn parse_proc_swaps() -> Vec<SwapEntry> {
    let mut entries = Vec::new();
    let content = match fs::read_to_string("/proc/swaps") {
        Ok(c) => c,
        Err(_) => return entries,
    };

    for line in content.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let filename = parts[0].to_string();
        let size_kb: u64 = parts[2].parse().unwrap_or(0);
        let used_kb: u64 = parts[3].parse().unwrap_or(0);
        entries.push(SwapEntry {
            filename,
            size_kb,
            used_kb,
        });
    }

    entries
}
