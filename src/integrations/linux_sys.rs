use anyhow::{Context, Result};
use std::fs;
use std::process::Command;

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
            swap_total,
            swap_used: swap_total - swap_free,
        })
    }

    // Disk functions
    pub fn get_disk_info(&self) -> Result<Vec<DiskInfo>> {
        let output = Command::new("df")
            .args(&["-B1", "-T"])  // Block size 1 byte, show type
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut disks = Vec::new();

        for line in stdout.lines().skip(1) {  // Skip header
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
            if fs_type == "tmpfs" || fs_type == "devtmpfs" || mount_point.starts_with("/sys") || mount_point.starts_with("/proc") {
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

    // Network functions
    pub fn get_network_stats(&self) -> Result<Vec<NetworkInterface>> {
        let content = fs::read_to_string("/proc/net/dev")?;
        let mut interfaces = Vec::new();

        for line in content.lines().skip(2) {  // Skip first 2 header lines
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
            let pages: Vec<u64> = statm.split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            pages.get(1).unwrap_or(&0) * 4096  // RSS in pages * page size (4096)
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
    pub swap_total: u64,
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
