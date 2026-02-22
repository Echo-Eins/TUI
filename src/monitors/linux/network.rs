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
}

impl LinuxNetworkMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            linux_sys: LinuxSysMonitor::new(),
            traffic_history: Mutex::new(VecDeque::with_capacity(60)),
            last_network_stats: Mutex::new(None),
        })
    }
}

impl NetworkMonitorTrait for LinuxNetworkMonitor {
    async fn collect_data(&self) -> Result<NetworkData> {
        // Get raw stats from /proc/net/dev
        let raw_ifaces = self.linux_sys.get_network_stats()?;
        let now = Instant::now();

        let mut last_stats = self.last_network_stats.lock();
        let mut total_download_mbps = 0.0;
        let mut total_upload_mbps = 0.0;

        let mut current_stats = HashMap::new();
        let mut interfaces = Vec::new();

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

            // Read link speed from sysfs
            let link_speed = std::fs::read_to_string(format!("/sys/class/net/{}/speed", iface.name))
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .filter(|&s| s < 1000000)
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

            // Read IP addresses from ip command
            let (ipv4, ipv6) = Self::get_ip_addresses(&iface.name);

            interfaces.push(NetworkInterface {
                name: iface.name.clone(),
                description: iface.name.clone(),
                status,
                link_speed,
                mac_address,
                mtu,
                duplex: String::new(),
                ipv4_address: ipv4,
                ipv6_address: ipv6,
                gateway: String::new(),
                dns_servers: Vec::new(),
                bytes_received: iface.rx_bytes,
                bytes_sent: iface.tx_bytes,
                download_speed,
                upload_speed,
                peak_download: download_speed,
                peak_upload: upload_speed,
            });
        }

        *last_stats = Some((now, current_stats));
        drop(last_stats);

        // Get connections from /proc/net/tcp and /proc/net/tcp6
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
            bandwidth_consumers: Vec::new(), // Per-process bandwidth not easily available on Linux
        })
    }
}

impl LinuxNetworkMonitor {
    fn get_ip_addresses(iface_name: &str) -> (String, String) {
        let mut ipv4 = String::new();
        let mut ipv6 = String::new();

        if let Ok(output) = std::process::Command::new("ip")
            .args(["-o", "addr", "show", "dev", iface_name])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    if parts[2] == "inet" {
                        ipv4 = parts[3].split('/').next().unwrap_or("").to_string();
                    } else if parts[2] == "inet6" {
                        if ipv6.is_empty() {
                            ipv6 = parts[3].split('/').next().unwrap_or("").to_string();
                        }
                    }
                }
            }
        }

        (ipv4, ipv6)
    }

    fn get_connections() -> Vec<NetworkConnection> {
        let mut conns = Vec::new();

        // Parse /proc/net/tcp
        if let Ok(content) = std::fs::read_to_string("/proc/net/tcp") {
            for line in content.lines().skip(1) {
                if let Some(conn) = Self::parse_tcp_line(line, "TCP") {
                    conns.push(conn);
                }
            }
        }

        // Parse /proc/net/tcp6
        if let Ok(content) = std::fs::read_to_string("/proc/net/tcp6") {
            for line in content.lines().skip(1) {
                if let Some(conn) = Self::parse_tcp_line(line, "TCP6") {
                    conns.push(conn);
                }
            }
        }

        conns.truncate(100);
        conns
    }

    fn parse_tcp_line(line: &str, protocol: &str) -> Option<NetworkConnection> {
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

        let uid = parts.get(7).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let inode = parts.get(9).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let _ = (uid, inode); // Could be used to find process name

        Some(NetworkConnection {
            process_name: String::new(),
            pid: 0,
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

        // Parse hex IP (little-endian for IPv4)
        let hex = parts[0];
        let addr = if hex.len() == 8 {
            // IPv4
            let bytes: Vec<u8> = (0..4)
                .map(|i| u8::from_str_radix(&hex[i*2..i*2+2], 16).unwrap_or(0))
                .collect();
            format!("{}.{}.{}.{}", bytes[3], bytes[2], bytes[1], bytes[0])
        } else {
            hex.to_string()
        };

        Some((addr, port))
    }
}
