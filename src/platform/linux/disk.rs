use super::LinuxSysMonitor;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::time::Instant;

impl LinuxSysMonitor {
    pub fn get_disk_info(&self) -> Result<Vec<DiskInfo>> {
        let output = Command::new("df")
            .args(["-B1", "-T"])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut disks = Vec::new();
        let mut seen_devices: HashMap<String, usize> = HashMap::new();

        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 7 {
                continue;
            }

            let filesystem = parts[0].to_string();
            let fs_type = parts[1].to_string();
            let total = parts[2].parse::<u64>().unwrap_or(0);
            let used = parts[3].parse::<u64>().unwrap_or(0);
            let available = parts[4].parse::<u64>().unwrap_or(0);
            let mount_point = parts[6].to_string();

            // Skip special/virtual filesystems
            if fs_type == "tmpfs"
                || fs_type == "devtmpfs"
                || fs_type == "squashfs"
                || fs_type == "overlay"
                || fs_type == "9p"
                || fs_type == "fuse.snapfuse"
                || mount_point.starts_with("/sys")
                || mount_point.starts_with("/proc")
                || mount_point.starts_with("/run")
                || mount_point.starts_with("/snap")
                || mount_point.starts_with("/dev/shm")
            {
                continue;
            }

            // For btrfs: same device can have multiple subvolumes showing
            // identical total/used/available. Track by device+total to deduplicate
            // the size, but keep all mount points.
            let device_key = format!("{}:{}", filesystem, total);

            disks.push(DiskInfo {
                name: filesystem.clone(),
                mount_point,
                total,
                used,
                available,
                fs_type: fs_type.clone(),
                is_primary_mount: !seen_devices.contains_key(&device_key),
            });

            seen_devices.entry(device_key).or_insert(disks.len() - 1);
        }

