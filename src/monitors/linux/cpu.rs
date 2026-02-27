use crate::integrations::LinuxSysMonitor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::Instant;

pub struct LinuxCpuMonitor {
    linux_sys: LinuxSysMonitor,
    linux_prev_ticks: Mutex<HashMap<u32, u64>>,
    linux_prev_timestamp: Mutex<Option<Instant>>,
}

impl LinuxCpuMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            linux_sys: LinuxSysMonitor::new(),
            linux_prev_ticks: Mutex::new(HashMap::new()),
            linux_prev_timestamp: Mutex::new(None),
        })
    }

    fn get_linux_processes(&self) -> Result<Vec<ProcessInfo>> {
        let processes = self.linux_sys.get_processes()?;
        let now = Instant::now();

        let clock_ticks_per_sec = self.linux_sys.get_clock_ticks_per_second();
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get() as f64)
            .unwrap_or(1.0);

        let mut prev_ticks = self.linux_prev_ticks.lock();
        let mut prev_ts = self.linux_prev_timestamp.lock();

        let elapsed_secs = prev_ts
            .map(|t| now.saturating_duration_since(t).as_secs_f64())
            .unwrap_or(0.0);

        let mut result: Vec<ProcessInfo> = processes
            .iter()
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

                ProcessInfo {
                    pid: p.pid,
                    name: p.name.clone(),
                    cpu_usage,
                    threads: p.threads,
                    memory: p.memory,
                }
            })
            .collect();

        // Update previous ticks
        prev_ticks.clear();
        for p in &processes {
            prev_ticks.insert(p.pid, p.cpu_ticks);
        }
        *prev_ts = Some(now);

        // Sort by CPU usage descending
        result.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(result)
    }
}

impl CpuMonitorTrait for LinuxCpuMonitor {
    async fn collect_data(&self) -> Result<CpuData> {
        let cpu_info = self.linux_sys.get_cpu_info()?;
        let overall_usage = self.linux_sys.get_cpu_usage()?;
        let core_usage_values = self.linux_sys.get_per_core_usage()?;

        let core_usage: Vec<CoreUsage> = core_usage_values
            .iter()
            .enumerate()
            .map(|(i, &usage)| CoreUsage { core_id: i, usage })
            .collect();

        // Get per-core frequencies for avg calculation
        let per_core_freqs = self.linux_sys.get_per_core_frequencies();
        let avg_freq_mhz = if !per_core_freqs.is_empty() {
            per_core_freqs.iter().sum::<f32>() / per_core_freqs.len() as f32
        } else {
            cpu_info.current_frequency_mhz
        };

        // Determine boost state from sysfs (accurate), with frequency-based fallback
        let boost_active = match self.linux_sys.is_boost_enabled() {
            Some(enabled) => enabled,
            None => avg_freq_mhz > cpu_info.base_frequency_mhz * 1.05,
        };

        let frequency = FrequencyInfo {
            base_clock: cpu_info.base_frequency_mhz / 1000.0,
            avg_frequency: avg_freq_mhz / 1000.0,
            max_frequency: cpu_info.max_frequency_mhz / 1000.0,
            boost_active,
        };

        // Get real temperature
        let temperature = self.linux_sys.get_cpu_temperature();

        // Prefer measured package power from RAPL deltas; fallback to usage-based estimate.
        let (current_power, max_power) = match self.linux_sys.get_cpu_power() {
            Some((measured, tdp)) if measured > 0.0 => (measured, tdp),
            Some((_, tdp)) => ((overall_usage / 100.0) * tdp, tdp),
            None => {
                let tdp = 65.0;
                ((overall_usage / 100.0) * tdp, tdp)
            }
        };

        // Get all processes with CPU usage
        let top_processes = self.get_linux_processes()?;

        Ok(CpuData {
            name: cpu_info.name,
            overall_usage,
            core_count: cpu_info.core_count,
            thread_count: cpu_info.thread_count,
            core_usage,
            frequency,
            power: PowerInfo {
                current_power,
                max_power,
            },
            temperature,
            top_processes,
        })
    }
}
