use super::LinuxSysMonitor;
use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::process::Command;

impl LinuxSysMonitor {
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

    /// Returns basic IP info per interface (name, ipv4, ipv6, mac).
    pub fn get_network_interfaces(&self) -> Result<Vec<NetworkInterfaceIpInfo>> {
        let mut map: HashMap<String, NetworkInterfaceIpInfo> = HashMap::new();

        // Parse `ip -o addr show`
        let output = Command::new("ip").args(["-o", "addr", "show"]).output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 4 {
                    continue;
                }
                let iface_name = parts[1].to_string();
                let family = parts[2]; // "inet" or "inet6"
                let addr_cidr = parts[3]; // e.g. "192.168.1.5/24"
                let addr = addr_cidr.split('/').next().unwrap_or("").to_string();

                let entry = map.entry(iface_name.clone()).or_insert_with(|| {
                    NetworkInterfaceIpInfo {
                        name: iface_name,
                        ipv4: String::new(),
                        ipv6: String::new(),
                        mac: String::new(),
                    }
                });

                if family == "inet" && entry.ipv4.is_empty() {
                    entry.ipv4 = addr;
                } else if family == "inet6" {
                    // Prefer global over link-local
                    if entry.ipv6.is_empty() || entry.ipv6.starts_with("fe80") {
                        if !addr.starts_with("fe80") || entry.ipv6.is_empty() {
                            entry.ipv6 = addr;
                        }
                    }
                }
            }
        }

        // Get MAC addresses from /sys/class/net/<iface>/address
        for entry in map.values_mut() {
            let mac_path = format!("/sys/class/net/{}/address", entry.name);
            if let Ok(mac) = fs::read_to_string(&mac_path) {
                let mac = mac.trim().to_string();
                if mac != "00:00:00:00:00:00" {
                    entry.mac = mac;
                }
            }
        }

        Ok(map.into_values().collect())
    }

    /// Returns interface stats suitable for the network monitor.
    pub fn get_network_interfaces_stats(&self) -> Result<Vec<NetworkInterfaceStats>> {
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

            let rx_bytes: u64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let tx_bytes: u64 = parts.get(9).and_then(|s| s.parse().ok()).unwrap_or(0);

            // Read status from sysfs
            let operstate = fs::read_to_string(format!("/sys/class/net/{}/operstate", name))
                .unwrap_or_default()
                .trim()
                .to_string();
            let status = if operstate == "up" {
                "Up".to_string()
            } else if operstate == "down" {
                "Down".to_string()
            } else {
                operstate
            };

            // Read MTU
            let mtu: u32 = fs::read_to_string(format!("/sys/class/net/{}/mtu", name))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(1500);

            // Read duplex
            let duplex = fs::read_to_string(format!("/sys/class/net/{}/duplex", name))
                .unwrap_or_default()
                .trim()
                .to_string();

            // Determine description
            let description = if name.starts_with("wl") {
                "Wireless".to_string()
            } else if name.starts_with("eth") || name.starts_with("en") {
                "Ethernet".to_string()
            } else if name.starts_with("docker") || name.starts_with("br-") {
                "Docker Bridge".to_string()
            } else if name.starts_with("veth") {
                "Virtual Ethernet".to_string()
            } else if name.starts_with("virbr") {
                "Virtual Bridge".to_string()
            } else {
                "Network Interface".to_string()
            };

            interfaces.push(NetworkInterfaceStats {
                name,
                description,
                status,
                link_speed: String::new(),
                mac_address: String::new(),
                mtu,
                duplex,
                ipv4_address: String::new(),
                ipv6_address: String::new(),
                bytes_received: rx_bytes,
                bytes_sent: tx_bytes,
                download_speed: 0.0,
                upload_speed: 0.0,
                peak_download: 0.0,
                peak_upload: 0.0,
            });
        }

        // Score-based sorting: active interfaces with gateway first
        let gateway_iface = self.get_default_gateway_iface();
        interfaces.sort_by(|a, b| {
            let score_a = self.interface_score(a, &gateway_iface);
            let score_b = self.interface_score(b, &gateway_iface);
            score_b.cmp(&score_a)
        });

        Ok(interfaces)
    }

    /// Get network connections from /proc/net/tcp and /proc/net/tcp6.
    pub fn get_network_connections(&self) -> Result<Vec<NetworkConnectionInfo>> {
        let pid_inode_map = self.build_pid_inode_map();
        let mut connections = Vec::new();

        // Parse TCP (IPv4)
        if let Ok(content) = fs::read_to_string("/proc/net/tcp") {
            for line in content.lines().skip(1) {
                if let Some(conn) = self.parse_proc_net_line(line, "TCP", &pid_inode_map) {
                    connections.push(conn);
                }
            }
        }

        // Parse TCP6 (IPv6)
        if let Ok(content) = fs::read_to_string("/proc/net/tcp6") {
            for line in content.lines().skip(1) {
                if let Some(conn) = self.parse_proc_net_line(line, "TCP6", &pid_inode_map) {
                    connections.push(conn);
                }
            }
        }

        // Parse UDP (IPv4)
        if let Ok(content) = fs::read_to_string("/proc/net/udp") {
            for line in content.lines().skip(1) {
                if let Some(conn) = self.parse_proc_net_line(line, "UDP", &pid_inode_map) {
                    connections.push(conn);
                }
            }
        }

        Ok(connections)
    }

    /// Get per-process bandwidth stats from /proc/net/tcp socket mapping.
    pub fn get_process_bandwidth(&self) -> Result<Vec<ProcessBandwidthInfo>> {
        // We approximate per-process network IO from /proc/<pid>/net/dev
        // or use socket inode mapping. For simplicity, gather from /proc/<pid>/net/dev
        // differences, but since that's complex, we'll provide basic data from socket counts.
        let mut results = Vec::new();
        let pid_inode_map = self.build_pid_inode_map();

        // Aggregate socket count per PID as a rough proxy
        let mut pid_sockets: HashMap<u32, (String, u64, u64)> = HashMap::new();

        // Read /proc/net/tcp for connection tracking
        if let Ok(content) = fs::read_to_string("/proc/net/tcp") {
            for line in content.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 10 {
                    continue;
                }
                let inode: u64 = parts[9].parse().unwrap_or(0);
                if inode == 0 {
                    continue;
                }

                let tx_queue_rx_queue = parts.get(4).unwrap_or(&"0:0");
                let queue_parts: Vec<&str> = tx_queue_rx_queue.split(':').collect();
                let tx_queue = u64::from_str_radix(queue_parts.first().unwrap_or(&"0"), 16).unwrap_or(0);
                let rx_queue = u64::from_str_radix(queue_parts.get(1).unwrap_or(&"0"), 16).unwrap_or(0);

                if let Some((pid, name)) = pid_inode_map.get(&inode) {
                    let entry = pid_sockets.entry(*pid).or_insert_with(|| (name.clone(), 0, 0));
                    entry.1 += rx_queue;
                    entry.2 += tx_queue;
                }
            }
        }

        for (pid, (name, rx, tx)) in pid_sockets {
            results.push(ProcessBandwidthInfo {
                pid,
                name,
                bytes_received: rx,
                bytes_sent: tx,
            });
        }

        Ok(results)
    }

    fn get_default_gateway_iface(&self) -> Option<String> {
        let output = Command::new("ip")
            .args(["route", "show", "default"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        // "default via X.X.X.X dev ethN ..."
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(dev_idx) = parts.iter().position(|&p| p == "dev") {
                return parts.get(dev_idx + 1).map(|s| s.to_string());
            }
        }
        None
    }

    fn interface_score(&self, iface: &NetworkInterfaceStats, gateway_iface: &Option<String>) -> i32 {
        let mut score = 0i32;
        if let Some(gw) = gateway_iface {
            if iface.name == *gw {
                score += 100;
            }
        }
        if iface.status == "Up" {
            score += 30;
        }
        if iface.bytes_received > 0 || iface.bytes_sent > 0 {
            score += 10;
        }
        // Penalize virtual interfaces
        if iface.name.starts_with("docker")
            || iface.name.starts_with("veth")
            || iface.name.starts_with("br-")
            || iface.name.starts_with("virbr")
        {
            score -= 40;
        }
        score
    }

    fn build_pid_inode_map(&self) -> HashMap<u64, (u32, String)> {
        let mut map = HashMap::new();

        let proc_dir = match fs::read_dir("/proc") {
            Ok(d) => d,
            Err(_) => return map,
        };

        for entry in proc_dir.flatten() {
            let file_name = entry.file_name();
            let pid_str = match file_name.to_str() {
                Some(s) => s,
                None => continue,
            };
            let pid: u32 = match pid_str.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let fd_dir = format!("/proc/{}/fd", pid);
            let fds = match fs::read_dir(&fd_dir) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let name = fs::read_to_string(format!("/proc/{}/comm", pid))
                .unwrap_or_default()
                .trim()
                .to_string();

            for fd_entry in fds.flatten() {
                if let Ok(link) = fs::read_link(fd_entry.path()) {
                    let link_str = link.to_string_lossy();
                    if let Some(inode_str) = link_str.strip_prefix("socket:[") {
                        if let Some(inode_str) = inode_str.strip_suffix(']') {
                            if let Ok(inode) = inode_str.parse::<u64>() {
                                map.insert(inode, (pid, name.clone()));
                            }
                        }
                    }
                }
            }
        }

        map
    }

    fn parse_proc_net_line(
        &self,
        line: &str,
        protocol: &str,
        pid_map: &HashMap<u64, (u32, String)>,
    ) -> Option<NetworkConnectionInfo> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 10 {
            return None;
        }

        let local = parts[1];
        let remote = parts[2];
        let state_hex = parts[3];
        let inode: u64 = parts[9].parse().unwrap_or(0);

        let (local_address, local_port) = self.parse_addr(local, protocol.contains('6'));
        let (remote_address, remote_port) = self.parse_addr(remote, protocol.contains('6'));

        let state = match state_hex {
            "01" => "ESTABLISHED",
            "02" => "SYN_SENT",
            "03" => "SYN_RECV",
            "04" => "FIN_WAIT1",
            "05" => "FIN_WAIT2",
            "06" => "TIME_WAIT",
            "07" => "CLOSE",
            "08" => "CLOSE_WAIT",
            "09" => "LAST_ACK",
            "0A" => "LISTEN",
            "0B" => "CLOSING",
            _ => "UNKNOWN",
        }
        .to_string();

        let (pid, process_name) = if inode > 0 {
            pid_map
                .get(&inode)
                .map(|(p, n)| (*p, n.clone()))
                .unwrap_or((0, String::new()))
        } else {
            (0, String::new())
        };

        Some(NetworkConnectionInfo {
            process_name,
            pid,
            protocol: protocol.to_string(),
            local_address,
            local_port,
            remote_address,
            remote_port,
            state,
        })
    }

    fn parse_addr(&self, hex_addr: &str, is_ipv6: bool) -> (String, u16) {
        let parts: Vec<&str> = hex_addr.split(':').collect();
        if parts.len() != 2 {
            return (String::new(), 0);
        }

        let port = u16::from_str_radix(parts[1], 16).unwrap_or(0);

        let addr = if is_ipv6 {
            self.parse_ipv6_hex(parts[0])
        } else {
            self.parse_ipv4_hex(parts[0])
        };

        (addr, port)
    }

    fn parse_ipv4_hex(&self, hex: &str) -> String {
        if hex.len() != 8 {
            return hex.to_string();
        }
        let bytes: Vec<u8> = (0..4)
            .filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
            .collect();
        if bytes.len() == 4 {
            // /proc/net/tcp stores in little-endian
            format!("{}.{}.{}.{}", bytes[3], bytes[2], bytes[1], bytes[0])
        } else {
            hex.to_string()
        }
    }

    fn parse_ipv6_hex(&self, hex: &str) -> String {
        if hex.len() != 32 {
            return hex.to_string();
        }
        // /proc/net/tcp6 stores as 4 groups of 4 bytes, each group in host byte order
        let mut groups = Vec::new();
        for i in 0..4 {
            let group = &hex[i * 8..(i + 1) * 8];
            // Each 8-char group is a 32-bit value in host (little-endian) byte order
            let b0 = &group[6..8];
            let b1 = &group[4..6];
            let b2 = &group[2..4];
            let b3 = &group[0..2];
            groups.push(format!("{}{}", b0, b1));
            groups.push(format!("{}{}", b2, b3));
        }
        // Format as proper IPv6
        let full = groups.join(":");
        // Simplify by removing leading zeros in each group
        let simplified: Vec<String> = full
            .split(':')
            .map(|g| {
                let trimmed = g.trim_start_matches('0');
                if trimmed.is_empty() {
                    "0".to_string()
                } else {
                    trimmed.to_string()
                }
            })
            .collect();
        simplified.join(":")
    }
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
pub struct NetworkInterfaceIpInfo {
    pub name: String,
    pub ipv4: String,
    pub ipv6: String,
    pub mac: String,
}

#[derive(Debug)]
pub struct NetworkInterfaceStats {
    pub name: String,
    pub description: String,
    pub status: String,
    pub link_speed: String,
    pub mac_address: String,
    pub mtu: u32,
    pub duplex: String,
    pub ipv4_address: String,
    pub ipv6_address: String,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub download_speed: f64,
    pub upload_speed: f64,
    pub peak_download: f64,
    pub peak_upload: f64,
}

#[derive(Debug)]
pub struct NetworkConnectionInfo {
    pub process_name: String,
    pub pid: u32,
    pub protocol: String,
    pub local_address: String,
    pub local_port: u16,
    pub remote_address: String,
    pub remote_port: u16,
    pub state: String,
}

#[derive(Debug)]
pub struct ProcessBandwidthInfo {
    pub pid: u32,
    pub name: String,
    pub bytes_received: u64,
    pub bytes_sent: u64,
}
