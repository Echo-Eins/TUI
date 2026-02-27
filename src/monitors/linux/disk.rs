use crate::integrations::LinuxSysMonitor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use anyhow::Result;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

pub struct LinuxDiskMonitor {
    linux_sys: LinuxSysMonitor,
    io_history: Mutex<HashMap<u32, DiskIOHistory>>,
    last_io_stats: Mutex<Option<(Instant, HashMap<String, crate::integrations::RawDiskStats>)>>,
    last_process_io: Mutex<Option<(Instant, HashMap<u32, crate::integrations::ProcessIoSample>)>>,
}

impl LinuxDiskMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            linux_sys: LinuxSysMonitor::new(),
            io_history: Mutex::new(HashMap::new()),
            last_io_stats: Mutex::new(None),
            last_process_io: Mutex::new(None),
        })
    }
}

impl DiskMonitorTrait for LinuxDiskMonitor {
    async fn collect_data(&self) -> Result<DiskData> {
        let mut logical_drives = self.get_logical_drives()?;
        let mut physical_disks = self.get_physical_disks()?;
        let mut disk_number_by_name = HashMap::new();
        for disk in &physical_disks {
            disk_number_by_name.insert(disk.friendly_name.clone(), disk.disk_number);
        }

        let mounts = self.linux_sys.get_mounts()?;
        for drive in &mut logical_drives {
            if let Some(mount) = mounts.iter().find(|m| m.mount_point == drive.letter) {
                if let Some(root_disk) = root_disk_name_from_device(&mount.device) {
                    if let Some(disk_number) = disk_number_by_name.get(&root_disk) {
                        drive.disk_number = Some(*disk_number);
                    }
                }
            }
        }

        for mount in &mounts {
            if mount.device.starts_with("/dev/") {
                let Some(disk_name) = root_disk_name_from_device(&mount.device) else {
                    continue;
                };
                if let Some(disk) = physical_disks
                    .iter_mut()
                    .find(|d| d.friendly_name == disk_name.as_str())
                {
                    if !disk.partitions.contains(&mount.mount_point) {
                        disk.partitions.push(mount.mount_point.clone());
                    }
                }
            }
        }

        let io_stats = self.get_io_stats(&physical_disks)?;
        let process_activity = self.get_process_activity()?;

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

impl LinuxDiskMonitor {
    fn get_logical_drives(&self) -> Result<Vec<DriveInfo>> {
        let mounts = self.linux_sys.get_mounts()?;
        let mut drives = Vec::new();

        for mount in mounts {
            if should_skip_mount(&mount) {
                continue;
            }
            if let Ok(space) = self.linux_sys.get_disk_space(&mount.mount_point) {
                let name = if mount.mount_point == "/" {
                    "Root".to_string()
                } else {
                    mount
                        .mount_point
                        .split('/')
                        .last()
                        .unwrap_or(&mount.mount_point)
                        .to_string()
                };

                drives.push(DriveInfo {
                    letter: mount.mount_point.clone(),
                    name,
                    drive_type: mount.fs_type.clone(),
                    file_system: mount.fs_type,
                    total: space.total_bytes,
                    used: space.used_bytes,
                    free: space.free_bytes,
                    disk_number: None,
                });
            }
        }

        Ok(drives)
    }

    fn get_physical_disks(&self) -> Result<Vec<PhysicalDiskInfo>> {
        let block_devices = self.linux_sys.get_block_devices()?;
        let mut disks = Vec::new();
        let temps = self.linux_sys.get_disk_temperatures();

        for (i, dev) in block_devices
            .into_iter()
            .filter(|d| d.dev_type == "disk")
            .enumerate()
        {
            let smart = self.linux_sys.get_smart_data(&dev.name);
            let temperature = temps
                .get(&dev.name)
                .copied()
                .or_else(|| smart.as_ref().and_then(|s| s.temperature));

            let media_type = if dev.rota { "HDD" } else { "SSD" }.to_string();
            let bus_type = if dev.transport.is_empty() {
                "Unknown".to_string()
            } else {
                dev.transport.to_uppercase()
            };

            disks.push(PhysicalDiskInfo {
                disk_number: i as u32,
                friendly_name: dev.name,
                model: dev.model.unwrap_or_else(|| "Unknown".to_string()),
                media_type,
                bus_type,
                size: dev.size,
                health_status: smart
                    .as_ref()
                    .map(|s| s.health_status.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                operational_status: "OK".to_string(),
                temperature,
                write_cache_enabled: false,
                power_on_hours: smart.as_ref().and_then(|s| s.power_on_hours),
                tbw: None,
                wear_level: None,
                partitions: Vec::new(),
            });
        }

        Ok(disks)
    }

    fn get_io_stats(&self, disks: &[PhysicalDiskInfo]) -> Result<Vec<DiskIOStats>> {
        let now = Instant::now();
        let raw_stats = self.linux_sys.get_raw_disk_stats()?;

        let mut last_guard = self.last_io_stats.lock();
        let mut result = Vec::new();

        if let Some((last_time, last_stats)) = last_guard.as_ref() {
            let elapsed = now.saturating_duration_since(*last_time).as_secs_f64();
            if elapsed > 0.0 {
                for disk in disks {
                    if let (Some(curr), Some(prev)) = (
                        raw_stats.get(&disk.friendly_name),
                        last_stats.get(&disk.friendly_name),
                    ) {
                        let read_sectors = curr.sectors_read.saturating_sub(prev.sectors_read);
                        let write_sectors =
                            curr.sectors_written.saturating_sub(prev.sectors_written);
                        let reads = curr.reads_completed.saturating_sub(prev.reads_completed);
                        let writes = curr.writes_completed.saturating_sub(prev.writes_completed);
                        let time_io = curr.ms_doing_io.saturating_sub(prev.ms_doing_io);
                        let weighted_time_io = curr
                            .weighted_ms_doing_io
                            .saturating_sub(prev.weighted_ms_doing_io);
                        let read_time_ms = curr.ms_reading.saturating_sub(prev.ms_reading);
                        let write_time_ms = curr.ms_writing.saturating_sub(prev.ms_writing);

                        let read_bytes = read_sectors * 512;
                        let write_bytes = write_sectors * 512;

                        let mut active_time = (time_io as f64 / 1000.0) / elapsed * 100.0;
                        active_time = active_time.clamp(0.0, 100.0);
                        let avg_read_latency = if reads > 0 {
                            read_time_ms as f64 / reads as f64
                        } else {
                            0.0
                        };
                        let avg_write_latency = if writes > 0 {
                            write_time_ms as f64 / writes as f64
                        } else {
                            0.0
                        };
                        let total_ops = reads + writes;
                        let avg_response_time = if total_ops > 0 {
                            ((avg_read_latency * reads as f64)
                                + (avg_write_latency * writes as f64))
                                / total_ops as f64
                        } else {
                            0.0
                        };
                        let queue_depth = ((weighted_time_io as f64 / 1000.0) / elapsed)
                            .max(curr.io_in_progress as f64);

                        result.push(DiskIOStats {
                            disk_number: disk.disk_number,
                            read_speed: (read_bytes as f64 / elapsed) / 1_048_576.0,
                            write_speed: (write_bytes as f64 / elapsed) / 1_048_576.0,
                            read_iops: reads as f64 / elapsed,
                            write_iops: writes as f64 / elapsed,
                            queue_depth,
                            avg_response_time,
                            active_time,
                        });
                    } else {
                        result.push(DiskIOStats {
                            disk_number: disk.disk_number,
                            read_speed: 0.0,
                            write_speed: 0.0,
                            read_iops: 0.0,
                            write_iops: 0.0,
                            queue_depth: 0.0,
                            avg_response_time: 0.0,
                            active_time: 0.0,
                        });
                    }
                }
            }
        } else {
            for disk in disks {
                result.push(DiskIOStats {
                    disk_number: disk.disk_number,
                    read_speed: 0.0,
                    write_speed: 0.0,
                    read_iops: 0.0,
                    write_iops: 0.0,
                    queue_depth: 0.0,
                    avg_response_time: 0.0,
                    active_time: 0.0,
                });
            }
        }

        *last_guard = Some((now, raw_stats));
        Ok(result)
    }

    fn get_process_activity(&self) -> Result<Vec<DiskProcessActivity>> {
        let now = Instant::now();
        let curr_io = self.linux_sys.get_process_io()?;

        let mut last_guard = self.last_process_io.lock();
        let mut result = Vec::new();

        if let Some((last_time, last_io)) = last_guard.as_ref() {
            let elapsed = now.saturating_duration_since(*last_time).as_secs_f64();
            if elapsed > 0.0 {
                for (pid, curr) in &curr_io {
                    if let Some(prev) = last_io.get(pid) {
                        let read_bytes = curr.read_bytes.saturating_sub(prev.read_bytes);
                        let write_bytes = curr.write_bytes.saturating_sub(prev.write_bytes);

                        if read_bytes > 0 || write_bytes > 0 {
                            result.push(DiskProcessActivity {
                                process_name: curr.name.clone(),
                                pid: *pid,
                                io_bytes_per_sec: (read_bytes + write_bytes) as f64 / elapsed,
                                read_bytes_per_sec: read_bytes as f64 / elapsed,
                                write_bytes_per_sec: write_bytes as f64 / elapsed,
                            });
                        }
                    }
                }
            }
        }

        result.sort_by(|a, b| {
            b.io_bytes_per_sec
                .partial_cmp(&a.io_bytes_per_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        result.truncate(10);

        *last_guard = Some((now, curr_io));
        Ok(result)
    }
}

fn should_skip_mount(mount: &crate::platform::linux::disk::MountInfo) -> bool {
    let fs_type = mount.fs_type.as_str();
    let mount_point = mount.mount_point.as_str();
    let device = mount.device.as_str();

    matches!(
        fs_type,
        "proc"
            | "sysfs"
            | "devtmpfs"
            | "tmpfs"
            | "devpts"
            | "securityfs"
            | "cgroup"
            | "cgroup2"
            | "pstore"
            | "bpf"
            | "tracefs"
            | "debugfs"
            | "configfs"
            | "squashfs"
            | "fusectl"
            | "mqueue"
            | "hugetlbfs"
            | "overlay"
            | "nsfs"
            | "autofs"
            | "rpc_pipefs"
            | "binfmt_misc"
            | "ramfs"
            | "zram"
            | "fuse.lxcfs"
            | "fuse.portal"
            | "fuse.gvfsd-fuse"
            | "fuse.snapfuse"
            | "9p"
    ) || device.starts_with("tmpfs")
        || device.starts_with("proc")
        || device.starts_with("sysfs")
        || mount_point.starts_with("/proc")
        || mount_point.starts_with("/sys")
        || mount_point.starts_with("/run")
        || mount_point.starts_with("/dev")
        || mount_point.starts_with("/snap")
        || mount_point.starts_with("/var/lib/docker/")
}

fn root_disk_name_from_device(device: &str) -> Option<String> {
    if !device.starts_with("/dev/") {
        return None;
    }
    let name = device.trim_start_matches("/dev/");
    if name.is_empty() || name.starts_with("mapper/") || name.starts_with("dm-") {
        return None;
    }
    if name.starts_with("loop") || name.starts_with("ram") {
        return None;
    }

    if let Some((prefix, suffix)) = name.rsplit_once('p') {
        if (prefix.starts_with("nvme") || prefix.starts_with("mmcblk"))
            && suffix.chars().all(|c| c.is_ascii_digit())
        {
            return Some(prefix.to_string());
        }
    }

    let mut chars: Vec<char> = name.chars().collect();
    while chars.last().copied().is_some_and(|c| c.is_ascii_digit()) {
        chars.pop();
    }
    if chars.is_empty() {
        return None;
    }
    Some(chars.into_iter().collect())
}
