use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::{PowerShellExecutor, LinuxSysMonitor};
use crate::utils::parse_json_array;
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkData {
    pub interfaces: Vec<NetworkInterface>,
    pub connections: Vec<NetworkConnection>,
    pub traffic_history: VecDeque<TrafficSample>,
    pub bandwidth_consumers: Vec<BandwidthConsumer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub description: String,
    pub status: String,
    pub link_speed: String,
    pub mac_address: String,
    pub mtu: u32,
    pub duplex: String,

    // IP Configuration
    pub ipv4_address: String,
    pub ipv6_address: String,
    pub gateway: String,
    pub dns_servers: Vec<String>,

    // Statistics
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub download_speed: f64,  // Mbps
    pub upload_speed: f64,     // Mbps
    pub peak_download: f64,
    pub peak_upload: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConnection {
    pub process_name: String,
    pub pid: u32,
    pub protocol: String,
    pub local_address: String,
    pub local_port: u16,
    pub remote_address: String,
    pub remote_port: u16,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficSample {
    pub timestamp: u64,
    pub download_mbps: f64,
    pub upload_mbps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandwidthConsumer {
    pub process_name: String,
    pub pid: u32,
    pub download_speed: f64,  // Mbps
    pub upload_speed: f64,    // Mbps
    pub total_bytes_received: u64,
    pub total_bytes_sent: u64,
    pub estimated: bool,
}

impl Default for NetworkData {
    fn default() -> Self {
        Self {
            interfaces: Vec::new(),
            connections: Vec::new(),
            traffic_history: VecDeque::with_capacity(60),
            bandwidth_consumers: Vec::new(),
        }
    }
}

pub struct NetworkMonitor {
    ps: PowerShellExecutor,
    #[allow(dead_code)]
    linux_sys: LinuxSysMonitor,
    last_stats: Option<Vec<InterfaceStats>>,
    last_timestamp: Option<std::time::Instant>,
    last_process_stats: Option<std::collections::HashMap<u32, ProcessNetworkStats>>,
}

#[derive(Debug, Clone)]
struct InterfaceStats {
    name: String,
    bytes_received: u64,
    bytes_sent: u64,
}

#[derive(Debug, Clone)]
struct ProcessNetworkStats {
    #[allow(dead_code)]
    process_name: String,
    bytes_received: u64,
    bytes_sent: u64,
}

const INTERFACES_SCRIPT: &str = r#"
    if (-not (Get-Command Get-NetAdapter -ErrorAction SilentlyContinue)) {
        "[]"
    } else {
        try {
            $adapters = Get-NetAdapter -ErrorAction Stop | Where-Object { $_.Status -eq 'Up' }

            $result = foreach ($adapter in $adapters) {
                $stats = Get-NetAdapterStatistics -Name $adapter.Name -ErrorAction SilentlyContinue
                $ipv4 = (Get-NetIPAddress -InterfaceAlias $adapter.Name -AddressFamily IPv4 -ErrorAction SilentlyContinue).IPAddress
                $ipv6 = (Get-NetIPAddress -InterfaceAlias $adapter.Name -AddressFamily IPv6 -ErrorAction SilentlyContinue | Where-Object { $_.PrefixOrigin -ne 'WellKnown' } | Select-Object -First 1).IPAddress
                $gateway = (Get-NetIPConfiguration -InterfaceAlias $adapter.Name -ErrorAction SilentlyContinue).IPv4DefaultGateway.NextHop
                $dns = (Get-DnsClientServerAddress -InterfaceAlias $adapter.Name -AddressFamily IPv4 -ErrorAction SilentlyContinue).ServerAddresses

                [PSCustomObject]@{
                    Name = $adapter.Name
                    Description = $adapter.InterfaceDescription
                    Status = $adapter.Status
                    LinkSpeed = $adapter.LinkSpeed
                    MacAddress = $adapter.MacAddress
                    MTU = $adapter.MtuSize
                    Duplex = $adapter.FullDuplex
                    IPv4 = if ($ipv4) { $ipv4 } else { "N/A" }
                    IPv6 = if ($ipv6) { $ipv6 } else { "N/A" }
                    Gateway = if ($gateway) { $gateway } else { "N/A" }
                    DNS = if ($dns) { $dns -join ', ' } else { "N/A" }
                    BytesReceived = if ($stats) { $stats.ReceivedBytes } else { 0 }
                    BytesSent = if ($stats) { $stats.SentBytes } else { 0 }
                }
            }

            if ($result) {
                $result | ConvertTo-Json -Depth 3
            } else {
                "[]"
            }
        } catch {
            "[]"
        }
    }
"#;

const CONNECTIONS_SCRIPT: &str = r#"
    if (-not (Get-Command Get-NetTCPConnection -ErrorAction SilentlyContinue)) {
        "[]"
    } else {
        try {
            $connections = Get-NetTCPConnection -State Established -ErrorAction Stop |
                Select-Object -First 10 OwningProcess, LocalAddress, LocalPort, RemoteAddress, RemotePort, State

            $result = foreach ($conn in $connections) {
                try {
                    $process = Get-Process -Id $conn.OwningProcess -ErrorAction SilentlyContinue
                    $processName = if ($process) { $process.ProcessName } else { "Unknown" }
                } catch {
                    $processName = "Unknown"
                }

                [PSCustomObject]@{
                    ProcessName = $processName
                    PID = $conn.OwningProcess
                    Protocol = "TCP"
                    LocalAddress = $conn.LocalAddress
                    LocalPort = $conn.LocalPort
                    RemoteAddress = $conn.RemoteAddress
                    RemotePort = $conn.RemotePort
                    State = $conn.State
                }
            }

            if ($result) {
                $result | ConvertTo-Json -Depth 2
            } else {
                "[]"
            }
        } catch {
            "[]"
        }
    }
"#;

const BANDWIDTH_SCRIPT: &str = r#"
    if (-not (Get-Command Get-NetTCPConnection -ErrorAction SilentlyContinue)) {
        "[]"
    } else {
        try {
            $netstat = Get-NetTCPConnection -ErrorAction Stop |
                Where-Object { $_.State -eq 'Established' } |
                Group-Object -Property OwningProcess |
                ForEach-Object {
                    $pid = $_.Name
                    try {
                        $process = Get-Process -Id $pid -ErrorAction SilentlyContinue
                        if ($process) {
                            $connCount = $_.Count

                            [PSCustomObject]@{
                                ProcessName = $process.ProcessName
                                PID = [int]$pid
                                ConnectionCount = $connCount
                            }
                        }
                    } catch {
                    }
                }

            if ($netstat) {
                $netstat | Sort-Object -Property ConnectionCount -Descending |
                    Select-Object -First 10 |
                    ConvertTo-Json -Depth 2
            } else {
                "[]"
            }
        } catch {
            "[]"
        }
    }
"#;

impl NetworkMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            linux_sys: LinuxSysMonitor::new(),
            last_stats: None,
            last_timestamp: None,
            last_process_stats: None,
        })
    }

    pub async fn collect_data(&mut self) -> Result<NetworkData> {
        #[cfg(target_os = "linux")]
        {
            return self.collect_data_linux().await;
        }

        #[cfg(not(target_os = "linux"))]
        {
            return self.collect_data_windows().await;
        }
    }

    #[allow(dead_code)]
    async fn collect_data_linux(&mut self) -> Result<NetworkData> {
        let interfaces = self.get_interfaces_linux().await?;
        let connections = self.get_connections_linux().await?;
        let bandwidth_consumers = Vec::new(); // TODO: Implement for Linux

        // Calculate traffic history
        let traffic_history = self.calculate_traffic_history(&interfaces);

        Ok(NetworkData {
            interfaces,
            connections,
            traffic_history,
            bandwidth_consumers,
        })
    }

    async fn collect_data_windows(&mut self) -> Result<NetworkData> {
        let outputs = self
            .ps
            .execute_batch(&[INTERFACES_SCRIPT, CONNECTIONS_SCRIPT, BANDWIDTH_SCRIPT])
            .await
            .context("Failed to execute network monitor batch")?;
        let interfaces = self.parse_interfaces(&outputs[0])?;
        let connections = self.parse_connections(&outputs[1])?;
        let bandwidth_consumers = self.parse_bandwidth_consumers(&outputs[2])?;

        // Calculate traffic history
        let traffic_history = self.calculate_traffic_history(&interfaces);

        Ok(NetworkData {
            interfaces,
            connections,
            traffic_history,
            bandwidth_consumers,
        })
    }

    // 5.1: Interface Statistics
    fn parse_interfaces(&mut self, output: &str) -> Result<Vec<NetworkInterface>> {
        let interfaces_raw: Vec<InterfaceData> = parse_json_array(output)
            .context("Failed to parse network interface data")?;
        if interfaces_raw.is_empty() {
            return Ok(Vec::new());
        }

        let current_time = std::time::Instant::now();
        let time_delta = if let Some(last_time) = self.last_timestamp {
            current_time.duration_since(last_time).as_secs_f64()
        } else {
            1.0
        };

        let mut interfaces = Vec::new();
        let mut current_stats = Vec::new();

        for iface in interfaces_raw {
            let (download_speed, upload_speed, peak_download, peak_upload) =
                self.calculate_speed(&iface.Name, iface.BytesReceived, iface.BytesSent, time_delta);

            current_stats.push(InterfaceStats {
                name: iface.Name.clone(),
                bytes_received: iface.BytesReceived,
                bytes_sent: iface.BytesSent,
            });

            // Parse DNS servers
            let dns_servers: Vec<String> = if iface.DNS == "N/A" {
                Vec::new()
            } else {
                iface.DNS.split(", ").map(|s| s.to_string()).collect()
            };

            interfaces.push(NetworkInterface {
                name: iface.Name,
                description: iface.Description,
                status: iface.Status,
                link_speed: iface.LinkSpeed,
                mac_address: iface.MacAddress,
                mtu: iface.MTU,
                duplex: if iface.Duplex { "Full".to_string() } else { "Half".to_string() },
                ipv4_address: iface.IPv4,
                ipv6_address: iface.IPv6,
                gateway: iface.Gateway,
                dns_servers,
                bytes_received: iface.BytesReceived,
                bytes_sent: iface.BytesSent,
                download_speed,
                upload_speed,
                peak_download,
                peak_upload,
            });
        }

        self.last_stats = Some(current_stats);
        self.last_timestamp = Some(current_time);

        Ok(interfaces)
    }

    // 5.2: Traffic Monitoring - Calculate speeds
    fn calculate_speed(&self, name: &str, current_rx: u64, current_tx: u64, time_delta: f64) -> (f64, f64, f64, f64) {
        if let Some(ref last_stats) = self.last_stats {
            if let Some(last) = last_stats.iter().find(|s| s.name == name) {
                let bytes_rx_delta = current_rx.saturating_sub(last.bytes_received) as f64;
                let bytes_tx_delta = current_tx.saturating_sub(last.bytes_sent) as f64;

                // Convert bytes/s to Mbps
                let download_speed = (bytes_rx_delta / time_delta) * 8.0 / 1_000_000.0;
                let upload_speed = (bytes_tx_delta / time_delta) * 8.0 / 1_000_000.0;

                // For now, peak is current (will be tracked over time in UI)
                return (download_speed, upload_speed, download_speed, upload_speed);
            }
        }

        (0.0, 0.0, 0.0, 0.0)
    }

    // 5.2: Traffic History for graphs (60s)
    fn calculate_traffic_history(&self, interfaces: &[NetworkInterface]) -> VecDeque<TrafficSample> {
        let mut history = VecDeque::with_capacity(60);

        // Sum all interfaces' traffic
        let total_download: f64 = interfaces.iter().map(|i| i.download_speed).sum();
        let total_upload: f64 = interfaces.iter().map(|i| i.upload_speed).sum();

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        history.push_back(TrafficSample {
            timestamp,
            download_mbps: total_download,
            upload_mbps: total_upload,
        });

        history
    }

    // 5.3: Active Connections
    fn parse_connections(&self, output: &str) -> Result<Vec<NetworkConnection>> {
        let connections_raw: Vec<ConnectionData> = parse_json_array(output)
            .context("Failed to parse connection data")?;
        if connections_raw.is_empty() {
            return Ok(Vec::new());
        }

        let mut connections = Vec::new();
        for conn in connections_raw {
            connections.push(NetworkConnection {
                process_name: conn.ProcessName,
                pid: conn.PID,
                protocol: conn.Protocol,
                local_address: conn.LocalAddress,
                local_port: conn.LocalPort,
                remote_address: conn.RemoteAddress,
                remote_port: conn.RemotePort,
                state: conn.State,
            });
        }

        Ok(connections)
    }

    // 5.4: Bandwidth Consumers - Top processes by network usage
    fn parse_bandwidth_consumers(&mut self, output: &str) -> Result<Vec<BandwidthConsumer>> {
        let consumers_raw: Vec<ProcessBandwidthData> = parse_json_array(output)
            .context("Failed to parse bandwidth consumers data")?;
        if consumers_raw.is_empty() {
            return Ok(Vec::new());
        }

        let current_time = std::time::Instant::now();
        let time_delta = if let Some(last_time) = self.last_timestamp {
            current_time.duration_since(last_time).as_secs_f64()
        } else {
            1.0
        };

        let mut bandwidth_consumers = Vec::new();
        let mut current_process_stats = std::collections::HashMap::new();

        for consumer in consumers_raw {
            // Estimate bandwidth based on connection count
            // This is a rough approximation since Windows doesn't provide per-process network stats
            let connection_count = consumer.ConnectionCount as f64;
            let estimated_bytes = (connection_count * 1024.0 * 100.0) as u64; // ~100KB per connection

            let (download_speed, upload_speed) = if let Some(ref last_stats) = self.last_process_stats {
                if let Some(last) = last_stats.get(&consumer.PID) {
                    let bytes_rx_delta = estimated_bytes.saturating_sub(last.bytes_received) as f64;
                    let bytes_tx_delta = estimated_bytes.saturating_sub(last.bytes_sent) as f64;

                    let download = (bytes_rx_delta / time_delta) * 8.0 / 1_000_000.0; // Mbps
                    let upload = (bytes_tx_delta / time_delta) * 8.0 / 1_000_000.0;   // Mbps

                    (download, upload)
                } else {
                    (0.0, 0.0)
                }
            } else {
                (0.0, 0.0)
            };

            current_process_stats.insert(consumer.PID, ProcessNetworkStats {
                process_name: consumer.ProcessName.clone(),
                bytes_received: estimated_bytes,
                bytes_sent: estimated_bytes,
            });

            bandwidth_consumers.push(BandwidthConsumer {
                process_name: consumer.ProcessName,
                pid: consumer.PID,
                download_speed,
                upload_speed,
                total_bytes_received: estimated_bytes,
                total_bytes_sent: estimated_bytes,
                estimated: true,
            });
        }

        self.last_process_stats = Some(current_process_stats);

        // Sort by total bandwidth (download + upload)
        bandwidth_consumers.sort_by(|a, b| {
            let total_a = a.download_speed + a.upload_speed;
            let total_b = b.download_speed + b.upload_speed;
            total_b.partial_cmp(&total_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(bandwidth_consumers)
    }

    // Linux-specific implementation
    #[allow(dead_code)]
    async fn get_interfaces_linux(&mut self) -> Result<Vec<NetworkInterface>> {
        let linux_interfaces = self.linux_sys.get_network_stats()?;

        let current_time = std::time::Instant::now();
        let time_delta = if let Some(last_time) = self.last_timestamp {
            current_time.duration_since(last_time).as_secs_f64()
        } else {
            1.0
        };

        let mut interfaces = Vec::new();
        let mut current_stats = Vec::new();

        for iface in linux_interfaces {
            let (download_speed, upload_speed, peak_download, peak_upload) =
                self.calculate_speed(&iface.name, iface.rx_bytes, iface.tx_bytes, time_delta);

            current_stats.push(InterfaceStats {
                name: iface.name.clone(),
                bytes_received: iface.rx_bytes,
                bytes_sent: iface.tx_bytes,
            });

            // Try to get IP address using ip command
            let (ipv4, gateway) = self.get_ip_info_linux(&iface.name);

            interfaces.push(NetworkInterface {
                name: iface.name.clone(),
                description: format!("Linux Network Interface {}", iface.name),
                status: "Up".to_string(),
                link_speed: "Unknown".to_string(),
                mac_address: "00:00:00:00:00:00".to_string(),
                mtu: 1500,
                duplex: "Full".to_string(),
                ipv4_address: ipv4,
                ipv6_address: "N/A".to_string(),
                gateway,
                dns_servers: Vec::new(),
                bytes_received: iface.rx_bytes,
                bytes_sent: iface.tx_bytes,
                download_speed,
                upload_speed,
                peak_download,
                peak_upload,
            });
        }

        self.last_stats = Some(current_stats);
        self.last_timestamp = Some(current_time);

        Ok(interfaces)
    }

    #[allow(dead_code)]
    fn get_ip_info_linux(&self, interface: &str) -> (String, String) {
        use std::process::Command;

        // Get IPv4 address
        let ipv4 = if let Ok(output) = Command::new("ip")
            .args(&["addr", "show", interface])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().find(|l| l.contains("inet ")) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 1 {
                    parts[1].split('/').next().unwrap_or("N/A").to_string()
                } else {
                    "N/A".to_string()
                }
            } else {
                "N/A".to_string()
            }
        } else {
            "N/A".to_string()
        };

        // Get default gateway
        let gateway = if let Ok(output) = Command::new("ip").args(&["route"]).output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().find(|l| l.contains("default")) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(pos) = parts.iter().position(|&p| p == "via") {
                    if pos + 1 < parts.len() {
                        parts[pos + 1].to_string()
                    } else {
                        "N/A".to_string()
                    }
                } else {
                    "N/A".to_string()
                }
            } else {
                "N/A".to_string()
            }
        } else {
            "N/A".to_string()
        };

        (ipv4, gateway)
    }

    #[allow(dead_code)]
    async fn get_connections_linux(&self) -> Result<Vec<NetworkConnection>> {
        use std::fs;

        let mut connections = Vec::new();

        // Read /proc/net/tcp for established connections
        if let Ok(content) = fs::read_to_string("/proc/net/tcp") {
            for line in content.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 10 {
                    continue;
                }

                // Parse connection state (0A = LISTEN, 01 = ESTABLISHED, etc.)
                let state_hex = parts.get(3).unwrap_or(&"00");
                if state_hex != &"01" {
                    // Skip non-established connections
                    continue;
                }

                // Parse local address:port
                let local = parts.get(1).unwrap_or(&"00000000:0000");
                let (local_addr, local_port) = self.parse_hex_address(local);

                // Parse remote address:port
                let remote = parts.get(2).unwrap_or(&"00000000:0000");
                let (remote_addr, remote_port) = self.parse_hex_address(remote);

                // Parse inode to find process (simplified - would need to search /proc/*/fd/)
                let uid = parts.get(7).and_then(|s| s.parse().ok()).unwrap_or(0);

                connections.push(NetworkConnection {
                    process_name: "unknown".to_string(),
                    pid: uid, // Using UID as placeholder
                    protocol: "TCP".to_string(),
                    local_address: local_addr,
                    local_port,
                    remote_address: remote_addr,
                    remote_port,
                    state: "ESTABLISHED".to_string(),
                });

                if connections.len() >= 10 {
                    break;
                }
            }
        }

        Ok(connections)
    }

    #[allow(dead_code)]
    fn parse_hex_address(&self, hex_addr: &str) -> (String, u16) {
        let parts: Vec<&str> = hex_addr.split(':').collect();
        if parts.len() != 2 {
            return ("0.0.0.0".to_string(), 0);
        }

        // Parse IP address (little-endian hex)
        let ip_hex = parts[0];
        if ip_hex.len() == 8 {
            let ip = format!(
                "{}.{}.{}.{}",
                u8::from_str_radix(&ip_hex[6..8], 16).unwrap_or(0),
                u8::from_str_radix(&ip_hex[4..6], 16).unwrap_or(0),
                u8::from_str_radix(&ip_hex[2..4], 16).unwrap_or(0),
                u8::from_str_radix(&ip_hex[0..2], 16).unwrap_or(0)
            );
            let port = u16::from_str_radix(parts[1], 16).unwrap_or(0);
            (ip, port)
        } else {
            ("0.0.0.0".to_string(), 0)
        }
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct InterfaceData {
    Name: String,
    Description: String,
    Status: String,
    LinkSpeed: String,
    MacAddress: String,
    MTU: u32,
    Duplex: bool,
    IPv4: String,
    IPv6: String,
    Gateway: String,
    DNS: String,
    BytesReceived: u64,
    BytesSent: u64,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct ConnectionData {
    ProcessName: String,
    PID: u32,
    Protocol: String,
    LocalAddress: String,
    LocalPort: u16,
    RemoteAddress: String,
    RemotePort: u16,
    State: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct ProcessBandwidthData {
    ProcessName: String,
    PID: u32,
    ConnectionCount: u32,
}
