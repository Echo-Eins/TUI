use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;

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

pub struct RamMonitor {
    ps: PowerShellExecutor,
}

impl RamMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    pub async fn collect_data(&self) -> Result<RamData> {
        let memory_info = self.get_memory_info().await?;
        let physical_memory = self.get_physical_memory_info().await?;

        Ok(RamData {
            total: memory_info.TotalVisibleMemorySize * 1024,
            used: (memory_info.TotalVisibleMemorySize - memory_info.FreePhysicalMemory) * 1024,
            available: memory_info.FreePhysicalMemory * 1024,
            cached: if memory_info.TotalVisibleMemorySize > memory_info.FreePhysicalMemory {
                ((memory_info.TotalVisibleMemorySize - memory_info.FreePhysicalMemory) as f64 * 0.3) as u64 * 1024
            } else {
                0
            },
            free: memory_info.FreePhysicalMemory * 1024,
            speed: physical_memory.speed,
            type_name: physical_memory.memory_type,
        })
    }

    async fn get_memory_info(&self) -> Result<Win32OperatingSystem> {
        let script = r#"
            Get-CimInstance Win32_OperatingSystem | Select-Object TotalVisibleMemorySize, FreePhysicalMemory | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        serde_json::from_str(&output).context("Failed to parse memory info")
    }

    async fn get_physical_memory_info(&self) -> Result<PhysicalMemoryInfo> {
        let script = r#"
            $mem = Get-CimInstance Win32_PhysicalMemory | Select-Object -First 1
            [PSCustomObject]@{
                Speed = "$($mem.Speed) MHz"
                MemoryType = switch ($mem.MemoryType) {
                    20 { "DDR" }
                    21 { "DDR2" }
                    24 { "DDR3" }
                    26 { "DDR4" }
                    34 { "DDR5" }
                    default { "Unknown" }
                }
            } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        let info: PhysicalMemory = serde_json::from_str(&output)
            .context("Failed to parse physical memory info")?;

        Ok(PhysicalMemoryInfo {
            speed: info.Speed,
            memory_type: info.MemoryType,
        })
    }
}

#[derive(Debug, Deserialize)]
struct Win32OperatingSystem {
    TotalVisibleMemorySize: u64,
    FreePhysicalMemory: u64,
}

#[derive(Debug, Deserialize)]
struct PhysicalMemory {
    Speed: String,
    MemoryType: String,
}

#[derive(Debug)]
struct PhysicalMemoryInfo {
    speed: String,
    memory_type: String,
}
