use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RamData {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub cached: u64,
    pub free: u64,
    pub speed: String,
    pub type_name: String,
}

pub struct RamMonitor {}

impl RamMonitor {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    pub async fn collect_data(&self) -> Result<RamData> {
        Ok(RamData {
            total: 64_000_000_000,
            used: 42_800_000_000,
            available: 21_200_000_000,
            cached: 18_400_000_000,
            free: 3_100_000_000,
            speed: "DDR5-6000".to_string(),
            type_name: "DDR5".to_string(),
        })
    }
}
