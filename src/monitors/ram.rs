use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::{PowerShellExecutor, LinuxSysMonitor};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RamData {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub cached: u64,
    pub free: u64,
    pub speed: String,
    pub type_name: String,

    // Memory Breakdown
    pub in_use: u64,
    pub standby: u64,
    pub modified: u64,

    // Committed Memory
    pub committed: u64,
    pub commit_limit: u64,
    pub commit_percent: f64,

    // Top Memory Consumers
    pub top_processes: Vec<ProcessMemoryInfo>,

    // Pagefile Information
    pub pagefiles: Vec<PagefileInfo>,
    pub total_pagefile_size: u64,
    pub total_pagefile_used: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessMemoryInfo {
    pub pid: u32,
    pub name: String,
    pub working_set: u64,
    pub private_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PagefileInfo {
    pub name: String,
    pub total_size: u64,
    pub current_usage: u64,
    pub peak_usage: u64,
    pub usage_percent: f64,
}

pub struct RamMonitor {
    ps: PowerShellExecutor,
    #[allow(dead_code)]
    linux_sys: LinuxSysMonitor,
}

const MEMORY_INFO_SCRIPT: &str = r#"
    try {
        $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop |
            Select-Object TotalVisibleMemorySize, FreePhysicalMemory
        if ($os) {
            $os | ConvertTo-Json
        } else {
            [PSCustomObject]@{
                TotalVisibleMemorySize = 0
                FreePhysicalMemory = 0
            } | ConvertTo-Json
        }
    } catch {
        [PSCustomObject]@{
            TotalVisibleMemorySize = 0
            FreePhysicalMemory = 0
        } | ConvertTo-Json
    }
"#;

const PHYSICAL_MEMORY_SCRIPT: &str = r#"
    try {
        $modules = Get-CimInstance Win32_PhysicalMemory -ErrorAction Stop
        if (-not $modules) {
            [PSCustomObject]@{ Speed = "Unknown"; MemoryType = "Unknown"; Modules = @() } | ConvertTo-Json
            return
        }

        $list = foreach ($mem in $modules) {
            $memType = switch ([int]$mem.SMBIOSMemoryType) {
                20 { "DDR" }
                21 { "DDR2" }
                24 { "DDR3" }
                26 { "DDR4" }
                27 { "LPDDR" }
                28 { "LPDDR2" }
                29 { "LPDDR3" }
                30 { "LPDDR4" }
                34 { "DDR5" }
                35 { "LPDDR5" }
                default { $null }
            }

            $formFactor = switch ([int]$mem.FormFactor) {
                12 { "SODIMM" }
                8 { "DIMM" }
                default { $null }
            }

            if (-not $memType) {
                $memType = switch ([int]$mem.MemoryType) {
                    20 { "DDR" }
                    21 { "DDR2" }
                    24 { "DDR3" }
                    26 { "DDR4" }
                    34 { "DDR5" }
                    default { "Unknown" }
                }
            }

            if ($formFactor -and $memType -and $memType -ne "Unknown") {
                $memType = "$formFactor $memType"
            }

            $speed = $null
            if ($mem.ConfiguredClockSpeed) {
                $speed = [uint32]$mem.ConfiguredClockSpeed
            } elseif ($mem.Speed) {
                $speed = [uint32]$mem.Speed
            }

            [PSCustomObject]@{
                Slot = $mem.DeviceLocator
                Manufacturer = ($mem.Manufacturer -as [string]).Trim()
                PartNumber = ($mem.PartNumber -as [string]).Trim()
                Capacity = [uint64]$mem.Capacity
                Speed = $speed
                MemoryType = $memType
            }
        }

        $types = $list | ForEach-Object { $_.MemoryType } | Where-Object { $_ -and $_ -ne 'Unknown' } | Sort-Object -Unique
        $typeSummary = if ($types.Count -eq 0) { "Unknown" } elseif ($types.Count -eq 1) { $types[0] } else { "Mixed (" + ($types -join "/") + ")" }

        $speeds = $list | ForEach-Object { $_.Speed } | Where-Object { $_ -ne $null } | Sort-Object -Unique
        $speedSummary = if ($speeds.Count -eq 0) { "Unknown" } elseif ($speeds.Count -eq 1) { "$($speeds[0]) MHz" } else { "$($speeds[0])-$($speeds[-1]) MHz" }

        [PSCustomObject]@{
            Speed = $speedSummary
            MemoryType = $typeSummary
            Modules = $list
        } | ConvertTo-Json -Depth 4
    } catch {
        [PSCustomObject]@{ Speed = "Unknown"; MemoryType = "Unknown"; Modules = @() } | ConvertTo-Json
    }
"#;

const DETAILED_MEMORY_SCRIPT: &str = r#"
    $counters = @(
        '\Memory\Available Bytes',
        '\Memory\Cache Bytes',
        '\Memory\Standby Cache Normal Priority Bytes',
        '\Memory\Standby Cache Reserve Bytes',
        '\Memory\Standby Cache Core Bytes',
        '\Memory\Free & Zero Page List Bytes',
        '\Memory\Modified Page List Bytes'
    )

    $available = 0
    $cached = 0
    $standbyNormal = 0
    $standbyReserve = 0
    $standbyCore = 0
    $free = 0
    $modified = 0

    $os = Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue
    $total = if ($os) { $os.TotalVisibleMemorySize * 1024 } else { 0 }

    try {
        $perfData = Get-Counter -Counter $counters -ErrorAction Stop

        $available = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Available Bytes*'}).CookedValue
        $cached = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Cache Bytes*'}).CookedValue
        $standbyNormal = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Standby Cache Normal*'}).CookedValue
        $standbyReserve = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Standby Cache Reserve*'}).CookedValue
        $standbyCore = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Standby Cache Core*'}).CookedValue
        $free = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Free && Zero*'}).CookedValue
        $modified = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Modified Page*'}).CookedValue
    } catch {
    }

    if ($available -eq 0 -and $os) {
        $available = $os.FreePhysicalMemory * 1024
    }
    if ($free -eq 0 -and $os) {
        $free = $os.FreePhysicalMemory * 1024
    }

    $standby = $standbyNormal + $standbyReserve + $standbyCore
    $inUse = if ($total -ge $available) { $total - $available } else { 0 }

    [PSCustomObject]@{
        InUse = [uint64]$inUse
        Available = [uint64]$available
        Cached = [uint64]$cached
        Standby = [uint64]$standby
        Free = [uint64]$free
        Modified = [uint64]$modified
    } | ConvertTo-Json
"#;

const COMMITTED_MEMORY_SCRIPT: &str = r#"
    $counters = @(
        '\Memory\Committed Bytes',
        '\Memory\Commit Limit'
    )

    $committed = 0
    $commitLimit = 0
    $commitPercent = 0

    try {
        $perfData = Get-Counter -Counter $counters -ErrorAction Stop

        $committed = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Committed Bytes*'}).CookedValue
        $commitLimit = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Commit Limit*'}).CookedValue
        $commitPercent = if ($commitLimit -gt 0) { ($committed / $commitLimit) * 100 } else { 0 }
    } catch {
        $os = Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue
        $pageFile = Get-CimInstance Win32_PageFileUsage -ErrorAction SilentlyContinue | Select-Object -First 1

        if ($os) {
            $committed = ($os.TotalVisibleMemorySize - $os.FreePhysicalMemory) * 1024
            $commitLimit = ($os.TotalVisibleMemorySize * 1024)
            if ($pageFile) {
                $commitLimit = $commitLimit + ($pageFile.AllocatedBaseSize * 1024 * 1024)
            }
            $commitPercent = if ($commitLimit -gt 0) { ($committed / $commitLimit) * 100 } else { 0 }
        }
    }

    [PSCustomObject]@{
        Committed = [uint64]$committed
        CommitLimit = [uint64]$commitLimit
        CommitPercent = [double]$commitPercent
    } | ConvertTo-Json
"#;

const TOP_PROCESSES_SCRIPT: &str = r#"
    try {
        Get-Process |
            Sort-Object WorkingSet64 -Descending |
            Select-Object -First 10 |
            ForEach-Object {
                [PSCustomObject]@{
                    Pid = $_.Id
                    Name = $_.ProcessName
                    WorkingSet = [uint64]$_.WorkingSet64
                    PrivateBytes = [uint64]$_.PrivateMemorySize64
                }
            } | ConvertTo-Json
    } catch {
        "[]"
    }
"#;

const PAGEFILE_SCRIPT: &str = r#"
    try {
        $pagefiles = Get-CimInstance Win32_PageFileUsage -ErrorAction Stop

        if ($pagefiles) {
            $result = @()
            foreach ($pf in $pagefiles) {
                $totalSize = [uint64]($pf.AllocatedBaseSize * 1024 * 1024)
                $currentUsage = [uint64]($pf.CurrentUsage * 1024 * 1024)
                $peakUsage = [uint64]($pf.PeakUsage * 1024 * 1024)
                $usagePercent = if ($totalSize -gt 0) { ($currentUsage / $totalSize) * 100 } else { 0 }

                $result += [PSCustomObject]@{
                    Name = $pf.Name
                    TotalSize = $totalSize
                    CurrentUsage = $currentUsage
                    PeakUsage = $peakUsage
                    UsagePercent = [double]$usagePercent
                }
            }
            $result | ConvertTo-Json
        } else {
            "[]"
        }
    } catch {
        "[]"
    }
"#;

impl RamMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            linux_sys: LinuxSysMonitor::new(),
        })
    }

    pub async fn collect_data(&self) -> Result<RamData> {
        #[cfg(target_os = "linux")]
        {
            return self.collect_data_linux().await;
        }

        #[cfg(not(target_os = "linux"))]
        {
            return self.collect_data_windows().await;
        }
    }

    #[allow(dead_code)]
    async fn collect_data_linux(&self) -> Result<RamData> {
        let mem_info = self.linux_sys.get_memory_info()?;

        Ok(RamData {
            total: mem_info.total,
            used: mem_info.used,
            available: mem_info.available,
            cached: mem_info.cached,
            free: mem_info.free,
            speed: String::from("Unknown"),
            type_name: String::from("DDR4"),
            in_use: mem_info.used,
            standby: 0,
            modified: 0,
            committed: mem_info.used,
            commit_limit: mem_info.total + mem_info.swap_total,
            commit_percent: (mem_info.used as f64 / mem_info.total as f64) * 100.0,
            top_processes: Vec::new(),
            pagefiles: Vec::new(),
            total_pagefile_size: mem_info.swap_total,
            total_pagefile_used: mem_info.swap_used,
        })
    }

    async fn collect_data_windows(&self) -> Result<RamData> {
        let outputs = self
            .ps
            .execute_batch(&[
                MEMORY_INFO_SCRIPT,
                PHYSICAL_MEMORY_SCRIPT,
                DETAILED_MEMORY_SCRIPT,
                COMMITTED_MEMORY_SCRIPT,
                TOP_PROCESSES_SCRIPT,
                PAGEFILE_SCRIPT,
            ])
            .await
            .context("Failed to execute RAM monitor batch")?;

        let memory_info = Self::parse_memory_info(&outputs[0])?;
        let physical_memory = Self::parse_physical_memory_info(&outputs[1])?;
        let detailed_memory = Self::parse_detailed_memory_breakdown(&outputs[2])?;
        let committed_memory = Self::parse_committed_memory(&outputs[3])?;
        let top_processes = Self::parse_top_memory_consumers(&outputs[4])?;
        let pagefiles = Self::parse_pagefile_info(&outputs[5])?;

        let total_pagefile_size: u64 = pagefiles.iter().map(|pf| pf.total_size).sum();
        let total_pagefile_used: u64 = pagefiles.iter().map(|pf| pf.current_usage).sum();

        Ok(RamData {
            total: memory_info.TotalVisibleMemorySize * 1024,
            used: (memory_info.TotalVisibleMemorySize - memory_info.FreePhysicalMemory) * 1024,
            available: memory_info.FreePhysicalMemory * 1024,
            cached: detailed_memory.cached(),
            free: detailed_memory.free(),
            speed: physical_memory.speed,
            type_name: physical_memory.memory_type,

            // Memory Breakdown
            in_use: detailed_memory.in_use(),
            standby: detailed_memory.standby(),
            modified: detailed_memory.modified(),

            // Committed Memory
            committed: committed_memory.committed(),
            commit_limit: committed_memory.commit_limit(),
            commit_percent: committed_memory.commit_percent(),

            // Top Memory Consumers
            top_processes,

            // Pagefile Information
            pagefiles,
            total_pagefile_size,
            total_pagefile_used,
        })
    }

    fn parse_memory_info(output: &str) -> Result<Win32OperatingSystem> {
        serde_json::from_str(output).context("Failed to parse memory info")
    }

    fn parse_physical_memory_info(output: &str) -> Result<PhysicalMemoryInfo> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        let info: PhysicalMemory = serde_json::from_str(trimmed)
            .context("Failed to parse physical memory info")?;

        Ok(PhysicalMemoryInfo {
            speed: info.Speed,
            memory_type: info.MemoryType,
        })
    }

    fn parse_detailed_memory_breakdown(output: &str) -> Result<DetailedMemory> {
        serde_json::from_str(output).context("Failed to parse detailed memory info")
    }

    fn parse_committed_memory(output: &str) -> Result<CommittedMemory> {
        serde_json::from_str(output).context("Failed to parse committed memory info")
    }

    fn parse_top_memory_consumers(output: &str) -> Result<Vec<ProcessMemoryInfo>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(Vec::new());
        }
        if !(trimmed.starts_with('[') || trimmed.starts_with('{')) {
            return Ok(Vec::new());
        }

        let samples: Vec<ProcessMemorySample> = if trimmed.starts_with('[') {
            serde_json::from_str(output).context("Failed to parse top processes")?
        } else {
            let single: ProcessMemorySample = serde_json::from_str(output)
                .context("Failed to parse single process")?;
            vec![single]
        };

        Ok(samples
            .into_iter()
            .map(|p| ProcessMemoryInfo {
                pid: p.Pid,
                name: p.Name,
                working_set: p.WorkingSet,
                private_bytes: p.PrivateBytes,
            })
            .collect())
    }

    fn parse_pagefile_info(output: &str) -> Result<Vec<PagefileInfo>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(Vec::new());
        }
        if !(trimmed.starts_with('[') || trimmed.starts_with('{')) {
            return Ok(Vec::new());
        }

        let samples: Vec<PagefileSample> = if trimmed.starts_with('[') {
            serde_json::from_str(output).context("Failed to parse pagefiles")?
        } else {
            let single: PagefileSample = serde_json::from_str(output)
                .context("Failed to parse single pagefile")?;
            vec![single]
        };

        Ok(samples
            .into_iter()
            .map(|p| PagefileInfo {
                name: p.Name,
                total_size: p.TotalSize,
                current_usage: p.CurrentUsage,
                peak_usage: p.PeakUsage,
                usage_percent: p.UsagePercent,
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct Win32OperatingSystem {
    TotalVisibleMemorySize: u64,
    FreePhysicalMemory: u64,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct PhysicalMemory {
    Speed: String,
    MemoryType: String,
}

#[derive(Debug)]
struct PhysicalMemoryInfo {
    speed: String,
    memory_type: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct DetailedMemory {
    InUse: u64,
    #[allow(dead_code)]
    Available: u64,
    Cached: u64,
    Standby: u64,
    Free: u64,
    Modified: u64,
}

impl DetailedMemory {
    fn in_use(&self) -> u64 { self.InUse }
    fn cached(&self) -> u64 { self.Cached }
    fn standby(&self) -> u64 { self.Standby }
    fn free(&self) -> u64 { self.Free }
    fn modified(&self) -> u64 { self.Modified }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct CommittedMemory {
    Committed: u64,
    CommitLimit: u64,
    CommitPercent: f64,
}

impl CommittedMemory {
    fn committed(&self) -> u64 { self.Committed }
    fn commit_limit(&self) -> u64 { self.CommitLimit }
    fn commit_percent(&self) -> f64 { self.CommitPercent }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct ProcessMemorySample {
    Pid: u32,
    Name: String,
    WorkingSet: u64,
    PrivateBytes: u64,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct PagefileSample {
    Name: String,
    TotalSize: u64,
    CurrentUsage: u64,
    PeakUsage: u64,
    UsagePercent: f64,
}
