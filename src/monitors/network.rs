#[cfg(target_os = "windows")]
pub use crate::monitors::windows::network::WindowsNetworkMonitor as NetworkMonitor;

#[cfg(target_os = "linux")]
pub use crate::monitors::linux::network::LinuxNetworkMonitor as NetworkMonitor;
