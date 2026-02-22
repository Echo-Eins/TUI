#[cfg(target_os = "windows")]
pub use crate::monitors::windows::disk::WindowsDiskMonitor as DiskMonitor;

#[cfg(target_os = "linux")]
pub use crate::monitors::linux::disk::LinuxDiskMonitor as DiskMonitor;
