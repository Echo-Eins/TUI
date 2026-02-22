use anyhow::Result;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use crate::integrations::LinuxSysMonitor;
use crate::monitors::types::*;
use crate::monitors::traits::*;

pub struct LinuxNetworkMonitor {
    linux_sys: LinuxSysMonitor,
    traffic_history: Mutex<VecDeque<TrafficSample>>,
    last_network_stats: Mutex<Option<(Instant, HashMap<String, (u64, u64)>)>>,
    peak_download: Mutex<HashMap<String, f64>>,
    peak_upload: Mutex<HashMap<String, f64>>,
}

impl LinuxNetworkMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            linux_sys: LinuxSysMonitor::new(),
            traffic_history: Mutex::new(VecDeque::with_capacity(60)),
            last_network_stats: Mutex::new(None),
            peak_download: Mutex::new(HashMap::new()),
            peak_upload: Mutex::new(HashMap::new()),
        })
    }
}

impl NetworkMonitorTrait for LinuxNetworkMonitor {
    async fn collect_data(&self) -> Result<NetworkData> {
        let raw_ifaces = self.linux_sys.get_network_stats()?;
        let now = Instant::now();

        let mut last_stats = self.last_network_stats.lock();
        let mut total_download_mbps = 0.0;
        let mut total_upload_mbps = 0.0;

        let mut current_stats = HashMap::new();
        let mut interfaces = Vec::new();

        // Read gateway and DNS once (shared by all interfaces)
        let gateways = Self::get_gateways();
        let dns_servers = Self::get_dns_servers();

        let mut peak_dl = self.peak_download.lock();
        let mut peak_ul = self.peak_upload.lock();

        for iface in &raw_ifaces {
            current_stats.insert(iface.name.clone(), (iface.rx_bytes, iface.tx_bytes));

            let mut download_speed = 0.0;
            let mut upload_speed = 0.0;

            if let Some((last_time, prev_stats)) = last_stats.as_ref() {
                let elapsed = now.saturating_duration_since(*last_time).as_secs_f64();
                if elapsed > 0.0 {
                    if let Some((prev_rx, prev_tx)) = prev_stats.get(&iface.name) {
                        let rx = iface.rx_bytes.saturating_sub(*prev_rx);
                        let tx = iface.tx_bytes.saturating_sub(*prev_tx);
                        download_speed = (rx as f64 * 8.0) / (1_000_000.0 * elapsed);
                        upload_speed = (tx as f64 * 8.0) / (1_000_000.0 * elapsed);
                    }
                }
            }

            total_download_mbps += download_speed;
            total_upload_mbps += upload_speed;

            // Track peaks
            let pd = peak_dl.entry(iface.name.clone()).or_insert(0.0);
            if download_speed > *pd { *pd = download_speed; }
            let pu = peak_ul.entry(iface.name.clone()).or_insert(0.0);
            if upload_speed > *pu { *pu = upload_speed; }

            // Read link speed from sysfs
            let link_speed = std::fs::read_to_string(format!("/sys/class/net/{}/speed", iface.name))
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok())
                .filter(|&s| s > 0 && s < 1000000)
                .map(|s| format!("{} Mbps", s))
                .unwrap_or_else(|| "Unknown".to_string());

            // Read MAC address
            let mac_address = std::fs::read_to_string(format!("/sys/class/net/{}/address", iface.name))
                .ok()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();

            // Read MTU
            let mtu = std::fs::read_to_string(format!("/sys/class/net/{}/mtu", iface.name))
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(1500);

            // Read operstate
            let status = std::fs::read_to_string(format!("/sys/class/net/{}/operstate", iface.name))
                .ok()
                .map(|s| {
                    let s = s.trim();
                    if s == "up" { "Up".to_string() } else { s.to_string() }
                })
                .unwrap_or_else(|| "Unknown".to_string());

            // Read duplex from sysfs
            let duplex = std::fs::read_to_string(format!("/sys/class/net/{}/duplex", iface.name))
                .ok()
                .map(|s| {
                    match s.trim() {
                        "full" => "Full".to_string(),
                        "half" => "Half".to_string(),
                        other => other.to_string(),
                    }
                })
                .unwrap_or_default();

            // Read IP addresses
            let (ipv4, ipv6) = Self::get_ip_addresses(&iface.name);

            // Find gateway for this interface
            let gateway = gateways.get(&iface.name).cloned().unwrap_or_default();

            interfaces.push(NetworkInterface {
                name: iface.name.clone(),
                description: iface.name.clone(),
                status,
                link_speed,
                mac_address,
                mtu,
                duplex,
                ipv4_address: ipv4,
                ipv6_address: ipv6,
                gateway,
                dns_servers: dns_servers.clone(),
                bytes_received: iface.rx_bytes,
                bytes_sent: iface.tx_bytes,
                download_speed,
                upload_speed,
                peak_download: *peak_dl.get(&iface.name).unwrap_or(&0.0),
                peak_upload: *peak_ul.get(&iface.name).unwrap_or(&0.0),
            });
        }

