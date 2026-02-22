#[cfg(target_os = "windows")]
pub use crate::monitors::windows::gpu::WindowsGpuMonitor as GpuMonitor;

#[cfg(target_os = "linux")]
pub use crate::monitors::linux::gpu::LinuxGpuMonitor as GpuMonitor;