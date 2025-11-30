use chrono::Local;
use log::{error, info};
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use super::state::{MonitorState, MonitorStatus};
use crate::integrations::PowerShellExecutor;
use crate::monitors::*;

pub fn spawn_monitor_tasks(
    cpu_data: Arc<RwLock<MonitorState<CpuData>>>,
    gpu_data: Arc<RwLock<MonitorState<GpuData>>>,
    ram_data: Arc<RwLock<MonitorState<RamData>>>,
    disk_data: Arc<RwLock<MonitorState<DiskData>>>,
    network_data: Arc<RwLock<MonitorState<NetworkData>>>,
    process_data: Arc<RwLock<MonitorState<ProcessData>>>,
    powershell_config: crate::app::config::PowerShellConfig,
    monitors_config: crate::app::config::MonitorsConfig,
) {
    let cpu_config = monitors_config.cpu.clone();
    let gpu_config = monitors_config.gpu.clone();
    let ram_config = monitors_config.ram.clone();
    let disk_config = monitors_config.disk.clone();
    let network_config = monitors_config.network.clone();
    // CPU monitor task
    {
        let cpu_data = Arc::clone(&cpu_data);
        let cpu_config = cpu_config.clone();
        let ps = PowerShellExecutor::new(
            powershell_config.executable.clone(),
            powershell_config.timeout_seconds,
            powershell_config.cache_ttl_seconds,
            powershell_config.use_cache,
        );
        let refresh_interval = Duration::from_millis(cpu_config.refresh_interval_ms);
        tokio::spawn(async move {
            let mut init_backoff = Duration::from_secs(1);
            let mut collect_backoff = Duration::from_secs(1);
            loop {
                match CpuMonitor::new(ps.clone()) {
                    Ok(monitor) => {
                        info!("CPU monitor initialized successfully");

                        loop {
                            match monitor.collect_data().await {
                                Ok(data) => {
                                    let mut state = cpu_data.write();
                                    state.data = Some(data);
                                    state.status = MonitorStatus::Ready;
                                    state.last_updated = Some(Local::now());
                                }
                                Err(e) => {
                                    error!("CPU monitor collection failed: {}", e);
                                    {
                                        let mut state = cpu_data.write();
                                        state.status = MonitorStatus::Error(format!(
                                            "Data collection failed: {}",
                                            e
                                        ));
                                    }
                                    sleep(collect_backoff).await;
                                    collect_backoff =
                                        (collect_backoff * 2).min(Duration::from_secs(30));
                                    break;
                                }
                            }

                            sleep(refresh_interval).await;
                        }
                    }
                    Err(e) => {
                        error!("CPU monitor initialization failed: {}", e);
                        {
                            let mut state = cpu_data.write();
                            state.status =
                                MonitorStatus::Error(format!("Initialization failed: {}", e));
                        }
                        sleep(init_backoff).await;
                        init_backoff = (init_backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        });
    }

    // GPU monitor task
    {
        let gpu_data = Arc::clone(&gpu_data);
        let gpu_config = gpu_config.clone();
        let ps = PowerShellExecutor::new(
            powershell_config.executable.clone(),
            powershell_config.timeout_seconds,
            powershell_config.cache_ttl_seconds,
            powershell_config.use_cache,
        );
        let refresh_interval = Duration::from_millis(gpu_config.refresh_interval_ms);
        tokio::spawn(async move {
            let mut init_backoff = Duration::from_secs(1);
            let mut collect_backoff = Duration::from_secs(1);
            loop {
                match GpuMonitor::new(ps.clone(), gpu_config.use_nvml) {
                    Ok(monitor) => {
                        info!("GPU monitor initialized successfully");

                        loop {
                            match monitor.collect_data().await {
                                Ok(data) => {
                                    let mut state = gpu_data.write();
                                    state.data = Some(data);
                                    state.status = MonitorStatus::Ready;
                                    state.last_updated = Some(Local::now());
                                }
                                Err(e) => {
                                    error!("GPU monitor collection failed: {}", e);
                                    {
                                        let mut state = gpu_data.write();
                                        state.status = MonitorStatus::Error(format!(
                                            "Data collection failed: {}",
                                            e
                                        ));
                                    }
                                    sleep(collect_backoff).await;
                                    collect_backoff =
                                        (collect_backoff * 2).min(Duration::from_secs(30));
                                    break;
                                }
                            }

                            sleep(refresh_interval).await;
                        }
                    }
                    Err(e) => {
                        error!("GPU monitor initialization failed: {}", e);
                        {
                            let mut state = gpu_data.write();
                            state.status =
                                MonitorStatus::Error(format!("Initialization failed: {}", e));
                        }
                        sleep(init_backoff).await;
                        init_backoff = (init_backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        });
    }

    // RAM monitor task
    {
        let ram_data = Arc::clone(&ram_data);
        let ram_config = ram_config.clone();
        let ps = PowerShellExecutor::new(
            powershell_config.executable.clone(),
            powershell_config.timeout_seconds,
            powershell_config.cache_ttl_seconds,
            powershell_config.use_cache,
        );
        let refresh_interval = Duration::from_millis(ram_config.refresh_interval_ms);
        tokio::spawn(async move {
            let mut init_backoff = Duration::from_secs(1);
            let mut collect_backoff = Duration::from_secs(1);
            loop {
                match RamMonitor::new(ps.clone()) {
                    Ok(monitor) => {
                        info!("RAM monitor initialized successfully");

                        loop {
                            match monitor.collect_data().await {
                                Ok(data) => {
                                    let mut state = ram_data.write();
                                    state.data = Some(data);
                                    state.status = MonitorStatus::Ready;
                                    state.last_updated = Some(Local::now());
                                }
                                Err(e) => {
                                    error!("RAM monitor collection failed: {}", e);
                                    {
                                        let mut state = ram_data.write();
                                        state.status = MonitorStatus::Error(format!(
                                            "Data collection failed: {}",
                                            e
                                        ));
                                    }
                                    sleep(collect_backoff).await;
                                    collect_backoff =
                                        (collect_backoff * 2).min(Duration::from_secs(30));
                                    break;
                                }
                            }

                            sleep(refresh_interval).await;
                        }
                    }
                    Err(e) => {
                        error!("RAM monitor initialization failed: {}", e);
                        {
                            let mut state = ram_data.write();
                            state.status =
                                MonitorStatus::Error(format!("Initialization failed: {}", e));
                        }
                        sleep(init_backoff).await;
                        init_backoff = (init_backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        });
    }

    // Disk monitor task
    {
        let disk_data = Arc::clone(&disk_data);
        let disk_config = disk_config.clone();
        let ps = PowerShellExecutor::new(
            powershell_config.executable.clone(),
            powershell_config.timeout_seconds,
            powershell_config.cache_ttl_seconds,
            powershell_config.use_cache,
        );
        let refresh_interval = Duration::from_millis(disk_config.refresh_interval_ms);
        tokio::spawn(async move {
            let mut init_backoff = Duration::from_secs(1);
            let mut collect_backoff = Duration::from_secs(1);
            loop {
                match DiskMonitor::new(ps.clone()) {
                    Ok(monitor) => {
                        info!("Disk monitor initialized successfully");

                        loop {
                            match monitor.collect_data().await {
                                Ok(data) => {
                                    let mut state = disk_data.write();
                                    state.data = Some(data);
                                    state.status = MonitorStatus::Ready;
                                    state.last_updated = Some(Local::now());
                                }
                                Err(e) => {
                                    error!("Disk monitor collection failed: {}", e);
                                    {
                                        let mut state = disk_data.write();
                                        state.status = MonitorStatus::Error(format!(
                                            "Data collection failed: {}",
                                            e
                                        ));
                                    }
                                    sleep(collect_backoff).await;
                                    collect_backoff =
                                        (collect_backoff * 2).min(Duration::from_secs(30));
                                    break;
                                }
                            }

                            sleep(refresh_interval).await;
                        }
                    }
                    Err(e) => {
                        error!("Disk monitor initialization failed: {}", e);
                        {
                            let mut state = disk_data.write();
                            state.status =
                                MonitorStatus::Error(format!("Initialization failed: {}", e));
                        }
                        sleep(init_backoff).await;
                        init_backoff = (init_backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        });
    }

    // Network monitor task
    {
        let network_data = Arc::clone(&network_data);
        let network_config = network_config.clone();
        let ps = PowerShellExecutor::new(
            powershell_config.executable.clone(),
            powershell_config.timeout_seconds,
            powershell_config.cache_ttl_seconds,
            powershell_config.use_cache,
        );
        let refresh_interval = Duration::from_millis(network_config.refresh_interval_ms);
        tokio::spawn(async move {
            let mut init_backoff = Duration::from_secs(1);
            let mut collect_backoff = Duration::from_secs(1);
            loop {
                match NetworkMonitor::new(ps.clone()) {
                    Ok(mut monitor) => {
                        info!("Network monitor initialized successfully");
                        let mut traffic_history = std::collections::VecDeque::with_capacity(60);

                        loop {
                            match monitor.collect_data().await {
                                Ok(mut data) => {
                                    if !data.traffic_history.is_empty() {
                                        for sample in data.traffic_history.iter() {
                                            traffic_history.push_back(sample.clone());
                                        }
                                    }

                                    while traffic_history.len() > 60 {
                                        traffic_history.pop_front();
                                    }

                                    data.traffic_history = traffic_history.clone();

                                    let mut state = network_data.write();
                                    state.data = Some(data);
                                    state.status = MonitorStatus::Ready;
                                    state.last_updated = Some(Local::now());
                                }
                                Err(e) => {
                                    error!("Network monitor collection failed: {}", e);
                                    {
                                        let mut state = network_data.write();
                                        state.status = MonitorStatus::Error(format!(
                                            "Data collection failed: {}",
                                            e
                                        ));
                                    }
                                    sleep(collect_backoff).await;
                                    collect_backoff =
                                        (collect_backoff * 2).min(Duration::from_secs(30));
                                    break;
                                }
                            }

                            sleep(refresh_interval).await;
                        }
                    }
                    Err(e) => {
                        error!("Network monitor initialization failed: {}", e);
                        {
                            let mut state = network_data.write();
                            state.status =
                                MonitorStatus::Error(format!("Initialization failed: {}", e));
                        }
                        sleep(init_backoff).await;
                        init_backoff = (init_backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        });
    }

    // Process monitor task
    {
        let process_data = Arc::clone(&process_data);
        let ps = PowerShellExecutor::new(
            powershell_config.executable.clone(),
            powershell_config.timeout_seconds,
            powershell_config.cache_ttl_seconds,
            powershell_config.use_cache,
        );
        tokio::spawn(async move {
            let mut init_backoff = Duration::from_secs(1);
            let mut collect_backoff = Duration::from_secs(1);
            loop {
                match ProcessMonitor::new(ps.clone()) {
                    Ok(monitor) => {
                        info!("Process monitor initialized successfully");

                        loop {
                            match monitor.collect_data().await {
                                Ok(data) => {
                                    let mut state = process_data.write();
                                    state.data = Some(data);
                                    state.status = MonitorStatus::Ready;
                                    state.last_updated = Some(Local::now());
                                }
                                Err(e) => {
                                    error!("Process monitor collection failed: {}", e);
                                    {
                                        let mut state = process_data.write();
                                        state.status = MonitorStatus::Error(format!(
                                            "Data collection failed: {}",
                                            e
                                        ));
                                    }
                                    sleep(collect_backoff).await;
                                    collect_backoff =
                                        (collect_backoff * 2).min(Duration::from_secs(30));
                                    break;
                                }
                            }

                            sleep(Duration::from_millis(2000)).await;
                        }
                    }
                    Err(e) => {
                        error!("Process monitor initialization failed: {}", e);
                        {
                            let mut state = process_data.write();
                            state.status =
                                MonitorStatus::Error(format!("Initialization failed: {}", e));
                        }
                        sleep(init_backoff).await;
                        init_backoff = (init_backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        });
    }
}
