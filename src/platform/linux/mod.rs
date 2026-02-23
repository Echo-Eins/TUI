#![allow(dead_code)]

pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;
pub mod process;
pub mod shell;

pub use cpu::CpuInfo;
pub use disk::{BlockDeviceInfo, BtrfsInfo, DiskInfo, ProcessIoSample, RawDiskStats, SmartData};
pub use memory::{MemoryHardwareInfo, MemoryInfo, ZramInfo};
pub use network::NetworkInterface;
pub use process::ProcessInfo;

/// Unified Linux system monitor.
/// Methods are split across submodules by domain (cpu, memory, disk, network, process).
pub struct LinuxSysMonitor;

impl LinuxSysMonitor {
    pub fn new() -> Self {
        Self
    }
}
