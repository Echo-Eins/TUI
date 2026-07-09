use crate::integrations::LinuxSysMonitor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use crate::platform::linux::BlockDeviceInfo;
use anyhow::Result;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
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
        let block_devices = self.linux_sys.get_block_devices()?;
        let mounts = self.linux_sys.get_mounts()?;
        let mut physical_disks = self.get_physical_disks(&block_devices)?;
        let (logical_drives, volume_disks) =
            self.get_logical_drives(&mounts, &block_devices, &physical_disks);
        apply_volume_usage(&mut physical_disks, &logical_drives, &volume_disks);

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
    fn get_logical_drives(
        &self,
        mounts: &[crate::platform::linux::disk::MountInfo],
        block_devices: &[BlockDeviceInfo],
        physical_disks: &[PhysicalDiskInfo],
    ) -> (Vec<DriveInfo>, HashMap<usize, Vec<u32>>) {
        let block_by_name: HashMap<&str, &BlockDeviceInfo> = block_devices
            .iter()
            .map(|device| (device.name.as_str(), device))
            .collect();
        let disk_number_by_name: HashMap<&str, u32> = physical_disks
            .iter()
            .map(|disk| (disk.friendly_name.as_str(), disk.disk_number))
            .collect();
        let mut drives = Vec::new();
        let mut drive_index_by_key = HashMap::new();
        let mut drive_disks: HashMap<usize, Vec<u32>> = HashMap::new();

        for mount in mounts {
            if should_skip_mount(&mount) {
                continue;
            }
            let Ok(space) = self.linux_sys.get_disk_space(&mount.mount_point) else {
                continue;
            };
            let source_name = block_name_for_mount(mount);
            let uuid = source_name
                .as_deref()
                .and_then(|name| block_by_name.get(name))
                .and_then(|device| device.uuid.clone());
            let volume_key = uuid
                .as_ref()
                .filter(|uuid| !uuid.is_empty())
                .map(|uuid| format!("uuid:{uuid}"))
                .unwrap_or_else(|| format!("dev:{}:{}", mount.major_minor, mount.fs_type));

            let disk_numbers = physical_disk_numbers_for_volume(
                source_name.as_deref(),
                uuid.as_deref(),
                block_devices,
                &block_by_name,
                &disk_number_by_name,
            );

            if let Some(index) = drive_index_by_key.get(&volume_key).copied() {
                let drive: &mut DriveInfo = &mut drives[index];
                if !drive.mount_points.contains(&mount.mount_point) {
                    drive.mount_points.push(mount.mount_point.clone());
                }
                if !drive
                    .mount_details
                    .iter()
                    .any(|details| details.path == mount.mount_point)
                {
                    drive.mount_details.push(MountPointInfo {
                        path: mount.mount_point.clone(),
                        total: space.total_bytes,
                        used: space.used_bytes,
                        free: space.free_bytes,
                    });
                }
                if mount.mount_point == "/" {
                    drive.letter = mount.mount_point.clone();
                    drive.name = volume_name(
                        mount,
                        block_by_name
                            .get(source_name.as_deref().unwrap_or_default())
                            .copied(),
                    );
                }
                merge_disk_numbers(&mut drive_disks, index, disk_numbers);
                drive.disk_number = drive_disks
                    .get(&index)
                    .filter(|disks| disks.len() == 1)
                    .map(|disks| disks[0]);
                continue;
            }

            let index = drives.len();
            drive_index_by_key.insert(volume_key, index);
            let disk_number = (disk_numbers.len() == 1).then_some(disk_numbers[0]);
            drive_disks.insert(index, disk_numbers);
            drives.push(DriveInfo {
                letter: mount.mount_point.clone(),
                name: volume_name(
                    mount,
                    block_by_name
                        .get(source_name.as_deref().unwrap_or_default())
                        .copied(),
                ),
                source: normalized_mount_source(&mount.device),
                uuid,
                mount_points: vec![mount.mount_point.clone()],
                mount_details: vec![MountPointInfo {
                    path: mount.mount_point.clone(),
                    total: space.total_bytes,
                    used: space.used_bytes,
                    free: space.free_bytes,
                }],
                drive_type: "Local filesystem".to_string(),
                file_system: mount.fs_type.clone(),
                total: space.total_bytes,
                used: space.used_bytes,
                free: space.free_bytes,
                disk_number,
            });
        }

        let mut paired: Vec<_> = drives
            .into_iter()
            .enumerate()
            .map(|(index, drive)| {
                let disks = drive_disks.remove(&index).unwrap_or_default();
                (drive, disks)
            })
            .collect();
        paired.sort_by(|(left, _), (right, _)| mount_sort_key(left).cmp(&mount_sort_key(right)));

        let mut sorted_drives = Vec::with_capacity(paired.len());
        let mut sorted_disks = HashMap::with_capacity(paired.len());
        for (index, (drive, disks)) in paired.into_iter().enumerate() {
            sorted_drives.push(drive);
            sorted_disks.insert(index, disks);
        }
        (sorted_drives, sorted_disks)
    }

    fn get_physical_disks(
        &self,
        block_devices: &[BlockDeviceInfo],
    ) -> Result<Vec<PhysicalDiskInfo>> {
        let mut disks = Vec::new();
        let temps = self.linux_sys.get_disk_temperatures();

        let mut physical_devices: Vec<_> = block_devices
            .iter()
            .filter(|device| is_physical_disk(device))
            .collect();
        physical_devices.sort_by(|a, b| a.name.cmp(&b.name));

        for (i, dev) in physical_devices.into_iter().enumerate() {
            let smart = self.linux_sys.get_smart_data(&dev.name);
            let temperature = temps
                .get(&dev.name)
                .copied()
                .or_else(|| smart.as_ref().and_then(|s| s.temperature));

            let media_type = if dev.removable {
                "Removable"
            } else if dev.transport.eq_ignore_ascii_case("nvme") || dev.name.starts_with("nvme") {
                "NVMe"
            } else if dev.rota {
                "HDD"
            } else {
                "SSD"
            }
            .to_string();
            let bus_type = if dev.transport.is_empty() {
                infer_bus_type(&dev.name).to_string()
            } else {
                dev.transport.to_uppercase()
            };

            disks.push(PhysicalDiskInfo {
                disk_number: i as u32,
                friendly_name: dev.name.clone(),
                device_path: dev.path.clone(),
                model: dev
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|model| !model.is_empty())
                    .unwrap_or(&dev.name)
                    .to_string(),
                media_type,
                bus_type,
                size: dev.size,
                filesystem_total: 0,
                filesystem_used: 0,
                filesystem_available: 0,
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
                        let queue_depth = (weighted_time_io as f64 / 1000.0) / elapsed;

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
            | "nfs"
            | "nfs4"
            | "cifs"
            | "smb3"
    ) || device.starts_with("tmpfs")
        || device.starts_with("proc")
        || device.starts_with("sysfs")
        || mount_point.starts_with("/proc")
        || mount_point.starts_with("/sys")
        || (mount_point.starts_with("/run")
            && !mount_point.starts_with("/run/media/")
            && mount_point != "/run/media")
        || mount_point.starts_with("/dev")
        || mount_point.starts_with("/snap")
        || mount_point.starts_with("/var/lib/docker/")
}

fn normalized_mount_source(source: &str) -> String {
    source
        .split_once('[')
        .map(|(device, _)| device)
        .unwrap_or(source)
        .to_string()
}

fn block_name_for_mount(mount: &crate::platform::linux::disk::MountInfo) -> Option<String> {
    if let Some(name) = fs::read_link(format!("/sys/dev/block/{}", mount.major_minor))
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_owned()))
        .and_then(|name| name.to_str().map(str::to_string))
    {
        return Some(name);
    }

    let source = normalized_mount_source(&mount.device);
    if let Some(name) = source.strip_prefix("/dev/") {
        let canonical = fs::canonicalize(&source).unwrap_or_else(|_| source.clone().into());
        return canonical
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .or_else(|| Some(name.trim_start_matches("mapper/").to_string()));
    }
    None
}

