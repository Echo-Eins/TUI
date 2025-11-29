use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkData {
    pub interfaces: Vec<NetworkInterface>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub status: String,
    pub speed: String,
    pub download_speed: f64,  // Mbps
    pub upload_speed: f64,    // Mbps
    pub total_received: u64,
    pub total_sent: u64,
}

pub struct NetworkMonitor {}

impl NetworkMonitor {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    pub async fn collect_data(&self) -> Result<NetworkData> {
        Ok(NetworkData {
            interfaces: vec![
                NetworkInterface {
                    name: "Ethernet".to_string(),
                    status: "Connected".to_string(),
                    speed: "2.5 Gbps".to_string(),
                    download_speed: 124.5,
                    upload_speed: 45.2,
                    total_received: 45_200_000_000,
                    total_sent: 18_700_000_000,
                },
            ],
        })
    }
}
