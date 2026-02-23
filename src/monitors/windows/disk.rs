use crate::integrations::PowerShellExecutor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use crate::utils::parse_json_array;
use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};

pub struct WindowsDiskMonitor {
    ps: PowerShellExecutor,
    io_history: Mutex<HashMap<u32, DiskIOHistory>>,
}

const DISK_INFO_SCRIPT: &str = r#"
    try {
        $physical = Get-CimInstance MSFT_PhysicalDisk -Namespace Root\Microsoft\Windows\Storage -ErrorAction SilentlyContinue
        $logical = Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" -ErrorAction SilentlyContinue
        $smart = Get-WmiObject -namespace root\wmi -class MSStorageDriver_ATAPISmartData -ErrorAction SilentlyContinue
        $temps = Get-WmiObject -namespace root\wmi -class MSStorageDriver_ATAPISmartData -ErrorAction SilentlyContinue

        $tempMap = @{}
        if ($temps) {
            foreach ($t in $temps) {
                try {
                    $vendor = $t.VendorSpecific
                    if ($vendor -and $vendor.Length -gt 194) {
                        $tempMap[$t.InstanceName] = $vendor[194]
                    }
                } catch { }
            }
        }

        $smartMap = @{}
        if ($smart) {
            foreach ($s in $smart) {
                $smartMap[$s.InstanceName] = $s
            }
        }

        $pInfo = foreach ($disk in $physical) {
            $num = $disk.DeviceId
            $temp = $null
            if ($tempMap.Count -gt 0) {
                $match = $tempMap.Keys | Where-Object { $_ -match "Disk$num" } | Select-Object -First 1
                if ($match) { $temp = $tempMap[$match] }
            }

            [PSCustomObject]@{
                DiskNumber = [uint32]$num
                FriendlyName = $disk.FriendlyName
                Model = $disk.Model
                MediaType = switch ($disk.MediaType) {
                    3 { "HDD" }
                    4 { "SSD" }
                    5 { "SCM" }
                    default { "Unspecified" }
                }
                BusType = switch ($disk.BusType) {
                    7 { "USB" }
                    11 { "SATA" }
                    17 { "NVMe" }
                    default { "Other" }
                }
                Size = [uint64]$disk.Size
                HealthStatus = switch ($disk.HealthStatus) {
                    0 { "Healthy" }
                    1 { "Warning" }
                    2 { "Unhealthy" }
                    default { "Unknown" }
                }
                OperationalStatus = switch ($disk.OperationalStatus) {
                    1 { "None" }
                    2 { "OK" }
                    3 { "Degraded" }
                    4 { "Stressed" }
                    5 { "Predictive Failure" }
                    default { "Unknown" }
                }
                Temperature = if ($temp -ne $null) { [float]$temp } else { $null }
                Partitions = @()
            }
        }

        $lInfo = foreach ($drive in $logical) {
            [PSCustomObject]@{
                Letter = $drive.DeviceID
                Name = $drive.VolumeName
                DriveType = "Local Disk"
                FileSystem = $drive.FileSystem
                Total = [uint64]$drive.Size
                Free = [uint64]$drive.FreeSpace
                Used = [uint64]($drive.Size - $drive.FreeSpace)
            }
        }

        [PSCustomObject]@{
            PhysicalDisks = if ($pInfo) { $pInfo } else { @() }
            LogicalDrives = if ($lInfo) { $lInfo } else { @() }
        } | ConvertTo-Json -Depth 4
    } catch {
        "{ `"PhysicalDisks`": [], `"LogicalDrives`": [] }"
    }
"#;

const DISK_IO_SCRIPT: &str = r#"
    try {
        $perfRaw = Get-CimInstance Win32_PerfFormattedData_PerfDisk_PhysicalDisk -ErrorAction Stop |
            Where-Object { $_.Name -ne '_Total' }

        $result = foreach ($disk in $perfRaw) {
            $numMatch = [regex]::Match($disk.Name, '^\d+')
            $num = if ($numMatch.Success) { [uint32]$numMatch.Value } else { 0 }

            [PSCustomObject]@{
                DiskNumber = $num
                ReadSpeed = [double]$disk.DiskReadBytesPersec / 1MB
                WriteSpeed = [double]$disk.DiskWriteBytesPersec / 1MB
                ReadIops = [double]$disk.DiskReadsPersec
                WriteIops = [double]$disk.DiskWritesPersec
                QueueDepth = [double]$disk.CurrentDiskQueueLength
                AvgResponseTime = [double]$disk.AvgDisksecPerTransfer * 1000
                ActiveTime = [double]$disk.PercentDiskTime
            }
        }
        $result | ConvertTo-Json
    } catch {
        "[]"
    }
