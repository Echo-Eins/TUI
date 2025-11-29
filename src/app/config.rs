use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub tabs: TabsConfig,
    pub monitors: MonitorsConfig,
    pub integrations: IntegrationsConfig,
    pub ui: UiConfig,
    pub hotkeys: HotkeysConfig,
    pub powershell: PowerShellConfig,
    pub theme: ThemeConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GeneralConfig {
    pub app_name: String,
    pub refresh_rate_ms: u64,
    pub compact_mode: bool,
    pub theme: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TabsConfig {
    pub enabled: Vec<String>,
    pub default: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MonitorsConfig {
    pub cpu: CpuMonitorConfig,
    pub gpu: GpuMonitorConfig,
    pub ram: RamMonitorConfig,
    pub disk: DiskMonitorConfig,
    pub network: NetworkMonitorConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CpuMonitorConfig {
    pub enabled: bool,
    pub refresh_interval_ms: u64,
    pub show_per_core: bool,
    pub show_frequency: bool,
    pub show_temperature: bool,
    pub top_processes_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GpuMonitorConfig {
    pub enabled: bool,
    pub refresh_interval_ms: u64,
    pub use_nvml: bool,
    pub show_processes: bool,
    pub show_memory: bool,
    pub top_processes_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RamMonitorConfig {
    pub enabled: bool,
    pub refresh_interval_ms: u64,
    pub show_breakdown: bool,
    pub show_pagefile: bool,
    pub top_processes_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiskMonitorConfig {
    pub enabled: bool,
    pub refresh_interval_ms: u64,
    pub show_health: bool,
    pub show_temperature: bool,
    pub show_activity: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkMonitorConfig {
    pub enabled: bool,
    pub refresh_interval_ms: u64,
    pub show_graph: bool,
    pub graph_duration_seconds: u64,
    pub show_connections: bool,
    pub max_connections: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IntegrationsConfig {
    pub ollama: OllamaConfig,
    pub everything: EverythingConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OllamaConfig {
    pub enabled: bool,
    pub refresh_interval_ms: u64,
    pub command_timeout_seconds: u64,
    pub show_vram_usage: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EverythingConfig {
    pub enabled: bool,
    pub es_executable: String,
    pub max_depth: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiConfig {
    pub mouse_support: bool,
    pub tab_switch_key: String,
    pub compact_toggle_key: String,
    pub command_menu_key: String,
    pub quit_key: String,
    pub graphs: GraphConfig,
    pub command_history: CommandHistoryConfig,
    pub section_highlight: SectionHighlightConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GraphConfig {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommandHistoryConfig {
    pub max_entries: usize,
    pub circular_menu_radius: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SectionHighlightConfig {
    pub variant: String,
    pub normal_bracket: String,
    pub highlighted_bracket: String,
    pub normal_color: String,
    pub highlighted_color: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HotkeysConfig {
    pub cpu: String,
    pub gpu: String,
    pub ram: String,
    pub disk: String,
    pub network: String,
    pub ollama: String,
    pub processes: String,
    pub services: String,
    pub disk_analyzer: String,
    pub settings: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PowerShellConfig {
    pub executable: String,
    pub timeout_seconds: u64,
    pub use_cache: bool,
    pub cache_ttl_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThemeConfig {
    pub dark: DarkTheme,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DarkTheme {
    pub background: String,
    pub foreground: String,
    pub cpu_color: String,
    pub gpu_color: String,
    pub ram_color: String,
    pub disk_color: String,
    pub network_color: String,
    pub warning_color: String,
    pub error_color: String,
    pub success_color: String,
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read config file: {:?}", path.as_ref()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| "Failed to parse config file")?;

        Ok(config)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .with_context(|| "Failed to serialize config")?;

        fs::write(path.as_ref(), content)
            .with_context(|| format!("Failed to write config file: {:?}", path.as_ref()))?;

        Ok(())
    }
}

pub struct ConfigManager {
    // TODO: Implement hot reload with notify crate
}
