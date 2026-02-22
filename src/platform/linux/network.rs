use super::LinuxSysMonitor;
use anyhow::Result;
use std::fs;

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
}

#[derive(Debug)]
pub struct NetworkInterface {
    pub name: String,
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
}
