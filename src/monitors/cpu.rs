use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuData {
    pub name: String,
    pub overall_usage: f32,
    pub core_count: usize,
    pub thread_count: usize,
    pub core_usage: Vec<CoreUsage>,
    pub frequency: FrequencyInfo,
    pub power: PowerInfo,
    pub temperature: Option<f32>,
    pub top_processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreUsage {
    pub core_id: usize,
    pub usage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrequencyInfo {
    pub base_clock: f32,      // GHz
    pub avg_frequency: f32,   // GHz
    pub max_frequency: f32,   // GHz
    pub boost_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerInfo {
    pub current_power: f32,   // Watts
    pub max_power: f32,       // Watts (TDP)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub threads: usize,
    pub memory: u64,  // Bytes
}

pub struct CpuMonitor {
    // TODO: Add PowerShell executor
}

impl CpuMonitor {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    pub async fn collect_data(&self) -> Result<CpuData> {
        // TODO: Implement data collection via PowerShell
        // For now, return mock data
        Ok(CpuData {
            name: "AMD Ryzen 9 5950X 16-Core @ 3.40GHz".to_string(),
            overall_usage: 34.0,
            core_count: 16,
            thread_count: 32,
            core_usage: (0..16)
                .map(|i| CoreUsage {
                    core_id: i,
                    usage: (i as f32 * 3.0 + 25.0) % 65.0,
                })
                .collect(),
            frequency: FrequencyInfo {
                base_clock: 3.4,
                avg_frequency: 3.87,
                max_frequency: 4.45,
                boost_active: true,
            },
            power: PowerInfo {
                current_power: 65.0,
                max_power: 105.0,
            },
            temperature: Some(45.0),
            top_processes: vec![
                ProcessInfo {
                    pid: 4521,
                    name: "chrome.exe".to_string(),
                    cpu_usage: 12.4,
                    threads: 42,
                    memory: 1_200_000_000,
                },
                ProcessInfo {
                    pid: 8934,
                    name: "rust-analyzer.exe".to_string(),
                    cpu_usage: 8.7,
                    threads: 16,
                    memory: 842_000_000,
                },
            ],
        })
    }
}
