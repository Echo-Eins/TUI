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
    last_process_stats: Mutex<Option<(Instant, HashMap<u32, (u64, u64)>)>>,
}

impl LinuxNetworkMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            linux_sys: LinuxSysMonitor::new(),
            traffic_history: Mutex::new(VecDeque::with_capacity(60)),
            last_network_stats: Mutex::new(None),
            last_process_stats: Mutex::new(None),
        })
    }
}

impl NetworkMonitorTrait for LinuxNetworkMonitor {
    async fn collect_data(&self) -> Result<NetworkData> {
        // Get IP configurations
        let ip_info = self.linux_sys.get_network_interfaces()?;

        // Get interface statistics
        let mut ifaces = self.linux_sys.get_network_interfaces_stats()?;
        let now = Instant::now();

        // Calculate Speeds
        let mut last_stats = self.last_network_stats.lock();
        let mut download_mbps = 0.0;
        let mut upload_mbps = 0.0;
        let mut total_download_mbps = 0.0;
        let mut total_upload_mbps = 0.0;

        let mut current_stats = HashMap::new();

        if let Some((last_time, prev_stats)) = last_stats.as_ref() {
            let elapsed = now.saturating_duration_since(*last_time).as_secs_f64();
            if elapsed > 0.0 {
                for iface in &mut ifaces {
                    current_stats.insert(iface.name.clone(), (iface.bytes_received, iface.bytes_sent));

                    if let Some((prev_rx, prev_tx)) = prev_stats.get(&iface.name) {
                        let rx = iface.bytes_received.saturating_sub(*prev_rx);
                        let tx = iface.bytes_sent.saturating_sub(*prev_tx);

                        iface.download_speed = (rx as f64 * 8.0) / (1_000_000.0 * elapsed);
                        iface.upload_speed = (tx as f64 * 8.0) / (1_000_000.0 * elapsed);

                        if iface.download_speed > iface.peak_download {
                            iface.peak_download = iface.download_speed;
                        }
                        if iface.upload_speed > iface.peak_upload {
                            iface.peak_upload = iface.upload_speed;
                        }

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

        download_mbps = total_download_mbps;
        upload_mbps = total_upload_mbps;

        *last_stats = Some((now, current_stats));
        drop(last_stats);

        // Merge stats with IP info
        let mut interfaces = Vec::new();
        for mut iface in ifaces {
            if let Some(info) = ip_info.iter().find(|i| i.name == iface.name) {
                iface.ipv4_address = info.ipv4.clone();
                iface.ipv6_address = info.ipv6.clone();
                iface.mac_address = info.mac.clone();
            }

            // Set link speed from sysfs if available, else derive
            let sysfs_speed = std::fs::read_to_string(format!("/sys/class/net/{}/speed", iface.name))
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok());

            if let Some(s) = sysfs_speed {
                if s < 1000000 {
                    iface.link_speed = format!("{} Mbps", s);
                }
            } else if iface.download_speed > 0.0 || iface.upload_speed > 0.0 {
                iface.link_speed = format!("{:.1} Mbps", iface.download_speed.max(iface.upload_speed));
            } else {
                iface.link_speed = "Unknown".to_string();
            }

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
                gateway: "".to_string(), // Need routing table for gateway
                dns_servers: Vec::new(), // Need /etc/resolv.conf for DNS
                bytes_received: iface.bytes_received,
                bytes_sent: iface.bytes_sent,
                download_speed: iface.download_speed,
                upload_speed: iface.upload_speed,
                peak_download: iface.download_speed,
                peak_upload: iface.upload_speed,
            });
        }

        // Get connections
        let conns = self.linux_sys.get_network_connections()?;
        let connections = conns.into_iter().map(|c| NetworkConnection {
            process_name: c.process_name,
            pid: c.pid,
            protocol: c.protocol,
            local_address: c.local_address,
            local_port: c.local_port,
            remote_address: c.remote_address,
            remote_port: c.remote_port,
            state: c.state,
        }).collect();

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
                                total_bytes_received: rx,
                                total_bytes_sent: tx,
                                estimated: false,
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
            b_total.partial_cmp(&a_total).unwrap_or(std::cmp::Ordering::Equal)
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
