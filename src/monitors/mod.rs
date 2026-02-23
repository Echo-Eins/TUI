pub mod cpu;
pub mod disk;
pub mod disk_analyzer;
pub mod gpu;
pub mod network;
pub mod processes;
pub mod ram;
pub mod services;
pub mod traits;
pub mod types;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

pub use cpu::CpuMonitor;
pub use disk::DiskMonitor;
pub use disk_analyzer::{AnalyzedDrive, DiskAnalyzerData, DiskAnalyzerMonitor};
pub use gpu::GpuMonitor;
pub use network::NetworkMonitor;
pub use processes::ProcessMonitor;
pub use ram::RamMonitor;
pub use services::ServiceMonitor;

pub use traits::*;
pub use types::*;