"#;

const DISK_PROCESS_SCRIPT: &str = r#"
    try {
        $perfProc = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -ErrorAction Stop |
            Where-Object { $_.IDProcess -ne 0 -and $_.Name -ne '_Total' -and $_.Name -ne 'Idle' } |
            Where-Object { $_.IODataOperationsPersec -gt 0 -or $_.IODataBytesPersec -gt 0 } |
            Sort-Object IODataBytesPersec -Descending |
            Select-Object -First 10

        $result = foreach ($entry in $perfProc) {
            $proc = Get-Process -Id $entry.IDProcess -ErrorAction SilentlyContinue
            [PSCustomObject]@{
                ProcessName = if ($proc) { $proc.ProcessName } else { $entry.Name }
                Pid = [uint32]$entry.IDProcess
                IoBytesPerSec = [double]$entry.IODataBytesPersec
                ReadBytesPerSec = [double]$entry.IOReadBytesPersec
                WriteBytesPerSec = [double]$entry.IOWriteBytesPersec
            }
        }

        $result | ConvertTo-Json
    } catch {
        "[]"
    }
"#;

impl WindowsDiskMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            ps,
            io_history: Mutex::new(HashMap::new()),
        })
    }

    fn parse_disk_info(output: &str) -> Result<DiskInfoParsed> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        serde_json::from_str(trimmed).context("Failed to parse disk info")
    }

    fn parse_disk_io_stats(output: &str) -> Result<Vec<DiskIOStats>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let stats: Vec<DiskIOSample> = if trimmed.starts_with('[') {
            parse_json_array(trimmed).context("Failed to parse disk IO stats array")?
        } else {
            let single: DiskIOSample =
                serde_json::from_str(trimmed).context("Failed to parse single disk IO stat")?;
            vec![single]
        };

        Ok(stats
            .into_iter()
            .map(|s| DiskIOStats {
                disk_number: s.DiskNumber,
                read_speed: s.ReadSpeed,
                write_speed: s.WriteSpeed,
                read_iops: s.ReadIops,
                write_iops: s.WriteIops,
                queue_depth: s.QueueDepth,
                avg_response_time: s.AvgResponseTime,
                active_time: s.ActiveTime.min(100.0),
            })
            .collect())
    }

    fn parse_disk_process_activity(output: &str) -> Result<Vec<DiskProcessActivity>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let activity: Vec<DiskProcessSample> = if trimmed.starts_with('[') {
            parse_json_array(trimmed).context("Failed to parse disk process activity array")?
        } else {
            let single: DiskProcessSample = serde_json::from_str(trimmed)
                .context("Failed to parse single disk process activity")?;
            vec![single]
        };

        Ok(activity
            .into_iter()
            .map(|s| DiskProcessActivity {
                process_name: s.ProcessName,
                pid: s.Pid,
                io_bytes_per_sec: s.IoBytesPerSec,
                read_bytes_per_sec: s.ReadBytesPerSec,
                write_bytes_per_sec: s.WriteBytesPerSec,
            })
            .collect())
    }
}

