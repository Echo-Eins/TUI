#![allow(dead_code)]

use anyhow::{Context, Result};
use std::fs;
use std::process::Command;
use std::time::Instant;

pub struct LinuxSysMonitor;

impl LinuxSysMonitor {
    pub fn new() -> Self {
        Self
    }

    // CPU functions
    pub fn get_cpu_usage(&self) -> Result<f32> {
        let stat1 = self.read_cpu_stat()?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        let stat2 = self.read_cpu_stat()?;

        let total_diff = stat2.total() - stat1.total();
        let idle_diff = stat2.idle - stat1.idle;

        if total_diff == 0 {
            return Ok(0.0);
        }

        let usage = 100.0 * (1.0 - (idle_diff as f64 / total_diff as f64));
        Ok(usage as f32)
    }

    pub fn get_cpu_info(&self) -> Result<CpuInfo> {
        let content = fs::read_to_string("/proc/cpuinfo")?;
        let mut name = String::from("Unknown CPU");
        let mut core_count = 0;
        let mut mhz = 0.0;

        for line in content.lines() {
            if line.starts_with("model name") {
                if let Some(value) = line.split(':').nth(1) {
                    name = value.trim().to_string();
                }
            } else if line.starts_with("processor") {
                core_count += 1;
            } else if line.starts_with("cpu MHz") {
                if let Some(value) = line.split(':').nth(1) {
                    if let Ok(freq) = value.trim().parse::<f32>() {
                        mhz = freq;
                    }
                }
            }
        }

        Ok(CpuInfo {
            name,
            core_count,
            frequency_mhz: mhz,
        })
    }

    pub fn get_core_usage(&self) -> Result<Vec<f32>> {
        // Simplified: return overall usage for each core
        // Full implementation would track each core separately
        let usage = self.get_cpu_usage()?;
        let info = self.get_cpu_info()?;
        Ok(vec![usage; info.core_count])
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

    // Memory functions
    pub fn get_memory_info(&self) -> Result<MemoryInfo> {
        let content = fs::read_to_string("/proc/meminfo")?;
        let mut total = 0;
        let mut available = 0;
        let mut free = 0;
        let mut buffers = 0;
        let mut cached = 0;
        let mut active = 0;
        let mut inactive = 0;
        let mut dirty = 0;
        let mut shmem = 0;
        let mut sreclaimable = 0;
        let mut slab = 0;
        let mut commit_limit = 0;
        let mut committed_as = 0;
        let mut swap_total = 0;
        let mut swap_free = 0;

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }

            let value = parts[1].parse::<u64>().unwrap_or(0);

            match parts[0] {
                "MemTotal:" => total = value * 1024, // Convert KB to bytes
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
                _ => {}
            }
        }

        let used = total - available;

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
            swap_used: swap_total - swap_free,
        })
    }

    // Disk functions
    pub fn get_disk_info(&self) -> Result<Vec<DiskInfo>> {
        let output = Command::new("df")
            .args(&["-B1", "-T"]) // Block size 1 byte, show type
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut disks = Vec::new();

        for line in stdout.lines().skip(1) {
            // Skip header
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

            // Skip special filesystems
            if fs_type == "tmpfs"
                || fs_type == "devtmpfs"
                || mount_point.starts_with("/sys")
                || mount_point.starts_with("/proc")
            {
                continue;
            }

            disks.push(DiskInfo {
                name: filesystem,
                mount_point,
                total,
                used,
                available,
                fs_type,
            });
        }

        Ok(disks)
    }

    pub fn get_block_devices(&self) -> Result<Vec<BlockDeviceInfo>> {
        let output = Command::new("lsblk")
            .args([
                "-b",
                "-J",
                "-o",
                "NAME,MODEL,TYPE,SIZE,ROTA,TRAN,FSTYPE,MOUNTPOINT,PKNAME",
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
        });

        if let Some(children) = device.children {
            for child in children {
                Self::flatten_block_device(Some(name.clone()), child, result);
            }
        }
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

    // Network functions
    pub fn get_network_stats(&self) -> Result<Vec<NetworkInterface>> {
        let content = fs::read_to_string("/proc/net/dev")?;
        let mut interfaces = Vec::new();

        for line in content.lines().skip(2) {
            // Skip first 2 header lines
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let name = parts[0].trim_end_matches(':').to_string();

            // Skip loopback
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

    // Process functions
    pub fn get_processes(&self) -> Result<Vec<ProcessInfo>> {
        let mut processes = Vec::new();

        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let path = entry.path();
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                // Check if directory name is a number (PID)
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

        let stat = fs::read_to_string(&stat_path)?;
        let parts: Vec<&str> = stat.split_whitespace().collect();

        // Extract name from stat (it's in parentheses)
        let name = if let Some(start) = stat.find('(') {
            if let Some(end) = stat.find(')') {
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
            .map(|s| s.replace('\0', " ").trim().to_string());

        // Parse values
        let threads = parts.get(19).and_then(|s| s.parse().ok()).unwrap_or(1);

        // Read memory from statm
        let statm_path = format!("/proc/{}/statm", pid);
        let memory = if let Ok(statm) = fs::read_to_string(&statm_path) {
            let pages: Vec<u64> = statm
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            pages.get(1).unwrap_or(&0) * 4096 // RSS in pages * page size (4096)
        } else {
            0
        };

        Ok(ProcessInfo {
            pid,
            name,
            cmdline,
            threads,
            memory,
        })
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
    pub frequency_mhz: f32,
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
}

#[derive(Debug)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub fs_type: String,
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
}
