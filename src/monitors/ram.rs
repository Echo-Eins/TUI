#[cfg(target_os = "windows")]
pub use crate::monitors::windows::ram::WindowsRamMonitor as RamMonitor;

#[cfg(target_os = "linux")]
pub use crate::monitors::linux::ram::LinuxRamMonitor as RamMonitor;
