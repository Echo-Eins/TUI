#[cfg(target_os = "windows")]
pub use crate::monitors::windows::processes::WindowsProcessMonitor as ProcessMonitor;

#[cfg(target_os = "linux")]
pub use crate::monitors::linux::processes::LinuxProcessMonitor as ProcessMonitor;
