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

        // Link logical drives to physical disks using device names from DiskInfo.
        // DiskInfo.name is e.g. "/dev/sda1" or "/dev/nvme0n1p2".
        // BlockDeviceInfo.name (physical) is e.g. "sda" or "nvme0n1".
        let disk_info = self.linux_sys.get_disk_info()?;
        for d in &disk_info {
            let dev_name = d.name.trim_start_matches("/dev/");
            // Strip partition suffix: sda1 -> sda, nvme0n1p2 -> nvme0n1
            let disk_name = if dev_name.contains("nvme") || dev_name.contains("mmc") {
                dev_name.rsplit_once('p')
                    .filter(|(base, suffix)| {
                        base.ends_with(|c: char| c.is_ascii_digit())
                            && suffix.chars().all(|c| c.is_ascii_digit())
                    })
                    .map(|(base, _)| base)
                    .unwrap_or(dev_name)
            } else {
                dev_name.trim_end_matches(|c: char| c.is_ascii_digit())
            };

            if let Some(phys) = physical_disks.iter().find(|p| p.friendly_name == disk_name) {
                let disk_num = phys.disk_number;
                if let Some(drive) = logical_drives.iter_mut().find(|l| l.letter == d.mount_point) {
                    drive.disk_number = Some(disk_num);
                }
            }

            if let Some(phys) = physical_disks.iter_mut().find(|p| p.friendly_name == disk_name) {
                if !phys.partitions.contains(&d.mount_point) {
                    phys.partitions.push(d.mount_point.clone());
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
            hist.iops_history.push_back(stat.read_iops + stat.write_iops);

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
        let disk_info = self.linux_sys.get_disk_info()?;
        let mut drives = Vec::new();

        for d in disk_info {
            if !d.is_primary_mount {
                continue; // Skip btrfs subvolume duplicates
            }
            let name = if d.mount_point == "/" {
                "Root".to_string()
            } else {
                d.mount_point.split('/').last().unwrap_or(&d.mount_point).to_string()
            };

            drives.push(DriveInfo {
                letter: d.mount_point,
                name,
                drive_type: d.fs_type.clone(),
                file_system: d.fs_type,
                total: d.total,
                used: d.used,
                free: d.available,
                disk_number: None, // linked in collect_data
            });
        }

        Ok(drives)
    }

    fn get_physical_disks(&self) -> Result<Vec<PhysicalDiskInfo>> {
        let block_devices = self.linux_sys.get_block_devices()?;
        let mut disks = Vec::new();

        // Only actual disks, not partitions
        let disk_devices: Vec<_> = block_devices.iter()
            .filter(|d| d.dev_type == "disk")
            .collect();

        for (i, dev) in disk_devices.iter().enumerate() {
            let smart = self.linux_sys.get_smart_data(&dev.name);
            let temperature = smart.as_ref().and_then(|s| s.temperature);

            let media_type = if dev.rotational {
                "HDD".to_string()
            } else if dev.transport == "nvme" || dev.name.starts_with("nvme") {
                "NVMe SSD".to_string()
            } else {
                "SSD".to_string()
            };

            let bus_type = if dev.transport.is_empty() {
                if dev.name.starts_with("nvme") { "NVMe".to_string() }
                else if dev.name.starts_with("sd") { "SATA/SAS".to_string() }
                else { "Unknown".to_string() }
            } else {
                dev.transport.to_uppercase()
            };

            let model_str = if dev.model.trim().is_empty() {
                dev.name.clone()
            } else {
                dev.model.trim().to_string()
            };

            let health_status = smart.as_ref()
                .map(|s| s.health_status.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            disks.push(PhysicalDiskInfo {
                disk_number: i as u32,
                friendly_name: dev.name.clone(),
                model: model_str,
                media_type,
                bus_type,
                size: dev.size,
                health_status,
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
        // get_disk_stats() returns Vec<RawDiskStats> - convert to HashMap
        let raw_stats: HashMap<String, crate::integrations::RawDiskStats> =
            self.linux_sys.get_disk_stats()?
                .into_iter()
                .map(|s| (s.name.clone(), s))
                .collect();

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
                        let write_sectors = curr.sectors_written.saturating_sub(prev.sectors_written);
                        let reads = curr.reads_completed.saturating_sub(prev.reads_completed);
                        let writes = curr.writes_completed.saturating_sub(prev.writes_completed);
                        let time_io = curr.ms_doing_io.saturating_sub(prev.ms_doing_io);

                        let read_bytes = read_sectors * 512;
                        let write_bytes = write_sectors * 512;

                        let active_time = ((time_io as f64 / 1000.0) / elapsed * 100.0)
                            .clamp(0.0, 100.0);

                        result.push(DiskIOStats {
                            disk_number: disk.disk_number,
                            read_speed: (read_bytes as f64 / elapsed) / 1_048_576.0,
                            write_speed: (write_bytes as f64 / elapsed) / 1_048_576.0,
                            read_iops: reads as f64 / elapsed,
                            write_iops: writes as f64 / elapsed,
                            queue_depth: 0.0,
                            avg_response_time: 0.0,
                            active_time,
                        });
                    }
                }
            }
        }

        *last_guard = Some((now, raw_stats));
        Ok(result)
    }

    fn get_process_activity(&self) -> Result<Vec<DiskProcessActivity>> {
        let now = Instant::now();
        // get_process_io_samples() returns Vec<ProcessIoSample> - convert to HashMap
        let curr_io: HashMap<u32, crate::integrations::ProcessIoSample> =
            self.linux_sys.get_process_io_samples()?
                .into_iter()
                .map(|s| (s.pid, s))
                .collect();

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
