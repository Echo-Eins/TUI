use crate::integrations::{LinuxSysMonitor, PowerShellExecutor};
use crate::integrations::{ProcessIoSample, RawDiskStats};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Instant;

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
    pub read_speed: f64,        // MB/s
    pub write_speed: f64,       // MB/s
    pub read_iops: f64,         // Operations per second
    pub write_iops: f64,        // Operations per second
    pub queue_depth: f64,       // Average queue length
    pub avg_response_time: f64, // Milliseconds
    pub active_time: f64,       // Percentage
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
    pub read_history: VecDeque<f64>,  // Last 60 samples of read speed
    pub write_history: VecDeque<f64>, // Last 60 samples of write speed
    pub iops_history: VecDeque<f64>,  // Last 60 samples of total IOPS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalDiskInfo {
    pub disk_number: u32,
    pub friendly_name: String,
    pub model: String,
    pub media_type: String, // HDD, SSD, NVMe
    pub bus_type: String,   // SATA, NVMe, USB, etc.
    pub size: u64,
    pub health_status: String, // Healthy, Warning, Unhealthy
    pub operational_status: String,
    pub temperature: Option<f32>,
    pub write_cache_enabled: bool,

    // SMART data
    pub power_on_hours: Option<u64>,
    pub tbw: Option<u64>,        // Total Bytes Written (for SSD)
    pub wear_level: Option<f32>, // Wear leveling percentage

    // Associated logical drives
    pub partitions: Vec<String>, // Drive letters (C:, D:, etc.)
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
    #[allow(dead_code)]
    linux_sys: LinuxSysMonitor,
    io_history_map:
        std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<u32, DiskIOHistory>>>,
    linux_diskstats_prev:
        std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<String, RawDiskStats>>>,
    linux_diskstats_ts: std::sync::Arc<parking_lot::Mutex<Option<Instant>>>,
    linux_proc_io_prev:
        std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<u32, ProcessIoSample>>>,
}

const PHYSICAL_DISKS_SCRIPT: &str = r#"
    if (-not (Get-Command Get-PhysicalDisk -ErrorAction SilentlyContinue)) {
        "[]"
    } else {
        $disks = Get-PhysicalDisk -ErrorAction SilentlyContinue
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
                DiskNumber = [uint32]$disk.DeviceId
                FriendlyName = $disk.FriendlyName
                Model = $disk.Model
                MediaType = $mediaType
                BusType = "$($disk.BusType)"
                Size = [uint64]$disk.Size
                HealthStatus = $healthStatus
                OperationalStatus = $operationalStatus
                Temperature = $temperature
                WriteCacheEnabled = if ($null -ne $disk.WriteCacheEnabled) { [bool]$disk.WriteCacheEnabled } else { $false }
                PowerOnHours = if ($smart) { [uint64]$smart.PowerOnHours } else { $null }
                TBW = $tbw
                WearLevel = $wearLevel
                Partitions = @($partitions)
            }
        }

        $result | ConvertTo-Json -Depth 3
    }
"#;

const LOGICAL_DRIVES_SCRIPT: &str = r#"
    try {
        $drives = Get-CimInstance Win32_LogicalDisk -ErrorAction Stop |
            Where-Object { $_.DriveType -eq 3 }

        $result = foreach ($drive in $drives) {
            $diskNumber = $null
            try {
                $partition = Get-Partition -DriveLetter $drive.DeviceID[0] -ErrorAction SilentlyContinue
                if ($partition) {
                    $diskNumber = $partition.DiskNumber
                }
            } catch {}

            [PSCustomObject]@{
                Letter = $drive.DeviceID
                Name = if ($drive.VolumeName) { $drive.VolumeName } else { "" }
                DriveType = "Fixed"
                FileSystem = $drive.FileSystem
                Total = [uint64]$drive.Size
                Free = [uint64]$drive.FreeSpace
                DiskNumber = $diskNumber
            }
        }

        if ($result) {
            $result | ConvertTo-Json
        } else {
            "[]"
        }
    } catch {
        "[]"
    }
"#;

