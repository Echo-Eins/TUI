use anyhow::Result;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use crate::integrations::PowerShellExecutor;
use crate::monitors::*;

pub fn spawn_monitor_tasks(
    cpu_data: Arc<RwLock<Option<CpuData>>>,
    gpu_data: Arc<RwLock<Option<GpuData>>>,
    ram_data: Arc<RwLock<Option<RamData>>>,
    disk_data: Arc<RwLock<Option<DiskData>>>,
    network_data: Arc<RwLock<Option<NetworkData>>>,
    process_data: Arc<RwLock<Option<ProcessData>>>,
    ps_executable: String,
    timeout: u64,
    cache_ttl: u64,
) {
    // CPU monitor task
    {
        let cpu_data = Arc::clone(&cpu_data);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl);
        tokio::spawn(async move {
            let monitor = CpuMonitor::new(ps).unwrap();
            loop {
                if let Ok(data) = monitor.collect_data().await {
                    *cpu_data.write() = Some(data);
                }
                sleep(Duration::from_millis(1000)).await;
            }
        });
    }

    // GPU monitor task
    {
        let gpu_data = Arc::clone(&gpu_data);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl);
        tokio::spawn(async move {
            let monitor = GpuMonitor::new(ps).unwrap();
            loop {
                if let Ok(data) = monitor.collect_data().await {
                    *gpu_data.write() = Some(data);
                }
                sleep(Duration::from_millis(1000)).await;
            }
        });
    }

    // RAM monitor task
    {
        let ram_data = Arc::clone(&ram_data);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl);
        tokio::spawn(async move {
            let monitor = RamMonitor::new(ps).unwrap();
            loop {
                if let Ok(data) = monitor.collect_data().await {
                    *ram_data.write() = Some(data);
                }
                sleep(Duration::from_millis(1000)).await;
            }
        });
    }

    // Disk monitor task
    {
        let disk_data = Arc::clone(&disk_data);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl);
        tokio::spawn(async move {
            let monitor = DiskMonitor::new(ps).unwrap();
            loop {
                if let Ok(data) = monitor.collect_data().await {
                    *disk_data.write() = Some(data);
                }
                sleep(Duration::from_millis(2000)).await;
            }
        });
    }

    // Network monitor task
    {
        let network_data = Arc::clone(&network_data);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl);
        tokio::spawn(async move {
            let monitor = NetworkMonitor::new(ps).unwrap();
            loop {
                if let Ok(data) = monitor.collect_data().await {
                    *network_data.write() = Some(data);
                }
                sleep(Duration::from_millis(1000)).await;
            }
        });
    }

    // Process monitor task
    {
        let process_data = Arc::clone(&process_data);
        let ps = PowerShellExecutor::new(ps_executable, timeout, cache_ttl);
        tokio::spawn(async move {
            let monitor = ProcessMonitor::new(ps).unwrap();
            loop {
                if let Ok(data) = monitor.collect_data().await {
                    *process_data.write() = Some(data);
                }
                sleep(Duration::from_millis(2000)).await;
            }
        });
    }
}
