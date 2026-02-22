use anyhow::Result;
use crate::integrations::LinuxSysMonitor;
use crate::monitors::types::*;
use crate::monitors::traits::*;
use std::collections::HashMap;

pub struct LinuxServiceMonitor {
    #[allow(dead_code)]
    linux_sys: LinuxSysMonitor,
}

impl LinuxServiceMonitor {
    pub fn new(_ps: crate::integrations::PowerShellExecutor) -> Result<Self> {
        Ok(Self {
            linux_sys: LinuxSysMonitor::new(),
        })
    }
}

impl ServiceMonitorTrait for LinuxServiceMonitor {
    async fn collect_data(&self) -> Result<ServiceData> {
        let services = Self::list_systemd_services()?;
        Ok(ServiceData { services })
    }

    async fn start_service(&self, service_name: &str) -> Result<()> {
        let output = std::process::Command::new("systemctl")
            .args(["start", service_name])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to start service: {}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    }

    async fn stop_service(&self, service_name: &str) -> Result<()> {
        let output = std::process::Command::new("systemctl")
            .args(["stop", service_name])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to stop service: {}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    }

    async fn restart_service(&self, service_name: &str) -> Result<()> {
        let output = std::process::Command::new("systemctl")
            .args(["restart", service_name])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to restart service: {}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    }

    async fn set_startup_type(&self, service_name: &str, startup_type: ServiceStartType) -> Result<()> {
        let cmd = match startup_type {
            ServiceStartType::Automatic | ServiceStartType::AutomaticDelayedStart => "enable",
            ServiceStartType::Disabled => "disable",
            ServiceStartType::Manual => "disable",
            ServiceStartType::Unknown => anyhow::bail!("Invalid startup type"),
        };

        let output = std::process::Command::new("systemctl")
            .args([cmd, service_name])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to change startup type: {}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    }
}

impl LinuxServiceMonitor {
    fn list_systemd_services() -> Result<Vec<ServiceEntry>> {
        // Step 1: Get unit file states in batch (enabled/disabled/static/masked)
        let unit_file_states = Self::get_all_unit_file_states();

        // Step 2: Get service descriptions in batch via systemctl show
        let descriptions = Self::get_all_descriptions();

        // Step 3: Parse running units from list-units
        let output = std::process::Command::new("systemctl")
            .args(["list-units", "--type=service", "--all", "--no-pager", "--no-legend", "--plain"])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("systemctl list-units failed");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut entries = Vec::new();

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 {
                continue;
            }

            // --plain output with --no-legend:
            // UNIT                     LOAD   ACTIVE SUB     DESCRIPTION
            // Some lines start with ● for failed services
            let (unit_name, load_idx) = if parts[0].contains('.') {
                // Normal: "docker.service loaded active running ..."
                (parts[0], 1)
            } else if parts.len() >= 5 && parts[1].contains('.') {
                // Has prefix like ●: "● docker.service loaded failed failed ..."
                (parts[1], 2)
            } else {
                continue;
            };

            let name = unit_name.strip_suffix(".service").unwrap_or(unit_name).to_string();

            let _load = parts.get(load_idx).copied().unwrap_or("");
            let active = parts.get(load_idx + 1).copied().unwrap_or("");
            let sub = parts.get(load_idx + 2).copied().unwrap_or("");
            let desc_start = load_idx + 3;
            let display_name = if parts.len() > desc_start {
                parts[desc_start..].join(" ")
            } else {
                name.clone()
            };

            let status = match active {
                "active" => {
                    match sub {
                        "running" => ServiceStatus::Running,
                        "exited" => ServiceStatus::Stopped, // oneshot that completed
                        _ => ServiceStatus::Running,
                    }
                }
                "inactive" => ServiceStatus::Stopped,
                "failed" => ServiceStatus::Stopped,
                "activating" => ServiceStatus::StartPending,
                "deactivating" => ServiceStatus::StopPending,
                _ => ServiceStatus::Unknown,
            };

            let start_type = unit_file_states.get(&name)
                .copied()
                .unwrap_or(ServiceStartType::Unknown);

            let description = descriptions.get(&name).cloned();

            entries.push(ServiceEntry {
                name: name.clone(),
                display_name,
                status,
                start_type,
                description,
                can_stop: status == ServiceStatus::Running,
                can_pause_and_continue: false,
                dependent_services: Vec::new(),
                service_type: Some("systemd".to_string()),
            });
        }

        Ok(entries)
    }

    /// Batch read all unit file states from `systemctl list-unit-files`
    fn get_all_unit_file_states() -> HashMap<String, ServiceStartType> {
        let mut states = HashMap::new();

        let output = match std::process::Command::new("systemctl")
            .args(["list-unit-files", "--type=service", "--no-pager", "--no-legend", "--plain"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return states,
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let unit = parts[0];
                let state = parts[1];
                let name = unit.strip_suffix(".service").unwrap_or(unit).to_string();

                let start_type = match state {
                    "enabled" | "enabled-runtime" => ServiceStartType::Automatic,
                    "disabled" => ServiceStartType::Disabled,
                    "static" => ServiceStartType::Manual,
                    "masked" | "masked-runtime" => ServiceStartType::Disabled,
                    "indirect" => ServiceStartType::Manual,
                    "generated" => ServiceStartType::Automatic,
                    "alias" => ServiceStartType::Automatic,
                    _ => ServiceStartType::Unknown,
                };

                states.insert(name, start_type);
            }
        }

        states
    }

    /// Batch read descriptions for all services using systemctl show
    fn get_all_descriptions() -> HashMap<String, String> {
        let mut descriptions = HashMap::new();

        // systemctl show --property=Id,Description --type=service '*'
        let output = match std::process::Command::new("systemctl")
            .args(["show", "--property=Id,Description", "--type=service", "*"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return descriptions,
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut current_id: Option<String> = None;

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                current_id = None;
                continue;
            }

            if let Some(id_val) = line.strip_prefix("Id=") {
                let name = id_val.strip_suffix(".service").unwrap_or(id_val).to_string();
                current_id = Some(name);
            } else if let Some(desc_val) = line.strip_prefix("Description=") {
                if let Some(id) = current_id.take() {
                    let desc = desc_val.trim().to_string();
                    if !desc.is_empty() {
                        descriptions.insert(id, desc);
                    }
                }
            }
        }

        descriptions
    }
}