impl DiskMonitorTrait for WindowsDiskMonitor {
    async fn collect_data(&self) -> Result<DiskData> {
        let outputs = self
            .ps
            .execute_batch(&[DISK_INFO_SCRIPT, DISK_IO_SCRIPT, DISK_PROCESS_SCRIPT])
            .await
            .context("Failed to execute disk monitor batch")?;

        let info = Self::parse_disk_info(&outputs[0])?;
        let io_stats = Self::parse_disk_io_stats(&outputs[1])?;
        let process_activity = Self::parse_disk_process_activity(&outputs[2])?;

        let physical_disks: Vec<PhysicalDiskInfo> = info
            .PhysicalDisks
            .into_iter()
            .map(|p| PhysicalDiskInfo {
                disk_number: p.DiskNumber,
                friendly_name: p.FriendlyName.unwrap_or_else(|| "Unknown".to_string()),
                model: p.Model.unwrap_or_else(|| "Unknown".to_string()),
                media_type: p.MediaType.unwrap_or_else(|| "Unspecified".to_string()),
                bus_type: p.BusType.unwrap_or_else(|| "Other".to_string()),
                size: p.Size.unwrap_or(0),
                health_status: p.HealthStatus.unwrap_or_else(|| "Unknown".to_string()),
                operational_status: p.OperationalStatus.unwrap_or_else(|| "Unknown".to_string()),
                temperature: p.Temperature,
                write_cache_enabled: false,
                power_on_hours: None,
                tbw: None,
                wear_level: None,
                partitions: p.Partitions,
            })
            .collect();

        // Try mapping partitions (logical drives) to physical disks if possible.
        // It's tricky in PS without explicitly matching partitions to disks.
        // We'll leave `disk_number` in `DriveInfo` None for now. It would need
        // Get-Partition -DiskNumber and Get-Volume.
        let logical_drives: Vec<DriveInfo> = info
            .LogicalDrives
            .into_iter()
            .map(|l| DriveInfo {
                letter: l.Letter,
                name: l.Name.unwrap_or_default(),
                drive_type: l.DriveType.unwrap_or_else(|| "Local Disk".to_string()),
                file_system: l.FileSystem.unwrap_or_default(),
                total: l.Total.unwrap_or(0),
                used: l.Used.unwrap_or(0),
                free: l.Free.unwrap_or(0),
                disk_number: None,
            })
            .collect();

        // Update history
        let mut history = self.io_history.lock();
        let mut io_history = Vec::new();

        for stat in &io_stats {
            let hist = history
                .entry(stat.disk_number)
                .or_insert_with(|| DiskIOHistory {
                    disk_number: stat.disk_number,
                    read_history: VecDeque::with_capacity(60),
                    write_history: VecDeque::with_capacity(60),
                    iops_history: VecDeque::with_capacity(60),
                });

            if hist.read_history.len() >= 60 {
                hist.read_history.pop_front();
                hist.write_history.pop_front();
                hist.iops_history.pop_front();
            }

            hist.read_history.push_back(stat.read_speed);
            hist.write_history.push_back(stat.write_speed);
            hist.iops_history
                .push_back(stat.read_iops + stat.write_iops);

            io_history.push(hist.clone());
        }

        Ok(DiskData {
            physical_disks,
            logical_drives,
            io_stats,
            process_activity,
            io_history,
        })
    }
}

// -----------------------------------------------------------------------------
// PowerShell JSON structures
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct DiskInfoParsed {
    #[serde(default)]
    PhysicalDisks: Vec<PhysicalDiskSample>,
    #[serde(default)]
    LogicalDrives: Vec<LogicalDriveSample>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct PhysicalDiskSample {
    DiskNumber: u32,
    FriendlyName: Option<String>,
    Model: Option<String>,
    MediaType: Option<String>,
    BusType: Option<String>,
    Size: Option<u64>,
    HealthStatus: Option<String>,
    OperationalStatus: Option<String>,
    Temperature: Option<f32>,
    #[serde(default)]
    Partitions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct LogicalDriveSample {
    Letter: String,
    Name: Option<String>,
    DriveType: Option<String>,
    FileSystem: Option<String>,
    Total: Option<u64>,
    Free: Option<u64>,
    Used: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct DiskIOSample {
    DiskNumber: u32,
    ReadSpeed: f64,
    WriteSpeed: f64,
    ReadIops: f64,
    WriteIops: f64,
    QueueDepth: f64,
    AvgResponseTime: f64,
    ActiveTime: f64,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct DiskProcessSample {
    ProcessName: String,
    Pid: u32,
    IoBytesPerSec: f64,
    ReadBytesPerSec: f64,
    WriteBytesPerSec: f64,
}
