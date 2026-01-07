use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use crate::app::Config;
use crate::integrations::{OllamaClient, OllamaData, PowerShellExecutor};
use crate::monitors::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct PsSettings {
    executable: String,
    timeout_seconds: u64,
    cache_ttl_seconds: u64,
    use_cache: bool,
}

fn refresh_duration(refresh_interval_ms: u64) -> Duration {
    let interval_ms = if refresh_interval_ms == 0 { 1000 } else { refresh_interval_ms };
    Duration::from_millis(interval_ms.max(100))
}

fn effective_cache_ttl_seconds(cache_ttl_seconds: u64, refresh_interval_ms: u64) -> u64 {
    if cache_ttl_seconds == 0 {
        return 0;
    }

    let interval_secs = (refresh_interval_ms + 999) / 1000;
    if interval_secs == 0 {
        return 0;
    }

    cache_ttl_seconds.min(interval_secs)
}

fn build_ps_settings(config: &Config, refresh_interval_ms: u64) -> PsSettings {
    let effective_cache_ttl = effective_cache_ttl_seconds(
        config.powershell.cache_ttl_seconds,
        refresh_interval_ms,
    );
    let effective_use_cache = config.powershell.use_cache && effective_cache_ttl > 0;

    PsSettings {
        executable: config.powershell.executable.clone(),
        timeout_seconds: config.powershell.timeout_seconds,
        cache_ttl_seconds: effective_cache_ttl,
        use_cache: effective_use_cache,
    }
}
pub fn spawn_monitor_tasks(
    config: Arc<RwLock<Config>>,
    cpu_data: Arc<RwLock<Option<CpuData>>>,
    cpu_error: Arc<RwLock<Option<String>>>,
    gpu_data: Arc<RwLock<Option<GpuData>>>,
    gpu_error: Arc<RwLock<Option<String>>>,
    ram_data: Arc<RwLock<Option<RamData>>>,
    ram_error: Arc<RwLock<Option<String>>>,
    disk_data: Arc<RwLock<Option<DiskData>>>,
    disk_error: Arc<RwLock<Option<String>>>,
    disk_analyzer_data: Arc<RwLock<Option<DiskAnalyzerData>>>,
    disk_analyzer_error: Arc<RwLock<Option<String>>>,
    network_data: Arc<RwLock<Option<NetworkData>>>,
    network_error: Arc<RwLock<Option<String>>>,
    process_data: Arc<RwLock<Option<ProcessData>>>,
    process_error: Arc<RwLock<Option<String>>>,
    service_data: Arc<RwLock<Option<ServiceData>>>,
    service_error: Arc<RwLock<Option<String>>>,
    ollama_data: Arc<RwLock<Option<OllamaData>>>,
    ollama_error: Arc<RwLock<Option<String>>>,
) {
    let config_snapshot = config.read().clone();
    let ps_executable = config_snapshot.powershell.executable.clone();
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
        let config = Arc::clone(&config);
        let cpu_data = Arc::clone(&cpu_data);
        let cpu_error = Arc::clone(&cpu_error);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            let mut monitor: Option<CpuMonitor> = None;
            let mut last_settings: Option<PsSettings> = None;
            let mut last_cache_ttl: Option<u64> = None;

            loop {
                let (enabled, refresh_interval_ms, settings, cache_ttl_config, use_cache_config) = {
                    let cfg = config.read();
                    (
                        cfg.monitors.cpu.enabled,
                        cfg.monitors.cpu.refresh_interval_ms,
                        build_ps_settings(&cfg, cfg.monitors.cpu.refresh_interval_ms),
                        cfg.powershell.cache_ttl_seconds,
                        cfg.powershell.use_cache,
                    )
                };

                if !enabled {
                    *cpu_data.write() = None;
                    *cpu_error.write() = Some("CPU monitor disabled in config".to_string());
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if !ps_available {
                    let message = unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "PowerShell is required for CPU monitor".to_string());
                    log::warn!("CPU monitor running in degraded mode: {}", message);
                    *cpu_error.write() = Some(message);
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if last_settings.as_ref() != Some(&settings) {
                    if use_cache_config && settings.cache_ttl_seconds < cache_ttl_config {
                        if last_cache_ttl != Some(settings.cache_ttl_seconds) {
                            log::info!(
                                "CPU monitor cache TTL clamped to {}s to match refresh interval",
                                settings.cache_ttl_seconds
                            );
                            last_cache_ttl = Some(settings.cache_ttl_seconds);
                        }
                    }

                    let ps = PowerShellExecutor::new(
                        settings.executable.clone(),
                        settings.timeout_seconds,
                        settings.cache_ttl_seconds,
                        settings.use_cache,
                    );
                    match CpuMonitor::new(ps) {
                        Ok(m) => {
                            monitor = Some(m);
                            last_settings = Some(settings);
                        }
                        Err(e) => {
                            log::error!("Failed to start CPU monitor: {}", e);
                            *cpu_error.write() = Some(e.to_string());
                            sleep(refresh_duration(refresh_interval_ms)).await;
                            continue;
                        }
                    }
                }

                if let Some(ref mut monitor) = monitor {
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
                }

                sleep(refresh_duration(refresh_interval_ms)).await;
            }
        });
    }

    // GPU monitor task
    {
        let config = Arc::clone(&config);
        let gpu_data = Arc::clone(&gpu_data);
        let gpu_error = Arc::clone(&gpu_error);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            let mut monitor: Option<GpuMonitor> = None;
            let mut last_settings: Option<PsSettings> = None;
            let mut last_cache_ttl: Option<u64> = None;

            loop {
                let (enabled, refresh_interval_ms, settings, cache_ttl_config, use_cache_config) = {
                    let cfg = config.read();
                    (
                        cfg.monitors.gpu.enabled,
                        cfg.monitors.gpu.refresh_interval_ms,
                        build_ps_settings(&cfg, cfg.monitors.gpu.refresh_interval_ms),
                        cfg.powershell.cache_ttl_seconds,
                        cfg.powershell.use_cache,
                    )
                };

                if !enabled {
                    *gpu_data.write() = None;
                    *gpu_error.write() = Some("GPU monitor disabled in config".to_string());
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if !ps_available {
                    let message = unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "PowerShell is required for GPU monitor".to_string());
                    log::warn!("GPU monitor running in degraded mode: {}", message);
                    *gpu_error.write() = Some(message);
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if last_settings.as_ref() != Some(&settings) {
                    if use_cache_config && settings.cache_ttl_seconds < cache_ttl_config {
                        if last_cache_ttl != Some(settings.cache_ttl_seconds) {
                            log::info!(
                                "GPU monitor cache TTL clamped to {}s to match refresh interval",
                                settings.cache_ttl_seconds
                            );
                            last_cache_ttl = Some(settings.cache_ttl_seconds);
                        }
                    }

                    let ps = PowerShellExecutor::new(
                        settings.executable.clone(),
                        settings.timeout_seconds,
                        settings.cache_ttl_seconds,
                        settings.use_cache,
                    );
                    match GpuMonitor::new(ps) {
                        Ok(m) => {
                            monitor = Some(m);
                            last_settings = Some(settings);
                        }
                        Err(e) => {
                            log::error!("Failed to start GPU monitor: {}", e);
                            *gpu_error.write() = Some(e.to_string());
                            sleep(refresh_duration(refresh_interval_ms)).await;
                            continue;
                        }
                    }
                }

                if let Some(ref mut monitor) = monitor {
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
                }

                sleep(refresh_duration(refresh_interval_ms)).await;
            }
        });
    }

    // RAM monitor task
    {
        let config = Arc::clone(&config);
        let ram_data = Arc::clone(&ram_data);
        let ram_error = Arc::clone(&ram_error);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            let mut monitor: Option<RamMonitor> = None;
            let mut last_settings: Option<PsSettings> = None;
            let mut last_cache_ttl: Option<u64> = None;

            loop {
                let (enabled, refresh_interval_ms, settings, cache_ttl_config, use_cache_config) = {
                    let cfg = config.read();
                    (
                        cfg.monitors.ram.enabled,
                        cfg.monitors.ram.refresh_interval_ms,
                        build_ps_settings(&cfg, cfg.monitors.ram.refresh_interval_ms),
                        cfg.powershell.cache_ttl_seconds,
                        cfg.powershell.use_cache,
                    )
                };

                if !enabled {
                    *ram_data.write() = None;
                    *ram_error.write() = Some("RAM monitor disabled in config".to_string());
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if !ps_available {
                    let message = unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "PowerShell is required for RAM monitor".to_string());
                    log::warn!("RAM monitor running in degraded mode: {}", message);
                    *ram_error.write() = Some(message);
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if last_settings.as_ref() != Some(&settings) {
                    if use_cache_config && settings.cache_ttl_seconds < cache_ttl_config {
                        if last_cache_ttl != Some(settings.cache_ttl_seconds) {
                            log::info!(
                                "RAM monitor cache TTL clamped to {}s to match refresh interval",
                                settings.cache_ttl_seconds
                            );
                            last_cache_ttl = Some(settings.cache_ttl_seconds);
                        }
                    }

                    let ps = PowerShellExecutor::new(
                        settings.executable.clone(),
                        settings.timeout_seconds,
                        settings.cache_ttl_seconds,
                        settings.use_cache,
                    );
                    match RamMonitor::new(ps) {
                        Ok(m) => {
                            monitor = Some(m);
                            last_settings = Some(settings);
                        }
                        Err(e) => {
                            log::error!("Failed to start RAM monitor: {}", e);
                            *ram_error.write() = Some(e.to_string());
                            sleep(refresh_duration(refresh_interval_ms)).await;
                            continue;
                        }
                    }
                }

                if let Some(ref mut monitor) = monitor {
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
                }

                sleep(refresh_duration(refresh_interval_ms)).await;
            }
        });
    }

    // Disk monitor task
    {
        let config = Arc::clone(&config);
        let disk_data = Arc::clone(&disk_data);
        let disk_error = Arc::clone(&disk_error);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            let mut monitor: Option<DiskMonitor> = None;
            let mut last_settings: Option<PsSettings> = None;
            let mut last_cache_ttl: Option<u64> = None;

            loop {
                let (enabled, refresh_interval_ms, settings, cache_ttl_config, use_cache_config) = {
                    let cfg = config.read();
                    (
                        cfg.monitors.disk.enabled,
                        cfg.monitors.disk.refresh_interval_ms,
                        build_ps_settings(&cfg, cfg.monitors.disk.refresh_interval_ms),
                        cfg.powershell.cache_ttl_seconds,
                        cfg.powershell.use_cache,
                    )
                };

                if !enabled {
                    *disk_data.write() = None;
                    *disk_error.write() = Some("Disk monitor disabled in config".to_string());
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if !ps_available {
                    let message = unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "PowerShell is required for disk monitor".to_string());
                    log::warn!("Disk monitor running in degraded mode: {}", message);
                    *disk_error.write() = Some(message);
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if last_settings.as_ref() != Some(&settings) {
                    if use_cache_config && settings.cache_ttl_seconds < cache_ttl_config {
                        if last_cache_ttl != Some(settings.cache_ttl_seconds) {
                            log::info!(
                                "Disk monitor cache TTL clamped to {}s to match refresh interval",
                                settings.cache_ttl_seconds
                            );
                            last_cache_ttl = Some(settings.cache_ttl_seconds);
                        }
                    }

                    let ps = PowerShellExecutor::new(
                        settings.executable.clone(),
                        settings.timeout_seconds,
                        settings.cache_ttl_seconds,
                        settings.use_cache,
                    );
                    match DiskMonitor::new(ps) {
                        Ok(m) => {
                            monitor = Some(m);
                            last_settings = Some(settings);
                        }
                        Err(e) => {
                            log::error!("Failed to start disk monitor: {}", e);
                            *disk_error.write() = Some(e.to_string());
                            sleep(refresh_duration(refresh_interval_ms)).await;
                            continue;
                        }
                    }
                }

                if let Some(ref mut monitor) = monitor {
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
                }

                sleep(refresh_duration(refresh_interval_ms)).await;
            }
        });
    }

    // Disk analyzer monitor task
    {
        let config = Arc::clone(&config);
        let disk_analyzer_data = Arc::clone(&disk_analyzer_data);
        let disk_analyzer_error = Arc::clone(&disk_analyzer_error);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            let mut monitor: Option<DiskAnalyzerMonitor> = None;
            let mut last_settings: Option<(PsSettings, String, usize, u64)> = None;
            let mut last_cache_ttl: Option<u64> = None;

            loop {
                let (
                    enabled,
                    refresh_interval_ms,
                    settings,
                    cache_ttl_config,
                    use_cache_config,
                    es_executable,
                    max_depth,
                ) = {
                    let cfg = config.read();
                    (
                        cfg.integrations.everything.enabled,
                        cfg.integrations.everything.refresh_interval_ms,
                        build_ps_settings(&cfg, cfg.integrations.everything.refresh_interval_ms),
                        cfg.powershell.cache_ttl_seconds,
                        cfg.powershell.use_cache,
                        cfg.integrations.everything.es_executable.clone(),
                        cfg.integrations.everything.max_depth,
                    )
                };

                if !enabled {
                    *disk_analyzer_data.write() = None;
                    *disk_analyzer_error.write() =
                        Some("Everything integration disabled in config".to_string());
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if !ps_available {
                    let message = unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "PowerShell is required for disk analyzer".to_string());
                    log::warn!("Disk analyzer running in degraded mode: {}", message);
                    *disk_analyzer_error.write() = Some(message);
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                let settings_key = (settings.clone(), es_executable.clone(), max_depth, refresh_interval_ms);
                if last_settings.as_ref() != Some(&settings_key) {
                    if use_cache_config && settings.cache_ttl_seconds < cache_ttl_config {
                        if last_cache_ttl != Some(settings.cache_ttl_seconds) {
                            log::info!(
                                "Disk analyzer cache TTL clamped to {}s to match refresh interval",
                                settings.cache_ttl_seconds
                            );
                            last_cache_ttl = Some(settings.cache_ttl_seconds);
                        }
                    }

                    let ps = PowerShellExecutor::new(
                        settings.executable.clone(),
                        settings.timeout_seconds,
                        settings.cache_ttl_seconds,
                        settings.use_cache,
                    );
                    match DiskAnalyzerMonitor::new(
                        ps,
                        es_executable.clone(),
                        max_depth,
                        settings.timeout_seconds,
                    ) {
                        Ok(m) => {
                            monitor = Some(m);
                            last_settings = Some(settings_key);
                        }
                        Err(e) => {
                            log::error!("Failed to start disk analyzer: {}", e);
                            *disk_analyzer_error.write() = Some(e.to_string());
                            sleep(refresh_duration(refresh_interval_ms)).await;
                            continue;
                        }
                    }
                }

                if let Some(ref mut monitor) = monitor {
                    match monitor.collect_data().await {
                        Ok(data) => {
                            *disk_analyzer_data.write() = Some(data);
                            *disk_analyzer_error.write() = None;
                        }
                        Err(e) => {
                            log::error!("Disk analyzer error: {}", e);
                            *disk_analyzer_error.write() = Some(e.to_string());
                        }
                    }
                }

                sleep(refresh_duration(refresh_interval_ms)).await;
            }
        });
    }

    // Network monitor task
    {
        let config = Arc::clone(&config);
        let network_data = Arc::clone(&network_data);
        let network_error = Arc::clone(&network_error);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            let mut monitor: Option<NetworkMonitor> = None;
            let mut last_settings: Option<PsSettings> = None;
            let mut last_cache_ttl: Option<u64> = None;
            let mut traffic_history = std::collections::VecDeque::with_capacity(60);

            loop {
                let (enabled, refresh_interval_ms, settings, cache_ttl_config, use_cache_config) = {
                    let cfg = config.read();
                    (
                        cfg.monitors.network.enabled,
                        cfg.monitors.network.refresh_interval_ms,
                        build_ps_settings(&cfg, cfg.monitors.network.refresh_interval_ms),
                        cfg.powershell.cache_ttl_seconds,
                        cfg.powershell.use_cache,
                    )
                };

                if !enabled {
                    traffic_history.clear();
                    *network_data.write() = None;
                    *network_error.write() = Some("Network monitor disabled in config".to_string());
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if !ps_available {
                    let message = unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "PowerShell is required for network monitor".to_string());
                    log::warn!("Network monitor running in degraded mode: {}", message);
                    *network_error.write() = Some(message);
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if last_settings.as_ref() != Some(&settings) {
                    if use_cache_config && settings.cache_ttl_seconds < cache_ttl_config {
                        if last_cache_ttl != Some(settings.cache_ttl_seconds) {
                            log::info!(
                                "Network monitor cache TTL clamped to {}s to match refresh interval",
                                settings.cache_ttl_seconds
                            );
                            last_cache_ttl = Some(settings.cache_ttl_seconds);
                        }
                    }

                    let ps = PowerShellExecutor::new(
                        settings.executable.clone(),
                        settings.timeout_seconds,
                        settings.cache_ttl_seconds,
                        settings.use_cache,
                    );
                    match NetworkMonitor::new(ps) {
                        Ok(m) => {
                            monitor = Some(m);
                            last_settings = Some(settings);
                        }
                        Err(e) => {
                            log::error!("Failed to start network monitor: {}", e);
                            *network_error.write() = Some(e.to_string());
                            sleep(refresh_duration(refresh_interval_ms)).await;
                            continue;
                        }
                    }
                }

                if let Some(monitor) = monitor.as_mut() {
                    if let Ok(mut data) = monitor.collect_data().await {
                        if !data.traffic_history.is_empty() {
                            for sample in data.traffic_history.iter() {
                                traffic_history.push_back(sample.clone());
                            }
                        }

                        while traffic_history.len() > 60 {
                            traffic_history.pop_front();
                        }

                        data.traffic_history = traffic_history.clone();

                        *network_data.write() = Some(data);
                        *network_error.write() = None;
                    } else {
                        *network_error.write() = Some("Failed to collect network data".to_string());
                    }
                }

                sleep(refresh_duration(refresh_interval_ms)).await;
            }
        });
    }

    // Process monitor task
    {
        let config = Arc::clone(&config);
        let process_data = Arc::clone(&process_data);
        let process_error = Arc::clone(&process_error);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            let mut monitor: Option<ProcessMonitor> = None;
            let mut last_settings: Option<PsSettings> = None;
            let mut last_cache_ttl: Option<u64> = None;

            loop {
                let (enabled, refresh_interval_ms, settings, cache_ttl_config, use_cache_config) = {
                    let cfg = config.read();
                    (
                        cfg.monitors.processes.enabled,
                        cfg.monitors.processes.refresh_interval_ms,
                        build_ps_settings(&cfg, cfg.monitors.processes.refresh_interval_ms),
                        cfg.powershell.cache_ttl_seconds,
                        cfg.powershell.use_cache,
                    )
                };

                if !enabled {
                    *process_data.write() = None;
                    *process_error.write() = Some("Process monitor disabled in config".to_string());
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if !ps_available {
                    let message = unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "PowerShell is required for process monitor".to_string());
                    log::warn!("Process monitor running in degraded mode: {}", message);
                    *process_error.write() = Some(message);
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if last_settings.as_ref() != Some(&settings) {
                    if use_cache_config && settings.cache_ttl_seconds < cache_ttl_config {
                        if last_cache_ttl != Some(settings.cache_ttl_seconds) {
                            log::info!(
                                "Process monitor cache TTL clamped to {}s to match refresh interval",
                                settings.cache_ttl_seconds
                            );
                            last_cache_ttl = Some(settings.cache_ttl_seconds);
                        }
                    }

                    let ps = PowerShellExecutor::new(
                        settings.executable.clone(),
                        settings.timeout_seconds,
                        settings.cache_ttl_seconds,
                        settings.use_cache,
                    );
                    match ProcessMonitor::new(ps) {
                        Ok(m) => {
                            monitor = Some(m);
                            last_settings = Some(settings);
                        }
                        Err(e) => {
                            log::error!("Failed to start process monitor: {}", e);
                            *process_error.write() = Some(e.to_string());
                            sleep(refresh_duration(refresh_interval_ms)).await;
                            continue;
                        }
                    }
                }

                if let Some(ref mut monitor) = monitor {
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
                }

                sleep(refresh_duration(refresh_interval_ms)).await;
            }
        });
    }

    // Service monitor task
    {
        let config = Arc::clone(&config);
        let service_data = Arc::clone(&service_data);
        let service_error = Arc::clone(&service_error);
        let ps_available = powershell_ready || cfg!(target_os = "linux");
        let unavailable_reason = ps_unavailable_reason.clone();
        tokio::spawn(async move {
            let mut monitor: Option<ServiceMonitor> = None;
            let mut last_settings: Option<PsSettings> = None;
            let mut last_cache_ttl: Option<u64> = None;

            loop {
                let (enabled, refresh_interval_ms, settings, cache_ttl_config, use_cache_config) = {
                    let cfg = config.read();
                    (
                        cfg.monitors.services.enabled,
                        cfg.monitors.services.refresh_interval_ms,
                        build_ps_settings(&cfg, cfg.monitors.services.refresh_interval_ms),
                        cfg.powershell.cache_ttl_seconds,
                        cfg.powershell.use_cache,
                    )
                };

                if !enabled {
                    *service_data.write() = None;
                    *service_error.write() = Some("Service monitor disabled in config".to_string());
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if !ps_available {
                    let message = unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "PowerShell is required for service monitor".to_string());
                    log::warn!("Service monitor running in degraded mode: {}", message);
                    *service_error.write() = Some(message);
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if last_settings.as_ref() != Some(&settings) {
                    if use_cache_config && settings.cache_ttl_seconds < cache_ttl_config {
                        if last_cache_ttl != Some(settings.cache_ttl_seconds) {
                            log::info!(
                                "Service monitor cache TTL clamped to {}s to match refresh interval",
                                settings.cache_ttl_seconds
                            );
                            last_cache_ttl = Some(settings.cache_ttl_seconds);
                        }
                    }

                    let ps = PowerShellExecutor::new(
                        settings.executable.clone(),
                        settings.timeout_seconds,
                        settings.cache_ttl_seconds,
                        settings.use_cache,
                    );
                    match ServiceMonitor::new(ps) {
                        Ok(m) => {
                            monitor = Some(m);
                            last_settings = Some(settings);
                        }
                        Err(e) => {
                            log::error!("Failed to start service monitor: {}", e);
                            *service_error.write() = Some(e.to_string());
                            sleep(refresh_duration(refresh_interval_ms)).await;
                            continue;
                        }
                    }
                }

                if let Some(ref mut monitor) = monitor {
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
                }

                sleep(refresh_duration(refresh_interval_ms)).await;
            }
        });
    }

    // Ollama monitor task
    {
        let config = Arc::clone(&config);
        let ollama_data = Arc::clone(&ollama_data);
        let ollama_error = Arc::clone(&ollama_error);
        tokio::spawn(async move {
            let mut client: Option<OllamaClient> = None;
            loop {
                let (enabled, refresh_interval_ms) = {
                    let cfg = config.read();
                    (
                        cfg.integrations.ollama.enabled,
                        cfg.integrations.ollama.refresh_interval_ms,
                    )
                };

                if !enabled {
                    client = None;
                    *ollama_data.write() = None;
                    *ollama_error.write() = Some("Ollama integration disabled in config".to_string());
                    sleep(refresh_duration(refresh_interval_ms)).await;
                    continue;
                }

                if client.is_none() {
                    match OllamaClient::new(None) {
                        Ok(c) => client = Some(c),
                        Err(e) => {
                            log::error!("Failed to start Ollama monitor: {}", e);
                            *ollama_error.write() = Some(e.to_string());
                            sleep(refresh_duration(refresh_interval_ms)).await;
                            continue;
                        }
                    }
                }

                if let Some(client) = client.as_mut() {
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
                }
                sleep(refresh_duration(refresh_interval_ms)).await;
            }
        });
    }
}
