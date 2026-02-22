use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use crate::integrations::PowerShellExecutor;
use crate::utils::parse_json_array;
use crate::monitors::types::*;
use crate::monitors::traits::*;
use serde::Deserialize;

pub struct WindowsNetworkMonitor {
    ps: PowerShellExecutor,
    traffic_history: Mutex<VecDeque<TrafficSample>>,
    last_network_stats: Mutex<Option<(Instant, u64, u64)>>, // (Timestamp, TotalReceived, TotalSent)
    bandwidth_consumers: Mutex<HashMap<u32, BandwidthConsumer>>, // PID -> Stats
}

const NETWORK_INTERFACES_SCRIPT: &str = r#"
    try {
        $adapters = Get-CimInstance Win32_NetworkAdapter -Filter "NetConnectionStatus = 2" -ErrorAction SilentlyContinue
        if (-not $adapters) {
            "[]"
            return
        }

        $configs = Get-CimInstance Win32_NetworkAdapterConfiguration -Filter "IPEnabled = True" -ErrorAction SilentlyContinue

        $perfData = Get-CimInstance Win32_PerfFormattedData_Tcpip_NetworkInterface -ErrorAction SilentlyContinue |
            Where-Object { $_.BytesTotalPersec -gt 0 -or $_.CurrentBandwidth -gt 0 }

        $result = foreach ($adapter in $adapters) {
            $config = $configs | Where-Object { $_.Index -eq $adapter.Index }
            $perf = $perfData | Where-Object {
                $safeName = $adapter.Name -replace '[\(\)]', '_' -replace '/', '_' -replace '#', '_'
                $_.Name -match [regex]::Escape($safeName) -or $_.Name -match [regex]::Escape($adapter.MACAddress)
            } | Select-Object -First 1

            $ipv4 = if ($config -and $config.IPAddress) {
                $config.IPAddress | Where-Object { $_ -match '\.' } | Select-Object -First 1
            } else { "" }

            $ipv6 = if ($config -and $config.IPAddress) {
                $config.IPAddress | Where-Object { $_ -match ':' } | Select-Object -First 1
            } else { "" }

            $gateway = if ($config -and $config.DefaultIPGateway) {
                $config.DefaultIPGateway[0]
            } else { "" }

            $dns = if ($config -and $config.DNSServerSearchOrder) {
                $config.DNSServerSearchOrder
            } else { @() }

            $speed = if ($adapter.Speed -gt 0) {
                ($adapter.Speed / 1000000).ToString("0.##") + " Mbps"
            } elseif ($perf -and $perf.CurrentBandwidth -gt 0) {
                ([uint64]$perf.CurrentBandwidth / 1000000).ToString("0.##") + " Mbps"
            } else {
                "Unknown"
            }

            [PSCustomObject]@{
                Name = $adapter.Name
                Description = $adapter.Description
                Status = "Connected"
                LinkSpeed = $speed
                MacAddress = $adapter.MACAddress
                Mtu = if ($config) { [uint32]$config.MTU } else { [uint32]1500 }
                Duplex = "Unknown"
                Ipv4Address = $ipv4
                Ipv6Address = $ipv6
                Gateway = $gateway
                DnsServers = $dns
                BytesReceived = if ($perf) { [uint64]$perf.BytesReceivedPersec } else { [uint64]0 }
                BytesSent = if ($perf) { [uint64]$perf.BytesSentPersec } else { [uint64]0 }
            }
        }
        $result | ConvertTo-Json -Depth 4
    } catch {
        "[]"
    }
"#;

const NETWORK_CONNECTIONS_SCRIPT: &str = r#"
    try {
        $tcp = Get-NetTCPConnection -State Established -ErrorAction SilentlyContinue |
            Select-Object LocalAddress, LocalPort, RemoteAddress, RemotePort, State, OwningProcess,
                          @{Name='Protocol';Expression={'TCP'}}
        $udp = Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
            Select-Object LocalAddress, LocalPort, @{Name='RemoteAddress';Expression={''}},
                          @{Name='RemotePort';Expression={0}}, @{Name='State';Expression={''}},
                          OwningProcess, @{Name='Protocol';Expression={'UDP'}}

        $all = @($tcp) + @($udp) | Group-Object OwningProcess | Sort-Object Count -Descending | Select-Object -First 15 | ForEach-Object {
            $_.Group | Select-Object -First 2
        }

        if (-not $all) {
            "[]"
            return
        }

        $pids = $all | Select-Object -ExpandProperty OwningProcess -Unique | Where-Object { $_ -ne 0 }
        $procMap = @{}
        if ($pids) {
            Get-Process -Id $pids -ErrorAction SilentlyContinue | ForEach-Object {
                $procMap[$_.Id] = $_.ProcessName
            }
        }

        $result = foreach ($conn in $all) {
            $pidVal = if ($conn.OwningProcess) { [uint32]$conn.OwningProcess } else { [uint32]0 }
            [PSCustomObject]@{
                ProcessName = if ($procMap.ContainsKey($pidVal)) { $procMap[$pidVal] } else { "System" }
                Pid = $pidVal
                Protocol = $conn.Protocol
                LocalAddress = $conn.LocalAddress
                LocalPort = [uint16]$conn.LocalPort
                RemoteAddress = $conn.RemoteAddress
                RemotePort = [uint16]$conn.RemotePort
                State = if ($conn.State) { $conn.State.ToString() } else { "" }
            }
        }
        $result | ConvertTo-Json
    } catch {
        "[]"
    }
"#;

