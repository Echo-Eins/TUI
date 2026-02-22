pub mod cpu;
pub mod gpu;
pub mod ram;
pub mod disk;
pub mod disk_analyzer;
pub mod network;
pub mod processes;
pub mod services;
pub mod types;
pub mod traits;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

pub use cpu::CpuMonitor;
pub use gpu::GpuMonitor;
pub use ram::RamMonitor;
pub use disk::DiskMonitor;
pub use disk_analyzer::{DiskAnalyzerMonitor, DiskAnalyzerData, AnalyzedDrive};
pub use network::NetworkMonitor;
pub use processes::ProcessMonitor;
pub use services::ServiceMonitor;

pub use types::*;
pub use traits::*;
