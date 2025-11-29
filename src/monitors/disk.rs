use anyhow::Result;
use serde::{Deserialize, Serialize};

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

pub struct DiskMonitor {}

impl DiskMonitor {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    pub async fn collect_data(&self) -> Result<DiskData> {
        Ok(DiskData {
            drives: vec![
                DriveInfo {
                    letter: "C:".to_string(),
                    name: "Samsung 990 PRO 2TB".to_string(),
                    drive_type: "NVMe SSD".to_string(),
                    total: 2_000_000_000_000,
                    used: 1_200_000_000_000,
                    free: 800_000_000_000,
                    health: 98.0,
                    temperature: Some(42.0),
                },
            ],
        })
    }
}