        *last_stats = Some((now, current_stats));
        drop(last_stats);
        drop(peak_dl);
        drop(peak_ul);

        // Sort interfaces: active ones with gateway/IP first, down ones last
        interfaces.sort_by(|a, b| {
            let score = |iface: &NetworkInterface| -> u32 {
                let mut s = 0u32;
                // Has default gateway → very likely the primary interface
                if !iface.gateway.is_empty() { s += 100; }
                // Has an IPv4 address
                if !iface.ipv4_address.is_empty() { s += 50; }
                // Is up
                if iface.status == "Up" { s += 30; }
                // Has actual traffic
                if iface.bytes_received > 0 || iface.bytes_sent > 0 { s += 10; }
                // Skip virtual/bridge/docker/veth interfaces for primary
                let name = &iface.name;
                if name.starts_with("docker") || name.starts_with("br-")
                    || name.starts_with("veth") || name.starts_with("virbr")
                    || name.starts_with("vnet")
                {
                    s = s.saturating_sub(40);
                }
                s
            };
            score(b).cmp(&score(a))
        });

        // Get connections with PID resolution
        let connections = Self::get_connections();

        // Update traffic history
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut history = self.traffic_history.lock();
        if history.len() >= 60 {
            history.pop_front();
        }
        history.push_back(TrafficSample {
            timestamp: current_time,
            download_mbps: total_download_mbps,
            upload_mbps: total_upload_mbps,
        });

        Ok(NetworkData {
            interfaces,
            connections,
            traffic_history: history.clone(),
            bandwidth_consumers: Vec::new(),
        })
    }
}