        Ok(disks)
    }

    pub fn get_block_devices(&self) -> Result<Vec<BlockDeviceInfo>> {
        let output = Command::new("lsblk")
            .args([
                "-b",
                "-J",
                "-o",
                "NAME,MODEL,TYPE,SIZE,ROTA,TRAN,FSTYPE,MOUNTPOINT,PKNAME,SERIAL,REV,LABEL,UUID",
            ])
            .output()
            .context("Failed to run lsblk")?;

        if !output.status.success() {
            anyhow::bail!("lsblk failed with status {}", output.status);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let root: LsblkRoot =
            serde_json::from_str(&stdout).context("Failed to parse lsblk json")?;

        let mut result = Vec::new();
        for device in root.blockdevices {
            Self::flatten_block_device(None, device, &mut result);
        }

        Ok(result)
    }

    fn flatten_block_device(
        parent: Option<String>,
        device: LsblkEntry,
        result: &mut Vec<BlockDeviceInfo>,
    ) {
        let name = device.name;
        let parent_name = device.pkname.or(parent);

        result.push(BlockDeviceInfo {
            name: name.clone(),
            model: device.model.unwrap_or_default(),
            dev_type: device.dev_type,
            size: device.size.unwrap_or(0),
            rotational: device.rota.unwrap_or(false),
            transport: device.tran.unwrap_or_default(),
            filesystem: device.fstype,
            mount_point: device.mountpoint,
            parent: parent_name.clone(),
            serial: device.serial,
            label: device.label,
            uuid: device.uuid,
        });

        if let Some(children) = device.children {
            for child in children {
                Self::flatten_block_device(Some(name.clone()), child, result);
            }
        }
    }

    /// Get btrfs-specific information if btrfs filesystem is present
    pub fn get_btrfs_info(&self) -> Vec<BtrfsInfo> {
        let mut results = Vec::new();

        // Try btrfs filesystem show
        let output = match Command::new("btrfs")
            .args(["filesystem", "show", "--raw"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return results,
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut current: Option<BtrfsInfo> = None;

        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Label:") {
                if let Some(fs) = current.take() {
                    results.push(fs);
                }

                let label = trimmed
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("none")
                    .trim_matches('\'')
                    .to_string();
                let uuid = trimmed
                    .split("uuid:")
                    .nth(1)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();

                current = Some(BtrfsInfo {
                    label: if label == "none" { String::new() } else { label },
                    uuid,
                    total_size: 0,
                    used: 0,
                    devices: Vec::new(),
                    subvolumes: Vec::new(),
                });
            } else if trimmed.starts_with("Total devices") {
                if let Some(ref mut fs) = current {
                    if let Some(size_str) = trimmed.split("size").nth(1) {
                        // Parse raw byte size
                        if let Ok(bytes) = size_str.trim().parse::<u64>() {
                            fs.total_size = bytes;
                        }
                    }
                }
            } else if trimmed.starts_with("devid") {
                if let Some(ref mut fs) = current {
                    let parts: Vec<&str> = trimmed.split_whitespace().collect();
                    // devid N size XXXX used YYYY path /dev/XXX
                    if let Some(path_idx) = parts.iter().position(|&p| p == "path") {
                        let device_path = parts.get(path_idx + 1).unwrap_or(&"").to_string();
                        fs.devices.push(device_path);
                    }
                }
            }
        }

        if let Some(fs) = current.take() {
            results.push(fs);
        }

        // Try to get subvolume info for each btrfs mount
        for info in &mut results {
            if let Some(mount) = self.find_btrfs_mount(&info.uuid) {
                if let Ok(output) = Command::new("btrfs")
                    .args(["subvolume", "list", "-a", &mount])
                    .output()
                {
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        for line in stdout.lines() {
                            // Format: ID xxx gen yyy top level zzz path <path>
                            if let Some(path_start) = line.find("path ") {
                                let subvol_path = line[path_start + 5..].trim().to_string();
                                info.subvolumes.push(subvol_path);
                            }
                        }
                    }
                }

                // Get usage info
                if let Ok(output) = Command::new("btrfs")
                    .args(["filesystem", "usage", "-b", &mount])
                    .output()
                {
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        for line in stdout.lines() {
                            let trimmed = line.trim();
                            if trimmed.starts_with("Used:") {
                                if let Some(val) = trimmed.split_whitespace().nth(1) {
                                    info.used = val.parse::<u64>().unwrap_or(0);
                                }
                            }
                        }
                    }
                }
            }
        }

        results
    }

    fn find_btrfs_mount(&self, uuid: &str) -> Option<String> {
        let content = fs::read_to_string("/proc/mounts").ok()?;
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && parts[2] == "btrfs" {
                // Check if this mount matches the UUID
                let dev = parts[0];
                // Try to read UUID from the device
                let uuid_path = format!("/dev/disk/by-uuid/{}", uuid);
                if let Ok(target) = fs::read_link(&uuid_path) {
                    let target_name = target.file_name()?.to_str()?;
                    if dev.contains(target_name) {
                        return Some(parts[1].to_string());
                    }
                }
                // Fallback: just return first btrfs mount
                return Some(parts[1].to_string());
            }
        }
        None
    }

    /// Get SMART data for a disk device using smartctl
    pub fn get_smart_data(&self, device_name: &str) -> Option<SmartData> {
        let device_path = format!("/dev/{}", device_name);
        let output = Command::new("smartctl")
            .args(["-a", "-j", &device_path])
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).ok()?;

        let temperature = json
            .get("temperature")
            .and_then(|t| t.get("current"))
            .and_then(|v| v.as_f64())
            .map(|t| t as f32);

        let power_on_hours = json
            .get("power_on_time")
            .and_then(|t| t.get("hours"))
            .and_then(|v| v.as_u64());

        let health = json
            .get("smart_status")
            .and_then(|s| s.get("passed"))
            .and_then(|v| v.as_bool())
            .map(|passed| if passed { "Healthy" } else { "Warning" }.to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        Some(SmartData {
            temperature,
            power_on_hours,
            health_status: health,
        })
    }

    pub fn get_disk_stats(&self) -> Result<Vec<RawDiskStats>> {
        let content = fs::read_to_string("/proc/diskstats")?;
        let mut stats = Vec::new();

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 14 {
                continue;
            }

            let name = parts[2].to_string();
            if name.starts_with("loop") || name.starts_with("ram") {
                continue;
            }

            stats.push(RawDiskStats {
                name,
                reads_completed: parts[3].parse().unwrap_or(0),
                sectors_read: parts[5].parse().unwrap_or(0),
                ms_reading: parts[6].parse().unwrap_or(0),
                writes_completed: parts[7].parse().unwrap_or(0),
                sectors_written: parts[9].parse().unwrap_or(0),
                ms_writing: parts[10].parse().unwrap_or(0),
                io_in_progress: parts[11].parse().unwrap_or(0),
                ms_doing_io: parts[12].parse().unwrap_or(0),
            });
        }

        Ok(stats)
    }

    pub fn get_process_io_samples(&self) -> Result<Vec<ProcessIoSample>> {
        let mut out = Vec::new();
        let now = Instant::now();

        for entry in fs::read_dir("/proc")? {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            let file_name = entry.file_name();
            let file_name = match file_name.to_str() {
                Some(v) => v,
                None => continue,
            };

            let pid = match file_name.parse::<u32>() {
                Ok(v) => v,
                Err(_) => continue,
            };

            let io_path = format!("/proc/{pid}/io");
            let io_content = match fs::read_to_string(io_path) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let mut read_bytes = 0u64;
            let mut write_bytes = 0u64;
            for line in io_content.lines() {
                if let Some(value) = line.strip_prefix("read_bytes:") {
                    read_bytes = value.trim().parse().unwrap_or(0);
                } else if let Some(value) = line.strip_prefix("write_bytes:") {
                    write_bytes = value.trim().parse().unwrap_or(0);
                }
            }

            let name = fs::read_to_string(format!("/proc/{pid}/comm"))
                .map(|v| v.trim().to_string())
                .unwrap_or_else(|_| "unknown".to_string());

            out.push(ProcessIoSample {
                pid,
                name,
                read_bytes,
                write_bytes,
                timestamp: now,
            });
        }

        Ok(out)
    }
}

