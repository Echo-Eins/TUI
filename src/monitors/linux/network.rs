use crate::integrations::LinuxSysMonitor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use anyhow::Result;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

pub struct LinuxNetworkMonitor {
    linux_sys: LinuxSysMonitor,
    traffic_history: Mutex<VecDeque<TrafficSample>>,
    last_network_stats: Mutex<Option<(Instant, HashMap<String, (u64, u64)>)>>,
    last_process_stats: Mutex<Option<(Instant, HashMap<u32, (u64, u64)>)>>,
    peak_interface_speeds: Mutex<HashMap<String, (f64, f64)>>,
}

impl LinuxNetworkMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            linux_sys: LinuxSysMonitor::new(),
            traffic_history: Mutex::new(VecDeque::with_capacity(60)),
            last_network_stats: Mutex::new(None),
            last_process_stats: Mutex::new(None),
            peak_interface_speeds: Mutex::new(HashMap::new()),
        })
    }
}

impl NetworkMonitorTrait for LinuxNetworkMonitor {
    async fn collect_data(&self) -> Result<NetworkData> {
        // Optional IP fallback if low-level interface stats miss address fields.
        let ip_info = self.linux_sys.get_network_interfaces().unwrap_or_default();

        // Get interface statistics
        let mut ifaces = self.linux_sys.get_network_interfaces_stats()?;
        let now = Instant::now();

        // Calculate Speeds
        let mut last_stats = self.last_network_stats.lock();
        let mut total_download_mbps = 0.0;
        let mut total_upload_mbps = 0.0;

        let mut current_stats = HashMap::new();

        if let Some((last_time, prev_stats)) = last_stats.as_ref() {
            let elapsed = now.saturating_duration_since(*last_time).as_secs_f64();
            if elapsed > 0.0 {
                for iface in &mut ifaces {
                    current_stats
                        .insert(iface.name.clone(), (iface.bytes_received, iface.bytes_sent));

                    if let Some((prev_rx, prev_tx)) = prev_stats.get(&iface.name) {
                        let rx = iface.bytes_received.saturating_sub(*prev_rx);
                        let tx = iface.bytes_sent.saturating_sub(*prev_tx);

                        iface.download_speed = (rx as f64 * 8.0) / (1_000_000.0 * elapsed);
                        iface.upload_speed = (tx as f64 * 8.0) / (1_000_000.0 * elapsed);

                        // Aggregate total speed (ignore loopback for totals if plausible)
                        if iface.name != "lo" {
                            total_download_mbps += iface.download_speed;
                            total_upload_mbps += iface.upload_speed;
                        }
                    }
                }
            }
        } else {
            for iface in &ifaces {
                current_stats.insert(iface.name.clone(), (iface.bytes_received, iface.bytes_sent));
            }
        }

        let download_mbps = total_download_mbps;
        let upload_mbps = total_upload_mbps;

        *last_stats = Some((now, current_stats));
        drop(last_stats);

        // Merge stats with IP info and maintain peak rates across monitor lifetime.
        let mut peak_speeds = self.peak_interface_speeds.lock();
        let mut interfaces = Vec::new();
        for mut iface in ifaces {
            if let Some(info) = ip_info.iter().find(|i| i.name == iface.name) {
                if iface.ipv4_address.is_empty() {
                    iface.ipv4_address = info.ipv4.clone();
                }
                if iface.ipv6_address.is_empty() {
                    iface.ipv6_address = info.ipv6.clone();
                }
                if iface.mac_address.is_empty() {
                    iface.mac_address = info.mac.clone();
                }
            }

            if iface.link_speed == "Unknown"
                && (iface.download_speed > 0.0 || iface.upload_speed > 0.0)
            {
                iface.link_speed =
                    format!("{:.1} Mbps", iface.download_speed.max(iface.upload_speed));
            }

            let entry = peak_speeds.entry(iface.name.clone()).or_insert((0.0, 0.0));
            entry.0 = entry.0.max(iface.download_speed);
            entry.1 = entry.1.max(iface.upload_speed);

            interfaces.push(NetworkInterface {
                name: iface.name,
                description: iface.description,
                status: iface.status,
                link_speed: iface.link_speed,
                mac_address: iface.mac_address,
                mtu: iface.mtu,
                duplex: iface.duplex,
                ipv4_address: iface.ipv4_address,
                ipv6_address: iface.ipv6_address,
                gateway: match (iface.gateway, iface.gateway_port) {
                    (Some(gw), Some(port)) => format!("{gw}:{port}"),
                    (Some(gw), None) => gw,
                    (None, _) => String::new(),
                },
                dns_servers: iface.dns_servers,
                bytes_received: iface.bytes_received,
                bytes_sent: iface.bytes_sent,
                download_speed: iface.download_speed,
                upload_speed: iface.upload_speed,
                peak_download: entry.0,
                peak_upload: entry.1,
            });
        }
        interfaces.sort_by(|a, b| {
            let a_score = interface_sort_score(a);
            let b_score = interface_sort_score(b);
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
        });

        // Get connections
        let conns = self.linux_sys.get_network_connections()?;
        let connections = conns
            .into_iter()
            .map(|c| NetworkConnection {
                process_name: c.process_name,
                pid: c.pid,
                protocol: c.protocol,
                local_address: c.local_address,
                local_port: c.local_port,
                remote_address: c.remote_address,
                remote_port: c.remote_port,
                state: c.state,
            })
            .collect();

        // Get bandwidth consumers (per-process network)
        let proc_bw = self.linux_sys.get_process_bandwidth()?;
        let mut last_proc = self.last_process_stats.lock();
        let mut consumers = Vec::new();
        let mut current_proc_stats = HashMap::new();

        if let Some((last_time, prev_stats)) = last_proc.as_ref() {
            let elapsed = now.saturating_duration_since(*last_time).as_secs_f64();
            if elapsed > 0.0 {
                for pb in &proc_bw {
                    current_proc_stats.insert(pb.pid, (pb.bytes_received, pb.bytes_sent));

                    if let Some((prev_rx, prev_tx)) = prev_stats.get(&pb.pid) {
                        let rx = pb.bytes_received.saturating_sub(*prev_rx);
                        let tx = pb.bytes_sent.saturating_sub(*prev_tx);

                        if rx > 0 || tx > 0 {
                            consumers.push(BandwidthConsumer {
                                process_name: pb.name.clone(),
                                pid: pb.pid,
                                download_speed: (rx as f64 * 8.0) / (1_000_000.0 * elapsed),
                                upload_speed: (tx as f64 * 8.0) / (1_000_000.0 * elapsed),
                                total_bytes_received: pb.bytes_received,
                                total_bytes_sent: pb.bytes_sent,
                                estimated: pb.estimated,
                            });
                        }
                    }
                }
            }
        } else {
            for pb in &proc_bw {
                current_proc_stats.insert(pb.pid, (pb.bytes_received, pb.bytes_sent));
            }
        }
        *last_proc = Some((now, current_proc_stats));
        drop(last_proc);

        // Sort consumers by total speed
        consumers.sort_by(|a, b| {
            let a_total = a.download_speed + a.upload_speed;
            let b_total = b.download_speed + b.upload_speed;
            b_total
                .partial_cmp(&a_total)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        consumers.truncate(15);

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

        Ok(NetworkData {
            interfaces,
            connections,
            traffic_history: history.clone(),
            bandwidth_consumers: consumers,
        })
    }
}

fn interface_sort_score(iface: &NetworkInterface) -> f64 {
    let mut score = 0.0;
    if iface.name != "lo" {
        score += 1000.0;
    }
    if iface.status.eq_ignore_ascii_case("connected") {
        score += 400.0;
    }
    if !iface.gateway.is_empty() {
        score += 300.0;
    }
    if !iface.ipv4_address.is_empty() {
        score += 200.0;
    }
    if !iface.ipv6_address.is_empty() {
        score += 100.0;
    }
    score + iface.download_speed + iface.upload_speed
}