impl WindowsNetworkMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            traffic_history: Mutex::new(VecDeque::with_capacity(60)),
            last_network_stats: Mutex::new(None),
            bandwidth_consumers: Mutex::new(HashMap::new()),
        })
    }

    fn parse_interfaces(output: &str) -> Result<Vec<NetworkInterface>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(Vec::new());
        }

        let adapters: Vec<NetworkAdapterSample> = if trimmed.starts_with('[') {
            parse_json_array(trimmed).context("Failed to parse network interfaces array")?
        } else {
            let single: NetworkAdapterSample =
                serde_json::from_str(trimmed).context("Failed to parse single network interface")?;
            vec![single]
        };

        Ok(adapters
            .into_iter()
            .map(|a| NetworkInterface {
                name: a.Name,
                description: a.Description.unwrap_or_default(),
                status: a.Status.unwrap_or_else(|| "Unknown".to_string()),
                link_speed: a.LinkSpeed.unwrap_or_else(|| "Unknown".to_string()),
                mac_address: a.MacAddress.unwrap_or_default(),
                mtu: a.Mtu.unwrap_or(1500),
                duplex: a.Duplex.unwrap_or_else(|| "Unknown".to_string()),
                ipv4_address: a.Ipv4Address.unwrap_or_default(),
                ipv6_address: a.Ipv6Address.unwrap_or_default(),
                gateway: a.Gateway.unwrap_or_default(),
                dns_servers: a.DnsServers,
                bytes_received: a.BytesReceived.unwrap_or(0),
                bytes_sent: a.BytesSent.unwrap_or(0),
                download_speed: 0.0,
                upload_speed: 0.0,
                peak_download: 0.0,
                peak_upload: 0.0,
            })
            .collect())
    }

    fn parse_connections(output: &str) -> Result<Vec<NetworkConnection>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(Vec::new());
        }

        let connections: Vec<NetworkConnectionSample> = if trimmed.starts_with('[') {
            parse_json_array(trimmed).context("Failed to parse network connections array")?
        } else {
            let single: NetworkConnectionSample =
                serde_json::from_str(trimmed).context("Failed to parse single network connection")?;
            vec![single]
        };

        Ok(connections
            .into_iter()
            .map(|c| NetworkConnection {
                process_name: c.ProcessName,
                pid: c.Pid,
                protocol: c.Protocol,
                local_address: c.LocalAddress.unwrap_or_default(),
                local_port: c.LocalPort.unwrap_or(0),
                remote_address: c.RemoteAddress.unwrap_or_default(),
                remote_port: c.RemotePort.unwrap_or(0),
                state: c.State.unwrap_or_default(),
            })
            .collect())
    }
}

impl NetworkMonitorTrait for WindowsNetworkMonitor {
    // Note: traits.rs updated to use `&self` for `collect_data`
    async fn collect_data(&self) -> Result<NetworkData> {
        let outputs = self
            .ps
            .execute_batch(&[NETWORK_INTERFACES_SCRIPT, NETWORK_CONNECTIONS_SCRIPT])
            .await
            .context("Failed to execute network monitor batch")?;

        let mut interfaces = Self::parse_interfaces(&outputs[0])?;
        let connections = Self::parse_connections(&outputs[1])?;

        // Calculate network speed based on overall stats
        let now = Instant::now();
        let total_received: u64 = interfaces.iter().map(|i| i.bytes_received).sum();
        let total_sent: u64 = interfaces.iter().map(|i| i.bytes_sent).sum();

        let mut download_mbps = 0.0;
        let mut upload_mbps = 0.0;

        let mut last_stats = self.last_network_stats.lock();
        if let Some((last_time, last_received, last_sent)) = *last_stats {
            let elapsed = now.saturating_duration_since(last_time).as_secs_f64();
            if elapsed > 0.0 {
                let bytes_rx = total_received.saturating_sub(last_received);
                let bytes_tx = total_sent.saturating_sub(last_sent);

                download_mbps = (bytes_rx as f64 * 8.0) / (1_000_000.0 * elapsed);
                upload_mbps = (bytes_tx as f64 * 8.0) / (1_000_000.0 * elapsed);

                // Apply rudimentary speed to active interfaces (rough estimate, as WMI BytesPerSec might not be cumulative)
                for interface in &mut interfaces {
                    if interface.bytes_received > 0 || interface.bytes_sent > 0 {
                        interface.download_speed = download_mbps;
                        interface.upload_speed = upload_mbps;
                    }
                }
            }
        }
        *last_stats = Some((now, total_received, total_sent));
        drop(last_stats);

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
            download_mbps,
            upload_mbps,
        });

        // WMI doesn't easily expose per-process bandwidth, so we would need ETW or similar.
        // We'll leave bandwidth consumers empty or estimated for Windows right now.

        Ok(NetworkData {
            interfaces,
            connections,
            traffic_history: history.clone(),
            bandwidth_consumers: Vec::new(),
        })
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct NetworkAdapterSample {
    Name: String,
    Description: Option<String>,
    Status: Option<String>,
    LinkSpeed: Option<String>,
    MacAddress: Option<String>,
    Mtu: Option<u32>,
    Duplex: Option<String>,
    Ipv4Address: Option<String>,
    Ipv6Address: Option<String>,
    Gateway: Option<String>,
    #[serde(default)]
    DnsServers: Vec<String>,
    BytesReceived: Option<u64>,
    BytesSent: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct NetworkConnectionSample {
    ProcessName: String,
    Pid: u32,
    Protocol: String,
    LocalAddress: Option<String>,
    LocalPort: Option<u16>,
    RemoteAddress: Option<String>,
    RemotePort: Option<u16>,
    State: Option<String>,
}
