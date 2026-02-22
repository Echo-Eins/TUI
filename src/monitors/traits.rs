use anyhow::Result;
use super::types::*;

pub trait CpuMonitorTrait: Send + Sync {
    async fn collect_data(&self) -> Result<CpuData>;
}

pub trait GpuMonitorTrait: Send + Sync {
    async fn collect_data(&self) -> Result<GpuData>;
}

pub trait RamMonitorTrait: Send + Sync {
    async fn collect_data(&self) -> Result<RamData>;
}

pub trait DiskMonitorTrait: Send + Sync {
    async fn collect_data(&self) -> Result<DiskData>;
}

pub trait NetworkMonitorTrait: Send + Sync {
    async fn collect_data(&self) -> Result<NetworkData>;
}

pub trait ProcessMonitorTrait: Send + Sync {
    async fn collect_data(&self) -> Result<ProcessData>;
}

pub trait ServiceMonitorTrait: Send + Sync {
    async fn collect_data(&self) -> Result<ServiceData>;
    async fn start_service(&self, service_name: &str) -> Result<()>;
    async fn stop_service(&self, service_name: &str) -> Result<()>;
    async fn restart_service(&self, service_name: &str) -> Result<()>;
    async fn set_startup_type(&self, service_name: &str, startup_type: ServiceStartType) -> Result<()>;
}
