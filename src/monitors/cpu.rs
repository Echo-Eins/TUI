#[cfg(target_os = "windows")]
pub use crate::monitors::windows::cpu::WindowsCpuMonitor as CpuMonitor;

#[cfg(target_os = "linux")]
pub use crate::monitors::linux::cpu::LinuxCpuMonitor as CpuMonitor;
