use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskData {
    pub physical_disks: Vec<PhysicalDiskInfo>,
    pub logical_drives: Vec<DriveInfo>,
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
}

impl DiskMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    pub async fn collect_data(&self) -> Result<DiskData> {
        let physical_disks = self.get_physical_disks().await?;
        let logical_drives = self.get_logical_drives().await?;

        Ok(DiskData {
            physical_disks,
            logical_drives,
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
