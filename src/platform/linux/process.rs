use super::LinuxSysMonitor;
use anyhow::Result;
use chrono::{Local, TimeZone, Utc};
use std::collections::HashMap;
use std::fs;

impl LinuxSysMonitor {
    pub fn get_clock_ticks_per_second(&self) -> u64 {
        clock_ticks_per_second().unwrap_or(100)
    }

    pub fn get_processes(&self) -> Result<Vec<ProcessInfo>> {
        let mut processes = Vec::new();
        let boot_time = read_boot_time_unix().unwrap_or(0);
        let ticks_per_sec = self.get_clock_ticks_per_second();
        let page_size = page_size_bytes().unwrap_or(4096);
        let users = read_uid_username_map();

        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let path = entry.path();
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let Ok(pid) = filename.parse::<u32>() else {
                    continue;
                };

                if let Ok(process) =
                    self.get_process_info(pid, boot_time, ticks_per_sec, page_size, &users)
                {
                    processes.push(process);
                }
            }
        }

        Ok(processes)
    }

    fn get_process_info(
        &self,
        pid: u32,
        boot_time_unix: u64,
        ticks_per_sec: u64,
        page_size: u64,
        users: &HashMap<u32, String>,
    ) -> Result<ProcessInfo> {
        let stat_path = format!("/proc/{pid}/stat");
        let cmdline_path = format!("/proc/{pid}/cmdline");
        let status_path = format!("/proc/{pid}/status");

        let stat = fs::read_to_string(&stat_path)?;
        let name = parse_proc_name_from_stat(&stat).unwrap_or_else(|| "unknown".to_string());

        let mut command_line = fs::read_to_string(&cmdline_path)
            .ok()
            .map(|s| s.replace('\0', " ").trim().to_string())
            .filter(|s| !s.is_empty());
        if command_line.is_none() {
            command_line = fs::read_to_string(format!("/proc/{pid}/comm"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }

        let after_name = stat.rfind(')').map(|i| &stat[i + 2..]).unwrap_or("");
        let fields: Vec<&str> = after_name.split_whitespace().collect();

        let threads = fields
            .get(17)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1);

        let utime = fields
            .get(11)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let stime = fields
            .get(12)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let cpu_ticks = utime.saturating_add(stime);

        let start_time_ticks = fields
            .get(19)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let start_time = if boot_time_unix > 0 && ticks_per_sec > 0 && start_time_ticks > 0 {
            let start_unix = boot_time_unix.saturating_add(start_time_ticks / ticks_per_sec);
            format_unix_local(start_unix)
        } else {
            None
        };

        let memory = read_rss_memory_bytes(pid, page_size).unwrap_or(0);
        let uid = read_uid_from_status(&status_path).unwrap_or(0);
        let user = Some(users.get(&uid).cloned().unwrap_or_else(|| uid.to_string()));
        let (io_read_bytes, io_write_bytes) = read_proc_io_bytes(pid);
        let handle_count = count_process_handles(pid);

        Ok(ProcessInfo {
            pid,
            name,
            command_line,
            threads,
            memory,
            cpu_ticks,
            uid,
            user,
            start_time,
            handle_count,
            io_read_bytes,
            io_write_bytes,
        })
    }
}

fn parse_proc_name_from_stat(stat: &str) -> Option<String> {
    let start = stat.find('(')?;
    let end = stat.rfind(')')?;
    if end > start {
        Some(stat[start + 1..end].to_string())
    } else {
        None
    }
}

fn read_boot_time_unix() -> Option<u64> {
    let content = fs::read_to_string("/proc/stat").ok()?;
    content
        .lines()
        .find(|line| line.starts_with("btime "))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok())
}

fn format_unix_local(ts: u64) -> Option<String> {
    let dt = Utc.timestamp_opt(ts as i64, 0).single()?;
    Some(
        dt.with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    )
}

fn read_rss_memory_bytes(pid: u32, page_size: u64) -> Option<u64> {
    let statm_path = format!("/proc/{pid}/statm");
    let statm = fs::read_to_string(statm_path).ok()?;
    let rss_pages = statm
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    Some(rss_pages.saturating_mul(page_size))
}

fn read_uid_from_status(status_path: &str) -> Option<u32> {
    let status = fs::read_to_string(status_path).ok()?;
    status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u32>().ok())
}

fn read_proc_io_bytes(pid: u32) -> (u64, u64) {
    let io_path = format!("/proc/{pid}/io");
    let mut read_bytes = 0u64;
    let mut write_bytes = 0u64;

    if let Ok(io) = fs::read_to_string(io_path) {
        for line in io.lines() {
            if let Some(v) = line.strip_prefix("read_bytes:") {
                read_bytes = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("write_bytes:") {
                write_bytes = v.trim().parse().unwrap_or(0);
            }
        }
    }

    (read_bytes, write_bytes)
}

fn count_process_handles(pid: u32) -> u32 {
    let fd_path = format!("/proc/{pid}/fd");
    fs::read_dir(fd_path)
        .ok()
        .map(|entries| entries.flatten().count() as u32)
        .unwrap_or(0)
}

fn read_uid_username_map() -> HashMap<u32, String> {
    let mut map = HashMap::new();
    if let Ok(content) = fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let mut parts = line.split(':');
            let username = parts.next().unwrap_or("").to_string();
            let _passwd = parts.next();
            let uid = parts
                .next()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(u32::MAX);
            if uid != u32::MAX && !username.is_empty() {
                map.insert(uid, username);
            }
        }
    }
    map
}

#[cfg(unix)]
fn clock_ticks_per_second() -> Option<u64> {
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v > 0 {
        Some(v as u64)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn clock_ticks_per_second() -> Option<u64> {
    None
}

#[cfg(unix)]
fn page_size_bytes() -> Option<u64> {
    let v = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if v > 0 {
        Some(v as u64)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn page_size_bytes() -> Option<u64> {
    None
}

#[derive(Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub command_line: Option<String>,
    pub threads: usize,
    pub memory: u64,
    pub cpu_ticks: u64,
    pub uid: u32,
    pub user: Option<String>,
    pub start_time: Option<String>,
    pub handle_count: u32,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
}
