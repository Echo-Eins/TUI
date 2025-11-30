use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;

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
}

impl RamMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    pub async fn collect_data(&self) -> Result<RamData> {
        let memory_info = self.get_memory_info().await?;
        let physical_memory = self.get_physical_memory_info().await?;
        let detailed_memory = self.get_detailed_memory_breakdown().await?;
        let committed_memory = self.get_committed_memory().await?;
        let top_processes = self.get_top_memory_consumers().await?;
        let pagefiles = self.get_pagefile_info().await?;

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

    async fn get_memory_info(&self) -> Result<Win32OperatingSystem> {
        let script = r#"
            Get-CimInstance Win32_OperatingSystem | Select-Object TotalVisibleMemorySize, FreePhysicalMemory | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        serde_json::from_str(&output).context("Failed to parse memory info")
    }

    async fn get_physical_memory_info(&self) -> Result<PhysicalMemoryInfo> {
        let script = r#"
            $mem = Get-CimInstance Win32_PhysicalMemory | Select-Object -First 1
            [PSCustomObject]@{
                Speed = "$($mem.Speed) MHz"
                MemoryType = switch ($mem.MemoryType) {
                    20 { "DDR" }
                    21 { "DDR2" }
                    24 { "DDR3" }
                    26 { "DDR4" }
                    34 { "DDR5" }
                    default { "Unknown" }
                }
            } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        let info: PhysicalMemory = serde_json::from_str(&output)
            .context("Failed to parse physical memory info")?;

        Ok(PhysicalMemoryInfo {
            speed: info.Speed,
            memory_type: info.MemoryType,
        })
    }

    async fn get_detailed_memory_breakdown(&self) -> Result<DetailedMemory> {
        let script = r#"
            $counters = @(
                '\Memory\Available Bytes',
                '\Memory\Cache Bytes',
                '\Memory\Standby Cache Normal Priority Bytes',
                '\Memory\Standby Cache Reserve Bytes',
                '\Memory\Standby Cache Core Bytes',
                '\Memory\Free & Zero Page List Bytes',
                '\Memory\Modified Page List Bytes'
            )

            $os = Get-CimInstance Win32_OperatingSystem
            $total = $os.TotalVisibleMemorySize * 1024

            try {
                $perfData = Get-Counter -Counter $counters -ErrorAction SilentlyContinue

                $available = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Available Bytes*'}).CookedValue
                $cached = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Cache Bytes*'}).CookedValue
                $standbyNormal = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Standby Cache Normal*'}).CookedValue
                $standbyReserve = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Standby Cache Reserve*'}).CookedValue
                $standbyCore = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Standby Cache Core*'}).CookedValue
                $free = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Free & Zero*'}).CookedValue
                $modified = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Modified Page*'}).CookedValue

                $standby = $standbyNormal + $standbyReserve + $standbyCore
                $inUse = $total - $available

                [PSCustomObject]@{
                    InUse = [uint64]$inUse
                    Available = [uint64]$available
                    Cached = [uint64]$cached
                    Standby = [uint64]$standby
                    Free = [uint64]$free
                    Modified = [uint64]$modified
                }
            } catch {
                # Fallback to basic memory info
                [PSCustomObject]@{
                    InUse = [uint64]($total - $available)
                    Available = [uint64]$available
                    Cached = [uint64]($cached)
                    Standby = [uint64]0
                    Free = [uint64]($os.FreePhysicalMemory * 1024)
                    Modified = [uint64]0
                }
            } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        serde_json::from_str(&output).context("Failed to parse detailed memory info")
    }

    async fn get_committed_memory(&self) -> Result<CommittedMemory> {
        let script = r#"
            $counters = @(
                '\Memory\Committed Bytes',
                '\Memory\Commit Limit'
            )

            try {
                $perfData = Get-Counter -Counter $counters -ErrorAction SilentlyContinue

                $committed = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Committed Bytes*'}).CookedValue
                $commitLimit = ($perfData.CounterSamples | Where-Object {$_.Path -like '*Commit Limit*'}).CookedValue

                $commitPercent = if ($commitLimit -gt 0) { ($committed / $commitLimit) * 100 } else { 0 }

                [PSCustomObject]@{
                    Committed = [uint64]$committed
                    CommitLimit = [uint64]$commitLimit
                    CommitPercent = [double]$commitPercent
                }
            } catch {
                $os = Get-CimInstance Win32_OperatingSystem
                $pageFile = Get-CimInstance Win32_PageFileUsage | Select-Object -First 1

                $committed = ($os.TotalVisibleMemorySize - $os.FreePhysicalMemory) * 1024
                $commitLimit = ($os.TotalVisibleMemorySize * 1024) + ($pageFile.AllocatedBaseSize * 1024 * 1024)
                $commitPercent = if ($commitLimit -gt 0) { ($committed / $commitLimit) * 100 } else { 0 }

                [PSCustomObject]@{
                    Committed = [uint64]$committed
                    CommitLimit = [uint64]$commitLimit
                    CommitPercent = [double]$commitPercent
                }
            } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        serde_json::from_str(&output).context("Failed to parse committed memory info")
    }

    async fn get_top_memory_consumers(&self) -> Result<Vec<ProcessMemoryInfo>> {
        let script = r#"
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
        "#;

        let output = self.ps.execute(script).await?;

        // Handle both single object and array responses
        let processes: Vec<ProcessMemoryInfo> = if output.trim().starts_with('[') {
            serde_json::from_str(&output).context("Failed to parse top processes")?
        } else {
            let single: ProcessMemoryInfo = serde_json::from_str(&output)
                .context("Failed to parse single process")?;
            vec![single]
        };

        Ok(processes)
    }

    async fn get_pagefile_info(&self) -> Result<Vec<PagefileInfo>> {
        let script = r#"
            $pagefiles = Get-CimInstance Win32_PageFileUsage

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
                # No pagefile configured, return empty array
                "[]"
            }
        "#;

        let output = self.ps.execute(script).await?;

        // Handle empty array, single object, and array responses
        if output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let pagefiles: Vec<PagefileInfo> = if output.trim().starts_with('[') {
            serde_json::from_str(&output).context("Failed to parse pagefiles")?
        } else {
            let single: PagefileInfo = serde_json::from_str(&output)
                .context("Failed to parse single pagefile")?;
            vec![single]
        };

        Ok(pagefiles)
    }
}

#[derive(Debug, Deserialize)]
struct Win32OperatingSystem {
    TotalVisibleMemorySize: u64,
    FreePhysicalMemory: u64,
}

#[derive(Debug, Deserialize)]
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
struct DetailedMemory {
    InUse: u64,
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
