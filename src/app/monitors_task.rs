use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use crate::integrations::{OllamaClient, OllamaData, PowerShellExecutor};
use crate::monitors::*;

pub fn spawn_monitor_tasks(
    cpu_data: Arc<RwLock<Option<CpuData>>>,
    cpu_error: Arc<RwLock<Option<String>>>,
    gpu_data: Arc<RwLock<Option<GpuData>>>,
    gpu_error: Arc<RwLock<Option<String>>>,
    ram_data: Arc<RwLock<Option<RamData>>>,
    ram_error: Arc<RwLock<Option<String>>>,
    disk_data: Arc<RwLock<Option<DiskData>>>,
    disk_error: Arc<RwLock<Option<String>>>,
    network_data: Arc<RwLock<Option<NetworkData>>>,
    network_error: Arc<RwLock<Option<String>>>,
    process_data: Arc<RwLock<Option<ProcessData>>>,
    process_error: Arc<RwLock<Option<String>>>,
    service_data: Arc<RwLock<Option<ServiceData>>>,
    service_error: Arc<RwLock<Option<String>>>,
    ollama_data: Arc<RwLock<Option<OllamaData>>>,
    ollama_error: Arc<RwLock<Option<String>>>,
    ps_executable: String,
    timeout: u64,
    cache_ttl: u64,
    use_cache: bool,
) {
    let ps_status = PowerShellExecutor::check_environment(&ps_executable);
    let powershell_ready = ps_status.available && ps_status.missing_modules.is_empty();

    if !ps_status.available {
        log::warn!("PowerShell executable '{}' is not available", ps_executable);
    }

    if !ps_status.missing_modules.is_empty() {
        log::warn!(
            "PowerShell is missing required modules: {:?}",
            ps_status.missing_modules
        );
    }

    let ps_unavailable_reason = if !ps_status.available {
        Some("PowerShell executable is not reachable".to_string())
    } else if !ps_status.missing_modules.is_empty() {
        Some(format!(
            "Missing PowerShell modules: {}",
            ps_status.missing_modules.join(", ")
        ))
    } else {
        None
    };

    // CPU monitor task
    {
        let cpu_data = Arc::clone(&cpu_data);
        let cpu_error = Arc::clone(&cpu_error);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl, use_cache);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            if !ps_available {
                let message = unavailable_reason
                    .unwrap_or_else(|| "PowerShell is required for CPU monitor".to_string());
                log::warn!("CPU monitor running in degraded mode: {}", message);
                *cpu_error.write() = Some(message);
                loop {
                    sleep(Duration::from_millis(1000)).await;
                }
            }

            let monitor = match CpuMonitor::new(ps) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("Failed to start CPU monitor: {}", e);
                    *cpu_error.write() = Some(e.to_string());
                    return;
                }
            };
            loop {
                match monitor.collect_data().await {
                    Ok(data) => {
                        *cpu_data.write() = Some(data);
                        *cpu_error.write() = None;
                    }
                    Err(e) => {
                        log::error!("CPU monitor error: {}", e);
                        *cpu_error.write() = Some(e.to_string());
                    }
                }
                sleep(Duration::from_millis(1000)).await;
            }
        });
    }

    // GPU monitor task
    {
        let gpu_data = Arc::clone(&gpu_data);
        let gpu_error = Arc::clone(&gpu_error);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl, use_cache);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            if !ps_available {
                let message = unavailable_reason
                    .unwrap_or_else(|| "PowerShell is required for GPU monitor".to_string());
                log::warn!("GPU monitor running in degraded mode: {}", message);
                *gpu_error.write() = Some(message);
                loop {
                    sleep(Duration::from_millis(1000)).await;
                }
            }

            let monitor = match GpuMonitor::new(ps) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("Failed to start GPU monitor: {}", e);
                    *gpu_error.write() = Some(e.to_string());
                    return;
                }
            };
            loop {
                match monitor.collect_data().await {
                    Ok(data) => {
                        *gpu_data.write() = Some(data);
                        *gpu_error.write() = None;
                    }
                    Err(e) => {
                        log::error!("GPU monitor error: {}", e);
                        *gpu_error.write() = Some(e.to_string());
                    }
                }
                sleep(Duration::from_millis(1000)).await;
            }
        });
    }

    // RAM monitor task
    {
        let ram_data = Arc::clone(&ram_data);
        let ram_error = Arc::clone(&ram_error);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl, use_cache);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            if !ps_available {
                let message = unavailable_reason
                    .unwrap_or_else(|| "PowerShell is required for RAM monitor".to_string());
                log::warn!("RAM monitor running in degraded mode: {}", message);
                *ram_error.write() = Some(message);
                loop {
                    sleep(Duration::from_millis(1000)).await;
                }
            }

            let monitor = match RamMonitor::new(ps) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("Failed to start RAM monitor: {}", e);
                    *ram_error.write() = Some(e.to_string());
                    return;
                }
            };
            loop {
                match monitor.collect_data().await {
                    Ok(data) => {
                        *ram_data.write() = Some(data);
                        *ram_error.write() = None;
                    }
                    Err(e) => {
                        log::error!("RAM monitor error: {}", e);
                        *ram_error.write() = Some(e.to_string());
                    }
                }
                sleep(Duration::from_millis(1000)).await;
            }
        });
    }

    // Disk monitor task
    {
        let disk_data = Arc::clone(&disk_data);
        let disk_error = Arc::clone(&disk_error);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl, use_cache);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            if !ps_available {
                let message = unavailable_reason
                    .unwrap_or_else(|| "PowerShell is required for disk monitor".to_string());
                log::warn!("Disk monitor running in degraded mode: {}", message);
                *disk_error.write() = Some(message);
                loop {
                    sleep(Duration::from_millis(2000)).await;
                }
            }

            let monitor = match DiskMonitor::new(ps) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("Failed to start disk monitor: {}", e);
                    *disk_error.write() = Some(e.to_string());
                    return;
                }
            };
            loop {
                match monitor.collect_data().await {
                    Ok(data) => {
                        *disk_data.write() = Some(data);
                        *disk_error.write() = None;
                    }
                    Err(e) => {
                        log::error!("Disk monitor error: {}", e);
                        *disk_error.write() = Some(e.to_string());
                    }
                }
                sleep(Duration::from_millis(2000)).await;
            }
        });
    }

    // Network monitor task
    {
        let network_data = Arc::clone(&network_data);
        let network_error = Arc::clone(&network_error);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl, use_cache);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            if !ps_available {
                let message = unavailable_reason
                    .unwrap_or_else(|| "PowerShell is required for network monitor".to_string());
                log::warn!("Network monitor running in degraded mode: {}", message);
                *network_error.write() = Some(message);
                loop {
                    sleep(Duration::from_millis(1000)).await;
                }
            }

            let mut monitor = match NetworkMonitor::new(ps) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("Failed to start network monitor: {}", e);
                    *network_error.write() = Some(e.to_string());
                    return;
                }
            };
            let mut traffic_history = std::collections::VecDeque::with_capacity(60);

            loop {
                if let Ok(mut data) = monitor.collect_data().await {
                    // Update traffic history (60 seconds)
                    if !data.traffic_history.is_empty() {
                        for sample in data.traffic_history.iter() {
                            traffic_history.push_back(sample.clone());
                        }
                    }

                    // Keep only last 60 samples
                    while traffic_history.len() > 60 {
                        traffic_history.pop_front();
                    }

                    // Update data with accumulated history
                    data.traffic_history = traffic_history.clone();

                    *network_data.write() = Some(data);
                    *network_error.write() = None;
                } else {
                    *network_error.write() = Some("Failed to collect network data".to_string());
                }
                sleep(Duration::from_millis(1000)).await;
            }
        });
    }

    // Process monitor task
    {
        let process_data = Arc::clone(&process_data);
        let process_error = Arc::clone(&process_error);
        let ps = PowerShellExecutor::new(ps_executable.clone(), timeout, cache_ttl, use_cache);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            if !ps_available {
                let message = unavailable_reason
                    .unwrap_or_else(|| "PowerShell is required for process monitor".to_string());
                log::warn!("Process monitor running in degraded mode: {}", message);
                *process_error.write() = Some(message);
                loop {
                    sleep(Duration::from_millis(2000)).await;
                }
            }

            let monitor = match ProcessMonitor::new(ps) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("Failed to start process monitor: {}", e);
                    *process_error.write() = Some(e.to_string());
                    return;
                }
            };
            loop {
                match monitor.collect_data().await {
                    Ok(data) => {
                        *process_data.write() = Some(data);
                        *process_error.write() = None;
                    }
                    Err(e) => {
                        log::error!("Process monitor error: {}", e);
                        *process_error.write() = Some(e.to_string());
                    }
                }
                sleep(Duration::from_millis(2000)).await;
            }
        });
    }

    // Service monitor task
    {
        let service_data = Arc::clone(&service_data);
        let service_error = Arc::clone(&service_error);
        let ps = PowerShellExecutor::new(ps_executable, timeout, cache_ttl, use_cache);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            if !ps_available {
                let message = unavailable_reason
                    .unwrap_or_else(|| "PowerShell is required for service monitor".to_string());
                log::warn!("Service monitor running in degraded mode: {}", message);
                *service_error.write() = Some(message);
                loop {
                    sleep(Duration::from_millis(3000)).await;
                }
            }

            let monitor = match ServiceMonitor::new(ps) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("Failed to start service monitor: {}", e);
                    *service_error.write() = Some(e.to_string());
                    return;
                }
            };
            loop {
                match monitor.collect_data().await {
                    Ok(data) => {
                        *service_data.write() = Some(data);
                        *service_error.write() = None;
                    }
                    Err(e) => {
                        log::error!("Service monitor error: {}", e);
                        *service_error.write() = Some(e.to_string());
                    }
                }
                sleep(Duration::from_millis(3000)).await;
            }
        });
    }

    // Ollama monitor task
    {
        let ollama_data = Arc::clone(&ollama_data);
        let ollama_error = Arc::clone(&ollama_error);
        tokio::spawn(async move {
            let mut client = match OllamaClient::new(None) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to start Ollama monitor: {}", e);
                    *ollama_error.write() = Some(e.to_string());
                    return;
                }
            };
            loop {
                match client.collect_data().await {
                    Ok(data) => {
                        *ollama_data.write() = Some(data);
                        *ollama_error.write() = None;
                    }
                    Err(e) => {
                        log::error!("Ollama monitor error: {}", e);
                        *ollama_error.write() = Some(e.to_string());
                    }
                }
                sleep(Duration::from_millis(5000)).await;
            }
        });
    }
}