#[derive(Debug)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub fs_type: String,
    pub is_primary_mount: bool,
}

#[derive(Debug, Clone)]
pub struct BlockDeviceInfo {
    pub name: String,
    pub model: String,
    pub dev_type: String,
    pub size: u64,
    pub rotational: bool,
    pub transport: String,
    pub filesystem: Option<String>,
    pub mount_point: Option<String>,
    pub parent: Option<String>,
    pub serial: Option<String>,
    pub label: Option<String>,
    pub uuid: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BtrfsInfo {
    pub label: String,
    pub uuid: String,
    pub total_size: u64,
    pub used: u64,
    pub devices: Vec<String>,
    pub subvolumes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SmartData {
    pub temperature: Option<f32>,
    pub power_on_hours: Option<u64>,
    pub health_status: String,
}

#[derive(Debug, Clone)]
pub struct RawDiskStats {
    pub name: String,
    pub reads_completed: u64,
    pub sectors_read: u64,
    pub ms_reading: u64,
    pub writes_completed: u64,
    pub sectors_written: u64,
    pub ms_writing: u64,
    pub io_in_progress: u64,
    pub ms_doing_io: u64,
}

#[derive(Debug, Clone)]
pub struct ProcessIoSample {
    pub pid: u32,
    pub name: String,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub timestamp: Instant,
}

#[derive(Debug, serde::Deserialize)]
struct LsblkRoot {
    blockdevices: Vec<LsblkEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct LsblkEntry {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "model")]
    model: Option<String>,
    #[serde(rename = "type")]
    dev_type: String,
    #[serde(rename = "size")]
    size: Option<u64>,
    #[serde(rename = "rota")]
    rota: Option<bool>,
    #[serde(rename = "tran")]
    tran: Option<String>,
    #[serde(rename = "fstype")]
    fstype: Option<String>,
    #[serde(rename = "mountpoint")]
    mountpoint: Option<String>,
    #[serde(rename = "pkname")]
    pkname: Option<String>,
    #[serde(rename = "children")]
    children: Option<Vec<LsblkEntry>>,
    #[serde(rename = "serial")]
    serial: Option<String>,
    #[serde(rename = "label")]
    label: Option<String>,
    #[serde(rename = "uuid")]
    uuid: Option<String>,
}
