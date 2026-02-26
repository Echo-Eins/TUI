use super::LinuxSysMonitor;
use crate::monitors::types::{ServiceEntry, ServiceStartType, ServiceStatus};
use anyhow::Result;
use std::process::Command;

impl LinuxSysMonitor {
    pub fn get_services(&self) -> Result<Vec<ServiceEntry>> {
        let mut services = Vec::new();

        // Use systemctl to list all services
        let output = Command::new("systemctl")
            .args(["list-units", "--type=service", "--all", "--no-pager", "--no-legend"])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("systemctl failed with status: {}", output.status);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 {
                continue;
            }

            let name = parts[0].trim_end_matches(".service").to_string();
            let load = parts[1];
            let active = parts[2];
            let sub = parts[3];

            // Reconstruct the description from the rest of the line
            let description = if parts.len() > 4 {
                Some(parts[4..].join(" "))
            } else {
                None
            };

            let status = match active {
                "active" => {
                    if sub == "running" {
                        ServiceStatus::Running
                    } else if sub == "exited" {
                        // Some oneshot services are marked active (exited)
                        ServiceStatus::Stopped
                    } else {
                        ServiceStatus::Running
                    }
                }
                "inactive" => ServiceStatus::Stopped,
                "activating" => ServiceStatus::StartPending,
                "deactivating" => ServiceStatus::StopPending,
                "failed" => ServiceStatus::Stopped,
                _ => ServiceStatus::Unknown,
            };

            // Linux services don't map perfectly to Windows pause/continue
            let can_stop = status == ServiceStatus::Running;
            
            // Getting start_type would require `systemctl show` for every unit, 
            // which is very slow. We'll use a placeholder or derived guess based on name/load.
            // A more comprehensive approach requires `systemctl show -p UnitFileState <service>`
            let start_type = if load == "loaded" {
                ServiceStartType::Automatic // Most loaded are enabled/automatic, but we don't know without querying
            } else {
                ServiceStartType::Unknown
            };

            services.push(ServiceEntry {
                name: name.clone(),
                display_name: name,
                status,
                start_type,
                description,
                can_stop,
                can_pause_and_continue: false, // systemd doesn't natively support pause/continue the same way
                dependent_services: Vec::new(),
                service_type: Some("systemd".to_string()),
            });
        }

        Ok(services)
    }
}