fn physical_disk_numbers_for_volume(
    source_name: Option<&str>,
    uuid: Option<&str>,
    block_devices: &[BlockDeviceInfo],
    block_by_name: &HashMap<&str, &BlockDeviceInfo>,
    disk_number_by_name: &HashMap<&str, u32>,
) -> Vec<u32> {
    let mut source_names = Vec::new();
    if let Some(source_name) = source_name {
        source_names.push(source_name);
    }
    if let Some(uuid) = uuid.filter(|uuid| !uuid.is_empty()) {
        source_names.extend(
            block_devices
                .iter()
                .filter(|device| device.uuid.as_deref() == Some(uuid))
                .map(|device| device.name.as_str()),
        );
    }

    let mut disk_numbers = HashSet::new();
    for source_name in source_names {
        for physical_name in root_physical_devices(source_name, block_by_name) {
            if let Some(disk_number) = disk_number_by_name.get(physical_name.as_str()) {
                disk_numbers.insert(*disk_number);
            }
        }
    }
    let mut disk_numbers: Vec<_> = disk_numbers.into_iter().collect();
    disk_numbers.sort_unstable();
    disk_numbers
}

fn root_physical_devices(
    source_name: &str,
    block_by_name: &HashMap<&str, &BlockDeviceInfo>,
) -> Vec<String> {
    let mut queue = VecDeque::from([source_name.to_string()]);
    let mut visited = HashSet::new();
    let mut roots = HashSet::new();

    while let Some(name) = queue.pop_front() {
        if !visited.insert(name.clone()) {
            continue;
        }
        if let Some(device) = block_by_name.get(name.as_str()) {
            if is_physical_disk(device) {
                roots.insert(name.clone());
                continue;
            }
            if let Some(parent) = device.parent.as_ref() {
                queue.push_back(parent.clone());
            }
        }

        if let Ok(slaves) = fs::read_dir(format!("/sys/class/block/{name}/slaves")) {
            for slave in slaves.flatten() {
                if let Some(slave_name) = slave.file_name().to_str() {
                    queue.push_back(slave_name.to_string());
                }
            }
        }
    }

    let mut roots: Vec<_> = roots.into_iter().collect();
    roots.sort();
    roots
}

