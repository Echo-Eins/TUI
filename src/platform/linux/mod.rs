#![allow(dead_code)]

pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;
pub mod process;
pub mod service;
pub mod shell;

pub use cpu::CpuInfo;
pub use disk::{BlockDeviceInfo, BtrfsInfo, DiskInfo, ProcessIoSample, RawDiskStats, SmartData};
pub use memory::{MemoryHardwareInfo, MemoryInfo, ZramInfo};
pub use network::{
    NetworkConnectionInfo, NetworkInterface, NetworkInterfaceIpInfo, NetworkInterfaceStats,
    ProcessBandwidthInfo,
};
pub use process::ProcessInfo;
pub use service::LinuxServiceInfo;

/// Unified Linux system monitor.
/// Methods are split across submodules by domain (cpu, memory, disk, network, process).
pub struct LinuxSysMonitor;

impl LinuxSysMonitor {
    pub fn new() -> Self {
        Self
    }
}
