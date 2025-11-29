use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkData {
    pub interfaces: Vec<NetworkInterface>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub status: String,
    pub speed: String,
    pub download_speed: f64,
    pub upload_speed: f64,
    pub total_received: u64,
    pub total_sent: u64,
}

pub struct NetworkMonitor {
    ps: PowerShellExecutor,
}

impl NetworkMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    pub async fn collect_data(&self) -> Result<NetworkData> {
        let interfaces = self.get_interfaces().await?;
        Ok(NetworkData { interfaces })
    }

    async fn get_interfaces(&self) -> Result<Vec<NetworkInterface>> {
        let script = r#"
            Get-NetAdapter | Where-Object { $_.Status -eq 'Up' } | ForEach-Object {
                $stats = Get-NetAdapterStatistics -Name $_.Name
                [PSCustomObject]@{
                    Name = $_.Name
                    Status = $_.Status
                    LinkSpeed = $_.LinkSpeed
                    ReceivedBytes = $stats.ReceivedBytes
                    SentBytes = $stats.SentBytes
                }
            } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let interfaces: Vec<NetworkSample> = serde_json::from_str(&output).unwrap_or_default();

        Ok(interfaces
            .into_iter()
            .map(|iface| NetworkInterface {
                name: iface.Name,
                status: iface.Status,
                speed: iface.LinkSpeed,
                download_speed: 0.0,
                upload_speed: 0.0,
                total_received: iface.ReceivedBytes,
                total_sent: iface.SentBytes,
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct NetworkSample {
    Name: String,
    Status: String,
    LinkSpeed: String,
    ReceivedBytes: u64,
    SentBytes: u64,
}
