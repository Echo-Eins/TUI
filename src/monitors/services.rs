#[cfg(target_os = "windows")]
pub use crate::monitors::windows::services::WindowsServiceMonitor as ServiceMonitor;

#[cfg(target_os = "linux")]
pub use crate::monitors::linux::services::LinuxServiceMonitor as ServiceMonitor;
