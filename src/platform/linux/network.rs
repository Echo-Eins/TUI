use super::LinuxSysMonitor;
use anyhow::Result;
use std::fs;

// Minimal IP info structure
#[derive(Debug)]
pub struct IpInfo {
    pub name: String,
    pub ipv4: String,
    pub ipv6: String,
    pub mac: String,
}

// Full Interface stats structure (expected by monitor)
#[derive(Debug)]
pub struct IfStats {
    pub name: String,
    pub description: String,
    pub status: String,
    pub mtu: u32,
    pub duplex: String,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub download_speed: f64,
    pub upload_speed: f64,
    pub peak_download: f64,
    pub peak_upload: f64,
    pub ipv4_address: String,
    pub ipv6_address: String,
    pub mac_address: String,
    pub link_speed: String,
}

// Connection info
#[derive(Debug)]
pub struct NetConnection {
    pub process_name: String,
    pub pid: u32,
    pub protocol: String,
    pub local_address: String,
    pub local_port: u16,
    pub remote_address: String,
    pub remote_port: u16,
    pub state: String,
}

// Process bandwidth info
#[derive(Debug)]
pub struct ProcessBandwidth {
    pub name: String,
    pub pid: u32,
    pub bytes_received: u64,
    pub bytes_sent: u64,
}

impl LinuxSysMonitor {
    pub fn get_network_interfaces(&self) -> Result<Vec<IpInfo>> {
        // Simplified: reading MAC from sysfs, leave IP empty for now (requires netlink or /proc/net/if_inet6)
        // This satisfies the compiler and prevents crashes. Full IP parsing can be added later.
        let mut interfaces = Vec::new();
        if let Ok(entries) = fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let mac_path = entry.path().join("address");
                let mac = fs::read_to_string(mac_path).unwrap_or_default().trim().to_string();
                interfaces.push(IpInfo {
                    name,
                    ipv4: String::new(),
                    ipv6: String::new(),
                    mac,
                });
            }
        }
        Ok(interfaces)
    }

    pub fn get_network_interfaces_stats(&self) -> Result<Vec<IfStats>> {
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

            let bytes_received = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let bytes_sent = parts.get(9).and_then(|s| s.parse().ok()).unwrap_or(0);
            
            let status = fs::read_to_string(format!("/sys/class/net/{}/operstate", name))
                .unwrap_or_else(|_| "unknown".to_string())
                .trim().to_string();
                
            let mtu = fs::read_to_string(format!("/sys/class/net/{}/mtu", name))
                .ok().and_then(|s| s.trim().parse().ok()).unwrap_or(1500);

            interfaces.push(IfStats {
                name: name.clone(),
                description: name,
                status,
                mtu,
                duplex: "Unknown".to_string(),
                bytes_received,
                bytes_sent,
                download_speed: 0.0,
                upload_speed: 0.0,
                peak_download: 0.0,
                peak_upload: 0.0,
                ipv4_address: String::new(),
                ipv6_address: String::new(),
                mac_address: String::new(),
                link_speed: String::new(),
            });
        }

        Ok(interfaces)
    }
    
    pub fn get_network_connections(&self) -> Result<Vec<NetConnection>> {
        // Dummy implementation to satisfy compiler for now.
        // Full implementation requires parsing /proc/net/tcp, /proc/net/udp, and matching inodes to /proc/[pid]/fd.
        Ok(Vec::new())
    }
    
    pub fn get_process_bandwidth(&self) -> Result<Vec<ProcessBandwidth>> {
        // Fallback implementation: use process I/O read/write as a rough proxy for bandwidth
        // since per-process network bandwidth on Linux requires eBPF or libpcap.
        let mut result = Vec::new();
        if let Ok(processes) = self.get_processes() {
            for p in processes {
                result.push(ProcessBandwidth {
                    name: p.name,
                    pid: p.pid,
                    bytes_received: p.io_read_bytes,
                    bytes_sent: p.io_write_bytes,
                });
            }
        }
        Ok(result)
    }
}
