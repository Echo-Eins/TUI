use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskData {
    pub physical_disks: Vec<PhysicalDiskInfo>,
    pub logical_drives: Vec<DriveInfo>,
    pub io_stats: Vec<DiskIOStats>,
    pub process_activity: Vec<DiskProcessActivity>,
    pub io_history: Vec<DiskIOHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskIOStats {
    pub disk_number: u32,
    pub read_speed: f64,       // MB/s
    pub write_speed: f64,      // MB/s
    pub read_iops: f64,        // Operations per second
    pub write_iops: f64,       // Operations per second
    pub queue_depth: f64,      // Average queue length
    pub avg_response_time: f64,// Milliseconds
    pub active_time: f64,      // Percentage
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskProcessActivity {
    pub process_name: String,
    pub pid: u32,
    pub io_bytes_per_sec: f64, // Total I/O bytes per second
    pub read_bytes_per_sec: f64,
    pub write_bytes_per_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskIOHistory {
    pub disk_number: u32,
    pub read_history: VecDeque<f64>,   // Last 60 samples of read speed
    pub write_history: VecDeque<f64>,  // Last 60 samples of write speed
    pub iops_history: VecDeque<f64>,   // Last 60 samples of total IOPS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalDiskInfo {
    pub disk_number: u32,
    pub friendly_name: String,
    pub model: String,
    pub media_type: String,      // HDD, SSD, NVMe
    pub bus_type: String,         // SATA, NVMe, USB, etc.
    pub size: u64,
    pub health_status: String,    // Healthy, Warning, Unhealthy
    pub operational_status: String,
    pub temperature: Option<f32>,
    pub write_cache_enabled: bool,

    // SMART data
    pub power_on_hours: Option<u64>,
    pub tbw: Option<u64>,         // Total Bytes Written (for SSD)
    pub wear_level: Option<f32>,  // Wear leveling percentage

    // Associated logical drives
    pub partitions: Vec<String>,  // Drive letters (C:, D:, etc.)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveInfo {
    pub letter: String,
    pub name: String,
    pub drive_type: String,
    pub file_system: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub disk_number: Option<u32>, // Link to physical disk
}

pub struct DiskMonitor {
    ps: PowerShellExecutor,
    io_history_map: std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<u32, DiskIOHistory>>>,
}

impl DiskMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            io_history_map: std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        })
    }

    pub async fn collect_data(&self) -> Result<DiskData> {
        let physical_disks = self.get_physical_disks().await?;
        let logical_drives = self.get_logical_drives().await?;
        let io_stats = self.get_io_stats().await?;
        let process_activity = self.get_process_activity().await?;

        // Update history
        let mut history_map = self.io_history_map.lock();
        for stat in &io_stats {
            let history = history_map
                .entry(stat.disk_number)
                .or_insert_with(|| DiskIOHistory {
                    disk_number: stat.disk_number,
                    read_history: VecDeque::with_capacity(60),
                    write_history: VecDeque::with_capacity(60),
                    iops_history: VecDeque::with_capacity(60),
                });

            // Add new data points
            history.read_history.push_back(stat.read_speed);
            history.write_history.push_back(stat.write_speed);
            history.iops_history.push_back(stat.read_iops + stat.write_iops);

            // Keep only last 60 samples
            if history.read_history.len() > 60 {
                history.read_history.pop_front();
            }
            if history.write_history.len() > 60 {
                history.write_history.pop_front();
            }
            if history.iops_history.len() > 60 {
                history.iops_history.pop_front();
            }
        }

        let io_history: Vec<DiskIOHistory> = history_map.values().cloned().collect();
        drop(history_map);

        Ok(DiskData {
            physical_disks,
            logical_drives,
            io_stats,
            process_activity,
            io_history,
        })
    }

    async fn get_physical_disks(&self) -> Result<Vec<PhysicalDiskInfo>> {
        let script = r#"
            $disks = Get-PhysicalDisk
            $result = @()

            foreach ($disk in $disks) {
                # Get partitions for this disk
                $partitions = Get-Partition -DiskNumber $disk.DeviceId -ErrorAction SilentlyContinue |
                    Where-Object { $_.DriveLetter } |
                    ForEach-Object { "$($_.DriveLetter):" }

                # Try to get SMART data (may not be available on all systems)
                $smart = $null
                try {
                    $smart = Get-StorageReliabilityCounter -PhysicalDisk $disk -ErrorAction SilentlyContinue
                } catch {}

                # Determine media type more precisely
                $mediaType = switch ($disk.MediaType) {
                    "HDD" { "HDD" }
                    "SSD" {
                        if ($disk.BusType -eq "NVMe") { "NVMe SSD" }
                        else { "SSD" }
                    }
                    "SCM" { "Storage Class Memory" }
                    default { $disk.MediaType }
                }

                # Get temperature if available
                $temperature = $null
                try {
                    $temp = Get-CimInstance -Namespace root/wmi -ClassName MSStorageDriver_FailurePredictData -ErrorAction SilentlyContinue |
                        Where-Object { $_.InstanceName -like "*$($disk.DeviceId)*" } |
                        Select-Object -First 1
                    if ($temp -and $temp.VendorSpecific) {
                        $temperature = $temp.VendorSpecific[12]
                    }
                } catch {}

                # Calculate TBW (Total Bytes Written) for SSDs
                $tbw = $null
                if ($smart -and $disk.MediaType -eq "SSD") {
                    try {
                        # Convert sectors to bytes (typically 512 bytes per sector)
                        $tbw = [uint64]($smart.WriteLatencyMax * 512)
                    } catch {}
                }

                # Wear level estimation (for SSDs)
                $wearLevel = $null
                if ($disk.MediaType -eq "SSD" -and $smart) {
                    try {
                        $wearLevel = 100.0 - ($smart.Wear)
                    } catch {}
                }

                # Health status translation
                $healthStatus = switch ($disk.HealthStatus) {
                    0 { "Healthy" }
                    1 { "Warning" }
                    2 { "Unhealthy" }
                    5 { "Unknown" }
                    default { "Healthy" }
                }

                # Operational status
                $operationalStatus = switch ($disk.OperationalStatus) {
                    "OK" { "OK" }
                    "Degraded" { "Degraded" }
                    "Error" { "Error" }
                    default { "$($disk.OperationalStatus)" }
                }

                $result += [PSCustomObject]@{
                    DiskNumber = $disk.DeviceId
                    FriendlyName = $disk.FriendlyName
                    Model = $disk.Model
                    MediaType = $mediaType
                    BusType = "$($disk.BusType)"
                    Size = [uint64]$disk.Size
                    HealthStatus = $healthStatus
                    OperationalStatus = $operationalStatus
                    Temperature = $temperature
                    WriteCacheEnabled = $disk.WriteCacheEnabled
                    PowerOnHours = if ($smart) { [uint64]$smart.PowerOnHours } else { $null }
                    TBW = $tbw
                    WearLevel = $wearLevel
                    Partitions = @($partitions)
                }
            }

            $result | ConvertTo-Json -Depth 3
        "#;

        let output = self.ps.execute(script).await?;

        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let disks: Vec<PhysicalDiskSample> = if output.trim().starts_with('[') {
            serde_json::from_str(&output).context("Failed to parse physical disks")?
        } else {
            let single: PhysicalDiskSample = serde_json::from_str(&output)
                .context("Failed to parse single physical disk")?;
            vec![single]
        };

        Ok(disks
            .into_iter()
            .map(|d| PhysicalDiskInfo {
                disk_number: d.DiskNumber,
                friendly_name: d.FriendlyName,
                model: d.Model,
                media_type: d.MediaType,
                bus_type: d.BusType,
                size: d.Size,
                health_status: d.HealthStatus,
                operational_status: d.OperationalStatus,
                temperature: d.Temperature,
                write_cache_enabled: d.WriteCacheEnabled,
                power_on_hours: d.PowerOnHours,
                tbw: d.TBW,
                wear_level: d.WearLevel,
                partitions: d.Partitions.unwrap_or_default(),
            })
            .collect())
    }

    async fn get_logical_drives(&self) -> Result<Vec<DriveInfo>> {
        let script = r#"
            Get-CimInstance Win32_LogicalDisk |
                Where-Object { $_.DriveType -eq 3 } |
                ForEach-Object {
                    # Try to find the disk number for this drive
                    $diskNumber = $null
                    try {
                        $partition = Get-Partition -DriveLetter $_.DeviceID[0] -ErrorAction SilentlyContinue
                        if ($partition) {
                            $diskNumber = $partition.DiskNumber
                        }
                    } catch {}

                    [PSCustomObject]@{
                        Letter = $_.DeviceID
                        Name = if ($_.VolumeName) { $_.VolumeName } else { "" }
                        DriveType = "Fixed"
                        FileSystem = $_.FileSystem
                        Total = [uint64]$_.Size
                        Free = [uint64]$_.FreeSpace
                        DiskNumber = $diskNumber
                    }
                } | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;

        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let drives: Vec<DriveSample> = if output.trim().starts_with('[') {
            serde_json::from_str(&output).context("Failed to parse logical drives")?
        } else {
            let single: DriveSample = serde_json::from_str(&output)
                .context("Failed to parse single logical drive")?;
            vec![single]
        };

        Ok(drives
            .into_iter()
            .map(|d| DriveInfo {
                letter: d.Letter,
                name: d.Name.unwrap_or_else(|| "Local Disk".to_string()),
                drive_type: d.DriveType.unwrap_or_else(|| "Fixed".to_string()),
                file_system: d.FileSystem.unwrap_or_else(|| "NTFS".to_string()),
                total: d.Total.unwrap_or(0),
                used: d.Total.unwrap_or(0).saturating_sub(d.Free.unwrap_or(0)),
                free: d.Free.unwrap_or(0),
                disk_number: d.DiskNumber,
            })
            .collect())
    }

    async fn get_io_stats(&self) -> Result<Vec<DiskIOStats>> {
        let script = r#"
            $disks = Get-PhysicalDisk
            $result = @()

            foreach ($disk in $disks) {
                try {
                    # Get I/O performance counters for this disk
                    $diskId = $disk.DeviceId

                    # PhysicalDisk counters
                    $readBytesPath = "\PhysicalDisk($diskId *)\Disk Read Bytes/sec"
                    $writeBytesPath = "\PhysicalDisk($diskId *)\Disk Write Bytes/sec"
                    $readOpsPath = "\PhysicalDisk($diskId *)\Disk Reads/sec"
                    $writeOpsPath = "\PhysicalDisk($diskId *)\Disk Writes/sec"
                    $queuePath = "\PhysicalDisk($diskId *)\Current Disk Queue Length"
                    $avgSecPath = "\PhysicalDisk($diskId *)\Avg. Disk sec/Transfer"
                    $activeTimePath = "\PhysicalDisk($diskId *)\% Disk Time"

                    # Try to get performance counters
                    $counters = @()
                    try {
                        $counters = Get-Counter -Counter @(
                            $readBytesPath,
                            $writeBytesPath,
                            $readOpsPath,
                            $writeOpsPath,
                            $queuePath,
                            $avgSecPath,
                            $activeTimePath
                        ) -ErrorAction SilentlyContinue
                    } catch {}

                    $readSpeed = 0.0
                    $writeSpeed = 0.0
                    $readIOPS = 0.0
                    $writeIOPS = 0.0
                    $queueDepth = 0.0
                    $avgResponseTime = 0.0
                    $activeTime = 0.0

                    if ($counters -and $counters.CounterSamples) {
                        foreach ($sample in $counters.CounterSamples) {
                            if ($sample.Path -like "*Read Bytes/sec*") {
                                $readSpeed = [math]::Round($sample.CookedValue / 1MB, 2)
                            }
                            elseif ($sample.Path -like "*Write Bytes/sec*") {
                                $writeSpeed = [math]::Round($sample.CookedValue / 1MB, 2)
                            }
                            elseif ($sample.Path -like "*Reads/sec*") {
                                $readIOPS = [math]::Round($sample.CookedValue, 2)
                            }
                            elseif ($sample.Path -like "*Writes/sec*") {
                                $writeIOPS = [math]::Round($sample.CookedValue, 2)
                            }
                            elseif ($sample.Path -like "*Queue Length*") {
                                $queueDepth = [math]::Round($sample.CookedValue, 2)
                            }
                            elseif ($sample.Path -like "*sec/Transfer*") {
                                $avgResponseTime = [math]::Round($sample.CookedValue * 1000, 2)
                            }
                            elseif ($sample.Path -like "*% Disk Time*") {
                                $activeTime = [math]::Round($sample.CookedValue, 2)
                            }
                        }
                    }

                    $result += [PSCustomObject]@{
                        DiskNumber = $diskId
                        ReadSpeed = $readSpeed
                        WriteSpeed = $writeSpeed
                        ReadIOPS = $readIOPS
                        WriteIOPS = $writeIOPS
                        QueueDepth = $queueDepth
                        AvgResponseTime = $avgResponseTime
                        ActiveTime = $activeTime
                    }
                } catch {
                    # If performance counters fail, return zeros for this disk
                    $result += [PSCustomObject]@{
                        DiskNumber = $disk.DeviceId
                        ReadSpeed = 0.0
                        WriteSpeed = 0.0
                        ReadIOPS = 0.0
                        WriteIOPS = 0.0
                        QueueDepth = 0.0
                        AvgResponseTime = 0.0
                        ActiveTime = 0.0
                    }
                }
            }

            $result | ConvertTo-Json -Depth 2
        "#;

        let output = self.ps.execute(script).await?;

        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let stats: Vec<IOStatsSample> = if output.trim().starts_with('[') {
            serde_json::from_str(&output).context("Failed to parse I/O stats")?
        } else {
            let single: IOStatsSample = serde_json::from_str(&output)
                .context("Failed to parse single I/O stat")?;
            vec![single]
        };

        Ok(stats
            .into_iter()
            .map(|s| DiskIOStats {
                disk_number: s.DiskNumber,
                read_speed: s.ReadSpeed.unwrap_or(0.0),
                write_speed: s.WriteSpeed.unwrap_or(0.0),
                read_iops: s.ReadIOPS.unwrap_or(0.0),
                write_iops: s.WriteIOPS.unwrap_or(0.0),
                queue_depth: s.QueueDepth.unwrap_or(0.0),
                avg_response_time: s.AvgResponseTime.unwrap_or(0.0),
                active_time: s.ActiveTime.unwrap_or(0.0),
            })
            .collect())
    }

    async fn get_process_activity(&self) -> Result<Vec<DiskProcessActivity>> {
        let script = r#"
            # Get top 10 processes by I/O activity
            $processes = Get-Counter '\Process(*)\IO Data Bytes/sec' -ErrorAction SilentlyContinue

            $result = @()

            if ($processes -and $processes.CounterSamples) {
                $sorted = $processes.CounterSamples |
                    Where-Object { $_.CookedValue -gt 0 } |
                    Sort-Object -Property CookedValue -Descending |
                    Select-Object -First 10

                foreach ($sample in $sorted) {
                    # Extract process name from path
                    if ($sample.Path -match '\\Process\(([^)]+)\)') {
                        $processName = $matches[1]

                        # Try to get PID and more details
                        try {
                            $proc = Get-Process -Name $processName -ErrorAction SilentlyContinue | Select-Object -First 1

                            if ($proc) {
                                # Try to get read/write breakdown (may not be available for all processes)
                                $readBytes = 0.0
                                $writeBytes = 0.0

                                try {
                                    $readCounter = Get-Counter "\Process($processName)\IO Read Bytes/sec" -ErrorAction SilentlyContinue
                                    if ($readCounter) {
                                        $readBytes = $readCounter.CounterSamples[0].CookedValue
                                    }
                                } catch {}

                                try {
                                    $writeCounter = Get-Counter "\Process($processName)\IO Write Bytes/sec" -ErrorAction SilentlyContinue
                                    if ($writeCounter) {
                                        $writeBytes = $writeCounter.CounterSamples[0].CookedValue
                                    }
                                } catch {}

                                $result += [PSCustomObject]@{
                                    ProcessName = $processName
                                    PID = $proc.Id
                                    IOBytesPerSec = [math]::Round($sample.CookedValue, 2)
                                    ReadBytesPerSec = [math]::Round($readBytes, 2)
                                    WriteBytesPerSec = [math]::Round($writeBytes, 2)
                                }
                            }
                        } catch {
                            # If we can't get process details, skip it
                        }
                    }
                }
            }

            $result | ConvertTo-Json -Depth 2
        "#;

        let output = self.ps.execute(script).await?;

        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let activities: Vec<ProcessActivitySample> = if output.trim().starts_with('[') {
            serde_json::from_str(&output).context("Failed to parse process activity")?
        } else {
            let single: ProcessActivitySample = serde_json::from_str(&output)
                .context("Failed to parse single process activity")?;
            vec![single]
        };

        Ok(activities
            .into_iter()
            .map(|a| DiskProcessActivity {
                process_name: a.ProcessName,
                pid: a.PID,
                io_bytes_per_sec: a.IOBytesPerSec.unwrap_or(0.0),
                read_bytes_per_sec: a.ReadBytesPerSec.unwrap_or(0.0),
                write_bytes_per_sec: a.WriteBytesPerSec.unwrap_or(0.0),
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct DriveSample {
    Letter: String,
    Name: Option<String>,
    DriveType: Option<String>,
    FileSystem: Option<String>,
    Total: Option<u64>,
    Free: Option<u64>,
    DiskNumber: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct PhysicalDiskSample {
    DiskNumber: u32,
    FriendlyName: String,
    Model: String,
    MediaType: String,
    BusType: String,
    Size: u64,
    HealthStatus: String,
    OperationalStatus: String,
    Temperature: Option<f32>,
    WriteCacheEnabled: bool,
    PowerOnHours: Option<u64>,
    TBW: Option<u64>,
    WearLevel: Option<f32>,
    Partitions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct IOStatsSample {
    DiskNumber: u32,
    ReadSpeed: Option<f64>,
    WriteSpeed: Option<f64>,
    ReadIOPS: Option<f64>,
    WriteIOPS: Option<f64>,
    QueueDepth: Option<f64>,
    AvgResponseTime: Option<f64>,
    ActiveTime: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ProcessActivitySample {
    ProcessName: String,
    PID: u32,
    IOBytesPerSec: Option<f64>,
    ReadBytesPerSec: Option<f64>,
    WriteBytesPerSec: Option<f64>,
}
