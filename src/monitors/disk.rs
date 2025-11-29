use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskData {
    pub drives: Vec<DriveInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveInfo {
    pub letter: String,
    pub name: String,
    pub drive_type: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub health: f32,
    pub temperature: Option<f32>,
}

pub struct DiskMonitor {
    ps: PowerShellExecutor,
}

impl DiskMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    pub async fn collect_data(&self) -> Result<DiskData> {
        let drives = self.get_drives().await?;
        Ok(DiskData { drives })
    }

    async fn get_drives(&self) -> Result<Vec<DriveInfo>> {
        let script = r#"
            Get-CimInstance Win32_LogicalDisk | Where-Object { $_.DriveType -eq 3 } | ForEach-Object {
                [PSCustomObject]@{
                    Letter = $_.DeviceID
                    Name = $_.VolumeName
                    Total = $_.Size
                    Free = $_.FreeSpace
                }
            } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let drives: Vec<DriveSample> = serde_json::from_str(&output).unwrap_or_default();

        Ok(drives
            .into_iter()
            .map(|d| DriveInfo {
                letter: d.Letter,
                name: d.Name.unwrap_or_else(|| "Local Disk".to_string()),
                drive_type: "Fixed".to_string(),
                total: d.Total.unwrap_or(0),
                used: d.Total.unwrap_or(0).saturating_sub(d.Free.unwrap_or(0)),
                free: d.Free.unwrap_or(0),
                health: 100.0,
                temperature: None,
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct DriveSample {
    Letter: String,
    Name: Option<String>,
    Total: Option<u64>,
    Free: Option<u64>,
}
