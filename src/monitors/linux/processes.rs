use crate::integrations::LinuxSysMonitor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::Instant;

pub struct LinuxProcessMonitor {
    linux_sys: LinuxSysMonitor,
    prev_ticks: Mutex<HashMap<u32, u64>>,
    prev_timestamp: Mutex<Option<Instant>>,
}

impl LinuxProcessMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            linux_sys: LinuxSysMonitor::new(),
            prev_ticks: Mutex::new(HashMap::new()),
            prev_timestamp: Mutex::new(None),
        })
    }
}

impl ProcessMonitorTrait for LinuxProcessMonitor {
    async fn collect_data(&self) -> Result<ProcessData> {
        let processes = self.linux_sys.get_processes()?;
        let now = Instant::now();

        let clock_ticks_per_sec = self.linux_sys.get_clock_ticks_per_second();
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get() as f64)
            .unwrap_or(1.0);

        let mut prev_ticks = self.prev_ticks.lock();
        let mut prev_ts = self.prev_timestamp.lock();

        let elapsed_secs = prev_ts
            .map(|t| now.saturating_duration_since(t).as_secs_f64())
            .unwrap_or(0.0);

        let mut next_ticks = HashMap::with_capacity(processes.len());
        let mut result: Vec<ProcessEntry> = processes
            .into_iter()
            .map(|p| {
                let cpu_usage = if elapsed_secs > 0.0 {
                    if let Some(&prev) = prev_ticks.get(&p.pid) {
                        let delta_ticks = p.cpu_ticks.saturating_sub(prev);
                        let delta_secs = delta_ticks as f64 / clock_ticks_per_sec as f64;
                        let usage = (delta_secs / elapsed_secs / num_cpus) * 100.0;
                        (usage.clamp(0.0, 100.0)) as f32
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                next_ticks.insert(p.pid, p.cpu_ticks);

                ProcessEntry {
                    pid: p.pid,
                    name: p.name,
                    cpu_usage,
                    memory: p.memory,
                    threads: p.threads,
                    user: p.user.unwrap_or_else(|| "Unknown".to_string()),
                    command_line: p.command_line,
                    start_time: p.start_time,
                    handle_count: p.handle_count,
                    io_read_bytes: p.io_read_bytes,
                    io_write_bytes: p.io_write_bytes,
                }
            })
            .collect();

        *prev_ticks = next_ticks;
        *prev_ts = Some(now);

        result.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        result.truncate(100);

        Ok(ProcessData { processes: result })
    }
}
