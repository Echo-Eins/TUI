use super::LinuxSysMonitor;
use anyhow::Result;
use std::fs;

impl LinuxSysMonitor {
    pub fn get_processes(&self) -> Result<Vec<ProcessInfo>> {
        let mut processes = Vec::new();

        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let path = entry.path();
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                if let Ok(pid) = filename.parse::<u32>() {
                    if let Ok(process) = self.get_process_info(pid) {
                        processes.push(process);
                    }
                }
            }
        }

        Ok(processes)
    }

    fn get_process_info(&self, pid: u32) -> Result<ProcessInfo> {
        let stat_path = format!("/proc/{}/stat", pid);
        let cmdline_path = format!("/proc/{}/cmdline", pid);
        let status_path = format!("/proc/{}/status", pid);

        let stat = fs::read_to_string(&stat_path)?;

        // Extract name from stat (it's in parentheses)
        // Handle names with spaces/parens by finding last ')'
        let name = if let Some(start) = stat.find('(') {
            if let Some(end) = stat.rfind(')') {
                stat[start + 1..end].to_string()
            } else {
                String::from("unknown")
            }
        } else {
            String::from("unknown")
        };

        // Read cmdline
        let cmdline = fs::read_to_string(&cmdline_path)
            .ok()
            .map(|s| s.replace('\0', " ").trim().to_string())
            .filter(|s| !s.is_empty());

        // Parse values from stat - fields after the closing paren
        let after_name = stat.rfind(')').map(|i| &stat[i + 2..]).unwrap_or("");
        let stat_fields: Vec<&str> = after_name.split_whitespace().collect();

        // Field 17 (0-indexed from after name) = num_threads
        let threads = stat_fields.get(17).and_then(|s| s.parse().ok()).unwrap_or(1);

        // Get CPU times: utime (field 11) and stime (field 12) from after name
        let utime = stat_fields.get(11).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let stime = stat_fields.get(12).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let cpu_ticks = utime + stime;

        // Read memory from statm
        let statm_path = format!("/proc/{}/statm", pid);
        let memory = if let Ok(statm) = fs::read_to_string(&statm_path) {
            let pages: Vec<u64> = statm
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            pages.get(1).unwrap_or(&0) * 4096 // RSS in pages * page size
        } else {
            0
        };

        // Read UID from status for user info
        let uid = if let Ok(status) = fs::read_to_string(&status_path) {
            status.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0)
        } else {
            0
        };

        // Resolve UID to username
        let user = self.uid_to_username(uid);

        Ok(ProcessInfo {
            pid,
            name,
            cmdline,
            threads,
            memory,
            cpu_ticks,
            uid,
            user,
        })
    }

    fn uid_to_username(&self, uid: u32) -> String {
        if uid == 0 {
            return "root".to_string();
        }

        // Try reading /etc/passwd
        if let Ok(content) = fs::read_to_string("/etc/passwd") {
            for line in content.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 {
                    if let Ok(file_uid) = parts[2].parse::<u32>() {
                        if file_uid == uid {
                            return parts[0].to_string();
                        }
                    }
                }
            }
        }

        uid.to_string()
    }
}

#[derive(Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cmdline: Option<String>,
    pub threads: usize,
    pub memory: u64,
    pub cpu_ticks: u64,
    pub uid: u32,
    pub user: String,
}