const IO_STATS_SCRIPT: &str = r#"
    if (-not (Get-Command Get-PhysicalDisk -ErrorAction SilentlyContinue)) {
        "[]"
    } elseif (-not (Get-Command Get-Counter -ErrorAction SilentlyContinue)) {
        "[]"
    } else {
        $disks = Get-PhysicalDisk -ErrorAction SilentlyContinue
        $result = @()

        foreach ($disk in $disks) {
            try {
                $diskId = [uint32]$disk.DeviceId

                $readBytesPath = "\PhysicalDisk($diskId *)\Disk Read Bytes/sec"
                $writeBytesPath = "\PhysicalDisk($diskId *)\Disk Write Bytes/sec"
                $readOpsPath = "\PhysicalDisk($diskId *)\Disk Reads/sec"
                $writeOpsPath = "\PhysicalDisk($diskId *)\Disk Writes/sec"
                $queuePath = "\PhysicalDisk($diskId *)\Current Disk Queue Length"
                $avgSecPath = "\PhysicalDisk($diskId *)\Avg. Disk sec/Transfer"
                $activeTimePath = "\PhysicalDisk($diskId *)\% Disk Time"

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
                $result += [PSCustomObject]@{
                    DiskNumber = [uint32]$disk.DeviceId
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
    }
"#;

const PROCESS_ACTIVITY_SCRIPT: &str = r#"
    if (-not (Get-Command Get-Counter -ErrorAction SilentlyContinue)) {
        "[]"
    } else {
        try {
            $processes = Get-Counter '\Process(*)\IO Data Bytes/sec' -ErrorAction Stop

            $result = @()

        if ($processes -and $processes.CounterSamples) {
            $sorted = $processes.CounterSamples |
                Where-Object { $_.CookedValue -gt 0 } |
                Sort-Object -Property CookedValue -Descending |
                Select-Object -First 10

            foreach ($sample in $sorted) {
                if ($sample.Path -match '\\Process\(([^)]+)\)') {
                    $processName = $matches[1]

                    try {
                        $proc = Get-Process -Name $processName -ErrorAction SilentlyContinue | Select-Object -First 1

                        if ($proc) {
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
                    }
                }
            }
        }

            $result | ConvertTo-Json -Depth 2
        } catch {
            "[]"
        }
    }
"#;

impl DiskMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            linux_sys: LinuxSysMonitor::new(),
            io_history_map: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
            linux_diskstats_prev: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
            linux_diskstats_ts: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            linux_proc_io_prev: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
        })
    }

    pub async fn collect_data(&self) -> Result<DiskData> {
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
    async fn collect_data_linux(&self) -> Result<DiskData> {
        let disks = self.linux_sys.get_disk_info()?;
        let block_devices = self.linux_sys.get_block_devices().unwrap_or_default();
        let disk_stats = self.linux_sys.get_disk_stats().unwrap_or_default();
        let process_samples = self.linux_sys.get_process_io_samples().unwrap_or_default();

        let physical_disks: Vec<PhysicalDiskInfo> = block_devices
            .iter()
            .filter(|d| d.dev_type == "disk")
            .enumerate()
            .map(|(idx, d)| PhysicalDiskInfo {
                disk_number: idx as u32,
                friendly_name: if d.model.trim().is_empty() {
                    d.name.clone()
                } else {
                    d.model.clone()
                },
                model: d.model.clone(),
                media_type: if d.rotational {
                    "HDD".to_string()
                } else {
                    "SSD/NVMe".to_string()
                },
                bus_type: if d.transport.is_empty() {
                    "Unknown".to_string()
                } else {
                    d.transport.clone()
                },
                size: d.size,
                health_status: "Unknown".to_string(),
                operational_status: "Online".to_string(),
                temperature: None,
                write_cache_enabled: false,
                power_on_hours: None,
                tbw: None,
                wear_level: None,
                partitions: block_devices
                    .iter()
                    .filter(|child| child.parent.as_deref() == Some(d.name.as_str()))
                    .filter_map(|child| child.mount_point.clone())
                    .collect(),
            })
            .collect();

        let logical_drives: Vec<DriveInfo> = disks
            .iter()
            .map(|d| DriveInfo {
                letter: d.mount_point.clone(),
                name: d.name.clone(),
                drive_type: d.fs_type.clone(),
                file_system: d.fs_type.clone(),
                total: d.total,
                used: d.used,
                free: d.available,
                disk_number: None,
            })
            .collect();

        let io_stats = self.compute_linux_io_stats(&disk_stats);
        let process_activity = self.compute_linux_process_activity(&process_samples);
        self.update_io_history(&io_stats);
        let io_history = self.io_history_map.lock().values().cloned().collect();

        Ok(DiskData {
            physical_disks,
            logical_drives,
            io_stats,
            process_activity,
            io_history,
        })
    }

    fn compute_linux_io_stats(&self, current_stats: &[RawDiskStats]) -> Vec<DiskIOStats> {
        let now = Instant::now();
        let mut prev_map = self.linux_diskstats_prev.lock();
        let mut prev_ts = self.linux_diskstats_ts.lock();
        let elapsed = prev_ts
            .map(|ts| now.saturating_duration_since(ts).as_secs_f64())
            .unwrap_or(0.0);

        let mut out = Vec::new();
        for (idx, stat) in current_stats.iter().enumerate() {
            let mut read_speed = 0.0;
            let mut write_speed = 0.0;
            let mut read_iops = 0.0;
            let mut write_iops = 0.0;
            let mut avg_response_time = 0.0;

            if elapsed > 0.0 {
                if let Some(prev) = prev_map.get(&stat.name) {
                    let delta_read_sectors = stat.sectors_read.saturating_sub(prev.sectors_read);
                    let delta_write_sectors =
                        stat.sectors_written.saturating_sub(prev.sectors_written);
                    let delta_reads = stat.reads_completed.saturating_sub(prev.reads_completed);
                    let delta_writes = stat.writes_completed.saturating_sub(prev.writes_completed);

                    read_speed = (delta_read_sectors as f64 * 512.0) / (1024.0 * 1024.0) / elapsed;
                    write_speed =
                        (delta_write_sectors as f64 * 512.0) / (1024.0 * 1024.0) / elapsed;
                    read_iops = delta_reads as f64 / elapsed;
                    write_iops = delta_writes as f64 / elapsed;

                    let delta_ms = stat.ms_reading.saturating_sub(prev.ms_reading)
                        + stat.ms_writing.saturating_sub(prev.ms_writing);
                    let total_ios = delta_reads + delta_writes;
                    if total_ios > 0 {
                        avg_response_time = delta_ms as f64 / total_ios as f64;
                    }
                }
            }

            out.push(DiskIOStats {
                disk_number: idx as u32,
                read_speed,
                write_speed,
                read_iops,
                write_iops,
                queue_depth: stat.io_in_progress as f64,
                avg_response_time,
                active_time: 0.0,
            });
        }

        prev_map.clear();
        for stat in current_stats {
            prev_map.insert(stat.name.clone(), stat.clone());
        }
        *prev_ts = Some(now);

        out
    }

    fn compute_linux_process_activity(
        &self,
        current: &[ProcessIoSample],
    ) -> Vec<DiskProcessActivity> {
        let mut prev = self.linux_proc_io_prev.lock();
        let mut out = Vec::new();

        for sample in current {
            if let Some(old) = prev.get(&sample.pid) {
                let dt = sample
                    .timestamp
                    .saturating_duration_since(old.timestamp)
                    .as_secs_f64();
                if dt > 0.0 {
                    let delta_read = sample.read_bytes.saturating_sub(old.read_bytes) as f64;
                    let delta_write = sample.write_bytes.saturating_sub(old.write_bytes) as f64;
                    let read_per_sec = delta_read / dt;
                    let write_per_sec = delta_write / dt;
                    let total = read_per_sec + write_per_sec;

                    if total > 0.0 {
                        out.push(DiskProcessActivity {
                            process_name: sample.name.clone(),
                            pid: sample.pid,
                            io_bytes_per_sec: total,
                            read_bytes_per_sec: read_per_sec,
                            write_bytes_per_sec: write_per_sec,
                        });
                    }
                }
            }
        }

        prev.clear();
        for sample in current {
            prev.insert(sample.pid, sample.clone());
        }

        out.sort_by(|a, b| {
            b.io_bytes_per_sec
                .partial_cmp(&a.io_bytes_per_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.truncate(10);
        out
    }

    fn update_io_history(&self, io_stats: &[DiskIOStats]) {
        let mut history_map = self.io_history_map.lock();
        for stat in io_stats {
            let history = history_map
                .entry(stat.disk_number)
                .or_insert_with(|| DiskIOHistory {
                    disk_number: stat.disk_number,
                    read_history: VecDeque::with_capacity(60),
                    write_history: VecDeque::with_capacity(60),
                    iops_history: VecDeque::with_capacity(60),
                });

            history.read_history.push_back(stat.read_speed);
            history.write_history.push_back(stat.write_speed);
            history
                .iops_history
                .push_back(stat.read_iops + stat.write_iops);

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
    }

    async fn collect_data_windows(&self) -> Result<DiskData> {
        let outputs = self
            .ps
            .execute_batch(&[
                PHYSICAL_DISKS_SCRIPT,
                LOGICAL_DRIVES_SCRIPT,
                IO_STATS_SCRIPT,
                PROCESS_ACTIVITY_SCRIPT,
            ])
            .await
            .context("Failed to execute disk monitor batch")?;

        let physical_disks = Self::parse_physical_disks(&outputs[0])?;
        let logical_drives = Self::parse_logical_drives(&outputs[1])?;
        let io_stats = Self::parse_io_stats(&outputs[2])?;
        let process_activity = Self::parse_process_activity(&outputs[3])?;

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
            history
                .iops_history
                .push_back(stat.read_iops + stat.write_iops);

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

    fn parse_physical_disks(output: &str) -> Result<Vec<PhysicalDiskInfo>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(Vec::new());
        }
        if !(trimmed.starts_with('[') || trimmed.starts_with('{')) {
            return Ok(Vec::new());
        }

        let disks: Vec<PhysicalDiskSample> = if trimmed.starts_with('[') {
            serde_json::from_str(output).context("Failed to parse physical disks")?
        } else {
            let single: PhysicalDiskSample =
                serde_json::from_str(output).context("Failed to parse single physical disk")?;
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

    #[allow(dead_code)]
    async fn get_physical_disks(&self) -> Result<Vec<PhysicalDiskInfo>> {
        let output = self.ps.execute(PHYSICAL_DISKS_SCRIPT).await?;
        Self::parse_physical_disks(&output)
    }

    fn parse_logical_drives(output: &str) -> Result<Vec<DriveInfo>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(Vec::new());
        }
        if !(trimmed.starts_with('[') || trimmed.starts_with('{')) {
            return Ok(Vec::new());
        }

        let drives: Vec<DriveSample> = if trimmed.starts_with('[') {
            serde_json::from_str(output).context("Failed to parse logical drives")?
        } else {
            let single: DriveSample =
                serde_json::from_str(output).context("Failed to parse single logical drive")?;
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

    #[allow(dead_code)]
    async fn get_logical_drives(&self) -> Result<Vec<DriveInfo>> {
        let output = self.ps.execute(LOGICAL_DRIVES_SCRIPT).await?;
        Self::parse_logical_drives(&output)
    }

    fn parse_io_stats(output: &str) -> Result<Vec<DiskIOStats>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(Vec::new());
        }
        if !(trimmed.starts_with('[') || trimmed.starts_with('{')) {
            return Ok(Vec::new());
        }

        let stats: Vec<IOStatsSample> = if trimmed.starts_with('[') {
            serde_json::from_str(output).context("Failed to parse I/O stats")?
        } else {
            let single: IOStatsSample =
                serde_json::from_str(output).context("Failed to parse single I/O stat")?;
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

    #[allow(dead_code)]
    async fn get_io_stats(&self) -> Result<Vec<DiskIOStats>> {
        let output = self.ps.execute(IO_STATS_SCRIPT).await?;
        Self::parse_io_stats(&output)
    }

    fn parse_process_activity(output: &str) -> Result<Vec<DiskProcessActivity>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(Vec::new());
        }
        if !(trimmed.starts_with('[') || trimmed.starts_with('{')) {
            return Ok(Vec::new());
        }

        let activities: Vec<ProcessActivitySample> = if trimmed.starts_with('[') {
            serde_json::from_str(output).context("Failed to parse process activity")?
        } else {
            let single: ProcessActivitySample =
                serde_json::from_str(output).context("Failed to parse single process activity")?;
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

    #[allow(dead_code)]
    async fn get_process_activity(&self) -> Result<Vec<DiskProcessActivity>> {
        let output = self.ps.execute(PROCESS_ACTIVITY_SCRIPT).await?;
        Self::parse_process_activity(&output)
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
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
#[allow(non_snake_case)]
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
#[allow(non_snake_case)]
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
#[allow(non_snake_case)]
struct ProcessActivitySample {
    ProcessName: String,
    PID: u32,
    IOBytesPerSec: Option<f64>,
    ReadBytesPerSec: Option<f64>,
    WriteBytesPerSec: Option<f64>,
}
