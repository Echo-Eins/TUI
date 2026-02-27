#![allow(dead_code)]

pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;
pub mod process;
pub mod services;
pub mod shell;

pub use cpu::CpuInfo;
pub use disk::{BlockDeviceInfo, BtrfsInfo, DiskInfo, ProcessIoSample, RawDiskStats, SmartData};
pub use memory::{MemoryHardwareInfo, MemoryInfo, ZramInfo};
pub use process::ProcessInfo;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone)]
pub(super) struct RaplDomainSample {
    pub energy_uj: u64,
    pub max_range_uj: u64,
}

#[derive(Debug, Clone)]
pub(super) struct RaplSnapshot {
    pub timestamp: Instant,
    pub domains: HashMap<String, RaplDomainSample>,
}

/// Unified Linux system monitor.
/// Methods are split across submodules by domain (cpu, memory, disk, network, process).
pub struct LinuxSysMonitor {
    pub(super) rapl_snapshot: Mutex<Option<RaplSnapshot>>,
}

impl LinuxSysMonitor {
    pub fn new() -> Self {
        Self {
            rapl_snapshot: Mutex::new(None),
        }
    }
}
