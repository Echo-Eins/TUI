use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

// ======== CPU ========
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuData {
    pub name: String,
    pub overall_usage: f32,
    pub core_count: usize,
    pub thread_count: usize,
    pub core_usage: Vec<CoreUsage>,
    pub frequency: FrequencyInfo,
    pub power: PowerInfo,
    pub temperature: Option<f32>,
    pub top_processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreUsage {
    pub core_id: usize,
    pub usage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrequencyInfo {
    pub base_clock: f32,    // GHz
    pub avg_frequency: f32, // GHz
    pub max_frequency: f32, // GHz
    pub boost_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerInfo {
    pub current_power: f32, // Watts
    pub max_power: f32,     // Watts (TDP)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub threads: usize,
    pub memory: u64, // Bytes
}

// ======== GPU ========
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuData {
    pub name: String,
    pub gpu_index: u32,
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
    pub bus_id: String,
    pub cuda_version: String,
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

// ======== RAM ========
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RamData {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub cached: u64,
    pub free: u64,
    pub speed: String,
    pub type_name: String,

    // Memory Breakdown
    pub in_use: u64,
    pub standby: u64,
    pub modified: u64,

    // Committed Memory
    pub committed: u64,
    pub commit_limit: u64,
    pub commit_percent: f64,

    // Top Memory Consumers
    pub top_processes: Vec<ProcessMemoryInfo>,

    // Pagefile / Swap Information
    pub pagefiles: Vec<PagefileInfo>,
    pub total_pagefile_size: u64,
    pub total_pagefile_used: u64,

    // Zram Information (Linux)
    pub zram_devices: Vec<ZramDeviceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZramDeviceInfo {
    pub name: String,
    pub disksize: u64,
    pub orig_data_size: u64,
    pub compr_data_size: u64,
    pub mem_used_total: u64,
    pub compression_ratio: f64,
    pub algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessMemoryInfo {
    pub pid: u32,
    pub name: String,
    pub working_set: u64,
    pub private_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PagefileInfo {
    pub name: String,
    pub total_size: u64,
    pub current_usage: u64,
    pub peak_usage: u64,
    pub usage_percent: f64,
}

// ======== DISK ========
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskData {
    pub physical_disks: Vec<PhysicalDiskInfo>,
    pub logical_drives: Vec<DriveInfo>,
    pub io_stats: Vec<DiskIOStats>,
    pub process_activity: Vec<DiskProcessActivity>,
    pub io_history: Vec<DiskIOHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskIOStats {
    pub disk_number: u32,
    pub read_speed: f64,        // MB/s
    pub write_speed: f64,       // MB/s
    pub read_iops: f64,         // Operations per second
    pub write_iops: f64,        // Operations per second
    pub queue_depth: f64,       // Average queue length
    pub avg_response_time: f64, // Milliseconds
    pub active_time: f64,       // Percentage
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskProcessActivity {
    pub process_name: String,
    pub pid: u32,
    pub io_bytes_per_sec: f64, // Total I/O bytes per second
    pub read_bytes_per_sec: f64,
    pub write_bytes_per_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskIOHistory {
    pub disk_number: u32,
    pub read_history: VecDeque<f64>,  // Last 60 samples of read speed
    pub write_history: VecDeque<f64>, // Last 60 samples of write speed
    pub iops_history: VecDeque<f64>,  // Last 60 samples of total IOPS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalDiskInfo {
    pub disk_number: u32,
    pub friendly_name: String,
    #[serde(default)]
    pub device_path: String,
    pub model: String,
    pub media_type: String, // HDD, SSD, NVMe
    pub bus_type: String,   // SATA, NVMe, USB, etc.
    pub size: u64,
    #[serde(default)]
    pub filesystem_total: u64,
    #[serde(default)]
    pub filesystem_used: u64,
    #[serde(default)]
    pub filesystem_available: u64,
    pub health_status: String, // Healthy, Warning, Unhealthy
    pub operational_status: String,
    pub temperature: Option<f32>,
    pub write_cache_enabled: bool,

    // SMART data
    pub power_on_hours: Option<u64>,
    pub tbw: Option<u64>,        // Total Bytes Written (for SSD)
    pub wear_level: Option<f32>, // Wear leveling percentage

    // Associated logical drives
    pub partitions: Vec<String>, // Drive letters (C:, D:, etc.)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountPointInfo {
    pub path: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveInfo {
    pub letter: String,
    pub name: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub mount_points: Vec<String>,
    #[serde(default)]
    pub mount_details: Vec<MountPointInfo>,
    pub drive_type: String,
    pub file_system: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub disk_number: Option<u32>, // Link to physical disk
}

impl DriveInfo {
    pub fn stable_key(&self) -> String {
        self.uuid
            .as_ref()
            .filter(|uuid| !uuid.is_empty())
            .map(|uuid| format!("uuid:{uuid}"))
            .unwrap_or_else(|| format!("source:{}:{}", self.source, self.file_system))
    }
}

// ======== NETWORK ========
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkData {
    pub interfaces: Vec<NetworkInterface>,
    pub connections: Vec<NetworkConnection>,
    pub traffic_history: VecDeque<TrafficSample>,
    pub bandwidth_consumers: Vec<BandwidthConsumer>,
}

impl Default for NetworkData {
    fn default() -> Self {
        Self {
            interfaces: Vec::new(),
            connections: Vec::new(),
            traffic_history: VecDeque::with_capacity(60),
            bandwidth_consumers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub description: String,
    pub status: String,
    pub link_speed: String,
    pub mac_address: String,
    pub mtu: u32,
    pub duplex: String,

    // IP Configuration
    pub ipv4_address: String,
    pub ipv6_address: String,
    pub gateway: String,
    pub dns_servers: Vec<String>,

    // Statistics
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub download_speed: f64, // Mbps
    pub upload_speed: f64,   // Mbps
    pub peak_download: f64,
    pub peak_upload: f64,

    // Per-interface traffic history
    pub traffic_history: VecDeque<TrafficSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConnection {
    pub process_name: String,
    pub pid: u32,
    pub protocol: String,
    pub local_address: String,
    pub local_port: u16,
    pub remote_address: String,
    pub remote_port: u16,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficSample {
    pub timestamp: u64,
    pub download_mbps: f64,
    pub upload_mbps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandwidthConsumer {
    pub process_name: String,
    pub pid: u32,
    pub download_speed: f64, // Mbps
    pub upload_speed: f64,   // Mbps
    pub total_bytes_received: u64,
    pub total_bytes_sent: u64,
    pub estimated: bool,
}

// ======== PROCESSES ========
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessData {
    pub processes: Vec<ProcessEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory: u64,
    pub threads: usize,
    pub user: String,
    pub command_line: Option<String>,
    pub start_time: Option<String>,
    pub handle_count: u32,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
}

// ======== SERVICES ========
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceData {
    pub services: Vec<ServiceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub name: String,
    pub display_name: String,
    pub status: ServiceStatus,
    pub start_type: ServiceStartType,
    pub description: Option<String>,
    pub can_stop: bool,
    pub can_pause_and_continue: bool,
    pub dependent_services: Vec<String>,
    pub service_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Paused,
    StartPending,
    StopPending,
    ContinuePending,
    PausePending,
    Unknown,
}

impl ServiceStatus {
    pub fn as_str(&self) -> &str {
        match self {
            ServiceStatus::Running => "Running",
            ServiceStatus::Stopped => "Stopped",
            ServiceStatus::Paused => "Paused",
            ServiceStatus::StartPending => "Starting",
            ServiceStatus::StopPending => "Stopping",
            ServiceStatus::ContinuePending => "Continuing",
            ServiceStatus::PausePending => "Pausing",
            ServiceStatus::Unknown => "Unknown",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Running" => ServiceStatus::Running,
            "Stopped" => ServiceStatus::Stopped,
            "Paused" => ServiceStatus::Paused,
            "StartPending" => ServiceStatus::StartPending,
            "StopPending" => ServiceStatus::StopPending,
            "ContinuePending" => ServiceStatus::ContinuePending,
            "PausePending" => ServiceStatus::PausePending,
            _ => ServiceStatus::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStartType {
    Automatic,
    Manual,
    Disabled,
    AutomaticDelayedStart,
    Unknown,
}

impl ServiceStartType {
    pub fn as_str(&self) -> &str {
        match self {
            ServiceStartType::Automatic => "Automatic",
            ServiceStartType::Manual => "Manual",
            ServiceStartType::Disabled => "Disabled",
            ServiceStartType::AutomaticDelayedStart => "Auto (Delayed)",
            ServiceStartType::Unknown => "Unknown",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Automatic" => ServiceStartType::Automatic,
            "Manual" => ServiceStartType::Manual,
            "Disabled" => ServiceStartType::Disabled,
            "AutomaticDelayedStart" => ServiceStartType::AutomaticDelayedStart,
            _ => ServiceStartType::Unknown,
        }
    }
}