fn is_physical_disk(device: &BlockDeviceInfo) -> bool {
    if device.dev_type != "disk"
        || device.size == 0
        || device.name.starts_with("loop")
        || device.name.starts_with("ram")
        || device.name.starts_with("zram")
        || device.name.starts_with("dm-")
        || device.name.starts_with("md")
        || device.name.starts_with("nbd")
    {
        return false;
    }

    fs::canonicalize(format!("/sys/class/block/{}", device.name))
        .map(|path| !path.to_string_lossy().contains("/devices/virtual/"))
        .unwrap_or(true)
}

fn infer_bus_type(name: &str) -> &'static str {
    if name.starts_with("nvme") {
        "NVME"
    } else if name.starts_with("mmcblk") {
        "MMC"
    } else if name.starts_with("sd") {
        "SCSI/SATA"
    } else if name.starts_with("vd") {
        "VIRTIO"
    } else {
        "UNKNOWN"
    }
}

fn volume_name(
    mount: &crate::platform::linux::disk::MountInfo,
    block_device: Option<&BlockDeviceInfo>,
) -> String {
    if mount.mount_point == "/" {
        return "Root".to_string();
    }
    block_device
        .and_then(|device| device.label.as_deref())
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .or_else(|| {
            mount
                .mount_point
                .rsplit('/')
                .find(|component| !component.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| mount.device.clone())
}

fn merge_disk_numbers(
    volume_disks: &mut HashMap<usize, Vec<u32>>,
    volume_index: usize,
    disk_numbers: Vec<u32>,
) {
    let entry = volume_disks.entry(volume_index).or_default();
    for disk_number in disk_numbers {
        if !entry.contains(&disk_number) {
            entry.push(disk_number);
        }
    }
    entry.sort_unstable();
}

fn mount_sort_key(drive: &DriveInfo) -> (bool, &str) {
    (drive.letter != "/", drive.letter.as_str())
}

fn apply_volume_usage(
    physical_disks: &mut [PhysicalDiskInfo],
    drives: &[DriveInfo],
    volume_disks: &HashMap<usize, Vec<u32>>,
) {
    for disk in physical_disks.iter_mut() {
        disk.filesystem_total = 0;
        disk.filesystem_used = 0;
        disk.filesystem_available = 0;
        disk.partitions.clear();
    }

    for (index, drive) in drives.iter().enumerate() {
        for disk_number in volume_disks.get(&index).into_iter().flatten() {
            let Some(disk) = physical_disks
                .iter_mut()
                .find(|disk| disk.disk_number == *disk_number)
            else {
                continue;
            };
            disk.filesystem_total = disk.filesystem_total.saturating_add(drive.total);
            disk.filesystem_used = disk.filesystem_used.saturating_add(drive.used);
            disk.filesystem_available = disk.filesystem_available.saturating_add(drive.free);
            for mount_point in &drive.mount_points {
                if !disk.partitions.contains(mount_point) {
                    disk.partitions.push(mount_point.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device(name: &str, dev_type: &str, parent: Option<&str>) -> BlockDeviceInfo {
        BlockDeviceInfo {
            name: name.to_string(),
            path: format!("/dev/{name}"),
            model: Some(name.to_string()),
            dev_type: dev_type.to_string(),
            size: 1_000,
            rota: false,
            transport: String::new(),
            filesystem: None,
            mount_point: None,
            parent: parent.map(str::to_string),
            serial: None,
            label: None,
            uuid: None,
            removable: false,
            hotplug: false,
        }
    }

    #[test]
    fn excludes_virtual_block_disks() {
        assert!(!is_physical_disk(&device("zram0", "disk", None)));
        assert!(!is_physical_disk(&device("loop0", "disk", None)));
        assert!(is_physical_disk(&device("testdisk", "disk", None)));
    }

    #[test]
    fn resolves_partition_through_mapper_to_physical_disk() {
        let devices = [
            device("nvme0n1", "disk", None),
            device("nvme0n1p3", "part", Some("nvme0n1")),
            device("dm-0", "crypt", Some("nvme0n1p3")),
        ];
        let by_name = devices
            .iter()
            .map(|device| (device.name.as_str(), device))
            .collect();
        assert_eq!(
            root_physical_devices("dm-0", &by_name),
            vec!["nvme0n1".to_string()]
        );
    }

    #[test]
    fn aggregates_each_unique_volume_once() {
        let mut disks = vec![PhysicalDiskInfo {
            disk_number: 0,
            friendly_name: "nvme0n1".to_string(),
            device_path: "/dev/nvme0n1".to_string(),
            model: "NVMe".to_string(),
            media_type: "NVMe".to_string(),
            bus_type: "NVME".to_string(),
            size: 1_000,
            filesystem_total: 0,
            filesystem_used: 0,
            filesystem_available: 0,
            health_status: "Healthy".to_string(),
            operational_status: "OK".to_string(),
            temperature: None,
            write_cache_enabled: false,
            power_on_hours: None,
            tbw: None,
            wear_level: None,
            partitions: Vec::new(),
        }];
        let drives = vec![DriveInfo {
            letter: "/".to_string(),
            name: "Root".to_string(),
            source: "/dev/nvme0n1p3".to_string(),
            uuid: Some("uuid".to_string()),
            mount_points: vec!["/".to_string(), "/home".to_string()],
            mount_details: vec![
                MountPointInfo {
                    path: "/".to_string(),
                    total: 900,
                    used: 400,
                    free: 500,
                },
                MountPointInfo {
                    path: "/home".to_string(),
                    total: 900,
                    used: 400,
                    free: 500,
                },
            ],
            drive_type: "Local filesystem".to_string(),
            file_system: "btrfs".to_string(),
            total: 900,
            used: 400,
            free: 500,
            disk_number: Some(0),
        }];
        apply_volume_usage(&mut disks, &drives, &HashMap::from([(0, vec![0])]));
        assert_eq!(disks[0].filesystem_total, 900);
        assert_eq!(disks[0].filesystem_used, 400);
        assert_eq!(disks[0].partitions, vec!["/", "/home"]);
    }

    #[test]
    fn keeps_desktop_automounted_removable_filesystems() {
        let removable = crate::platform::linux::disk::MountInfo {
            mount_point: "/run/media/user/USB".to_string(),
            device: "/dev/sda1".to_string(),
            fs_type: "exfat".to_string(),
            major_minor: "8:1".to_string(),
            fs_root: "/".to_string(),
        };
        let runtime = crate::platform::linux::disk::MountInfo {
            mount_point: "/run/user/1000".to_string(),
            device: "tmpfs".to_string(),
            fs_type: "tmpfs".to_string(),
            major_minor: "0:42".to_string(),
            fs_root: "/".to_string(),
        };

        assert!(!should_skip_mount(&removable));
        assert!(should_skip_mount(&runtime));
    }
}