impl LinuxNetworkMonitor {
    fn get_ip_addresses(iface_name: &str) -> (String, String) {
        let mut ipv4 = String::new();
        let mut ipv6 = String::new();

        // Try reading from /proc and sysfs first (no subprocess)
        // Parse from `ip -o addr show dev <iface>`
        if let Ok(output) = std::process::Command::new("ip")
            .args(["-o", "addr", "show", "dev", iface_name])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    // Format: idx name family addr/prefix ...
                    // e.g.: 2: eth0    inet 10.0.0.5/24 ...
                    //        2: eth0    inet6 fe80::1/64 scope link ...
                    for (i, part) in parts.iter().enumerate() {
                        if *part == "inet" {
                            if ipv4.is_empty() {
                                if let Some(addr) = parts.get(i + 1) {
                                    ipv4 = addr.split('/').next().unwrap_or("").to_string();
                                }
                            }
                        } else if *part == "inet6" {
                            if let Some(addr) = parts.get(i + 1) {
                                let a = addr.split('/').next().unwrap_or("");
                                // Prefer global address over link-local
                                if ipv6.is_empty() || (ipv6.starts_with("fe80") && !a.starts_with("fe80")) {
                                    ipv6 = a.to_string();
                                }
                            }
                        }
                    }
                }
            }
        }

        (ipv4, ipv6)
    }

    /// Parse default gateways from `ip route`
    fn get_gateways() -> HashMap<String, String> {
        let mut gateways = HashMap::new();

        if let Ok(output) = std::process::Command::new("ip")
            .args(["route"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    // "default via 192.168.1.1 dev eth0 ..."
                    if line.starts_with("default") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        let mut via = None;
                        let mut dev = None;
                        for (i, p) in parts.iter().enumerate() {
                            if *p == "via" { via = parts.get(i + 1).copied(); }
                            if *p == "dev" { dev = parts.get(i + 1).copied(); }
                        }
                        if let (Some(gw), Some(iface)) = (via, dev) {
                            gateways.entry(iface.to_string())
                                .or_insert_with(|| gw.to_string());
                        }
                    }
                }
            }
        }

        gateways
    }

    /// Read DNS servers from /etc/resolv.conf
    fn get_dns_servers() -> Vec<String> {
        let mut servers = Vec::new();

        if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("nameserver") {
                    if let Some(addr) = line.split_whitespace().nth(1) {
                        servers.push(addr.to_string());
                    }
                }
            }
        }

        servers
    }

    fn get_connections() -> Vec<NetworkConnection> {
        // Build inode->PID+name map from /proc
        let inode_map = Self::build_socket_inode_map();

        let mut conns = Vec::new();

        // Parse /proc/net/tcp
        if let Ok(content) = std::fs::read_to_string("/proc/net/tcp") {
            for line in content.lines().skip(1) {
                if let Some(conn) = Self::parse_tcp_line(line, "TCP", &inode_map) {
                    conns.push(conn);
                }
            }
        }

        // Parse /proc/net/tcp6
        if let Ok(content) = std::fs::read_to_string("/proc/net/tcp6") {
            for line in content.lines().skip(1) {
                if let Some(conn) = Self::parse_tcp_line(line, "TCP6", &inode_map) {
                    conns.push(conn);
                }
            }

        }

        // Parse /proc/net/udp
        if let Ok(content) = std::fs::read_to_string("/proc/net/udp") {
            for line in content.lines().skip(1) {
                if let Some(conn) = Self::parse_tcp_line(line, "UDP", &inode_map) {
                    conns.push(conn);
                }
            }
        }

        conns
    }

    /// Build a map of socket inode -> (pid, process_name) by scanning /proc/*/fd
    fn build_socket_inode_map() -> HashMap<u64, (u32, String)> {
        let mut map = HashMap::new();

        let proc_dir = match std::fs::read_dir("/proc") {
            Ok(d) => d,
            Err(_) => return map,
        };

        for entry in proc_dir.flatten() {
            let file_name = entry.file_name();
            let file_name = match file_name.to_str() {
                Some(s) => s,
                None => continue,
            };
            let pid: u32 = match file_name.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let fd_dir = format!("/proc/{}/fd", pid);
            let fd_entries = match std::fs::read_dir(&fd_dir) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let mut proc_name: Option<String> = None;

            for fd_entry in fd_entries.flatten() {
                let link = match std::fs::read_link(fd_entry.path()) {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                let link_str = link.to_string_lossy();
                // socket:[12345]
                if let Some(inode_str) = link_str.strip_prefix("socket:[").and_then(|s| s.strip_suffix(']')) {
                    if let Ok(inode) = inode_str.parse::<u64>() {
                        let name = proc_name.get_or_insert_with(|| {
                            std::fs::read_to_string(format!("/proc/{}/comm", pid))
                                .map(|s| s.trim().to_string())
                                .unwrap_or_default()
                        });
                        map.insert(inode, (pid, name.clone()));
                    }
                }
            }
        }

        map
    }

    fn parse_tcp_line(line: &str, protocol: &str, inode_map: &HashMap<u64, (u32, String)>) -> Option<NetworkConnection> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 10 {
            return None;
        }

        let local = Self::parse_addr_port(parts[1])?;
        let remote = Self::parse_addr_port(parts[2])?;

        let state_num = u8::from_str_radix(parts[3], 16).unwrap_or(0);
        let state = match state_num {
            1 => "ESTABLISHED",
            2 => "SYN_SENT",
            3 => "SYN_RECV",
            4 => "FIN_WAIT1",
            5 => "FIN_WAIT2",
            6 => "TIME_WAIT",
            7 => "CLOSE",
            8 => "CLOSE_WAIT",
            9 => "LAST_ACK",
            10 => "LISTEN",
            11 => "CLOSING",
            _ => "UNKNOWN",
        }.to_string();

        // Field index 9 is the inode
        let inode = parts.get(9).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);

        let (pid, process_name) = if inode > 0 {
            inode_map.get(&inode)
                .map(|(pid, name)| (*pid, name.clone()))
                .unwrap_or((0, String::new()))
        } else {
            (0, String::new())
        };

        Some(NetworkConnection {
            process_name,
            pid,
            protocol: protocol.to_string(),
            local_address: local.0,
            local_port: local.1,
            remote_address: remote.0,
            remote_port: remote.1,
            state,
        })
    }

    fn parse_addr_port(s: &str) -> Option<(String, u16)> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return None;
        }
        let port = u16::from_str_radix(parts[1], 16).ok()?;

        let hex = parts[0];
        let addr = if hex.len() == 8 {
            // IPv4 - stored in little-endian
            let bytes: Vec<u8> = (0..4)
                .map(|i| u8::from_str_radix(&hex[i*2..i*2+2], 16).unwrap_or(0))
                .collect();
            format!("{}.{}.{}.{}", bytes[3], bytes[2], bytes[1], bytes[0])
        } else if hex.len() == 32 {
            // IPv6 - stored as 4 little-endian 32-bit words
            let mut parts_v6 = Vec::new();
            for word_idx in 0..4 {
                let offset = word_idx * 8;
                let b0 = u8::from_str_radix(&hex[offset..offset+2], 16).unwrap_or(0);
                let b1 = u8::from_str_radix(&hex[offset+2..offset+4], 16).unwrap_or(0);
                let b2 = u8::from_str_radix(&hex[offset+4..offset+6], 16).unwrap_or(0);
                let b3 = u8::from_str_radix(&hex[offset+6..offset+8], 16).unwrap_or(0);
                // Little-endian word: bytes are reversed
                parts_v6.push(format!("{:02x}{:02x}", b1, b0));
                parts_v6.push(format!("{:02x}{:02x}", b3, b2));
            }
            // Simple formatting (no compression for clarity)
            parts_v6.join(":")
        } else {
            hex.to_string()
        };

        Some((addr, port))
    }
}
