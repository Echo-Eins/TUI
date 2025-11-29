use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuData {
    pub name: String,
    pub utilization: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub temperature: f32,
    pub power_usage: f32,
    pub power_limit: f32,
    pub fan_speed: f32,
    pub clock_speed: u32,
    pub memory_clock: u32,
    pub driver_version: String,
    pub processes: Vec<GpuProcessInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuProcessInfo {
    pub pid: u32,
    pub name: String,
    pub gpu_usage: f32,
    pub vram: u64,
    pub process_type: String,
}

pub struct GpuMonitor {
    ps: PowerShellExecutor,
}

impl GpuMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    pub async fn collect_data(&self) -> Result<GpuData> {
        let gpu_info = self.get_gpu_info().await?;
        Ok(gpu_info)
    }

    async fn get_gpu_info(&self) -> Result<GpuData> {
        let script = r#"
            $gpu = Get-CimInstance Win32_VideoController | Select-Object -First 1
            [PSCustomObject]@{
                Name = $gpu.Name
                DriverVersion = $gpu.DriverVersion
                AdapterRAM = $gpu.AdapterRAM
            } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        let info: GpuInfo = serde_json::from_str(&output)
            .context("Failed to parse GPU info")?;

        Ok(GpuData {
            name: info.Name,
            utilization: 0.0,
            memory_used: 0,
            memory_total: info.AdapterRAM,
            temperature: 0.0,
            power_usage: 0.0,
            power_limit: 300.0,
            fan_speed: 0.0,
            clock_speed: 0,
            memory_clock: 0,
            driver_version: info.DriverVersion,
            processes: vec![],
        })
    }
}

#[derive(Debug, Deserialize)]
struct GpuInfo {
    Name: String,
    DriverVersion: String,
    AdapterRAM: u64,
}
