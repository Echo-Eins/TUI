use anyhow::Result;
use crate::integrations::LinuxSysMonitor;
use crate::monitors::types::*;
use crate::monitors::traits::*;

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
        let output = std::process::Command::new("systemctl")
            .args(["list-units", "--type=service", "--all", "--no-pager", "--no-legend"])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("systemctl failed");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut entries = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Format: UNIT LOAD ACTIVE SUB DESCRIPTION...
            if parts.len() < 5 {
                continue;
            }

            let unit_name = parts[0].trim_start_matches('●').trim();
            let name = unit_name.strip_suffix(".service").unwrap_or(unit_name).to_string();
            let _load = parts[1];
            let active = parts[2];
            let sub = parts[3];
            let description = parts[4..].join(" ");

            let status = match (active, sub) {
                ("active", "running") => ServiceStatus::Running,
                ("active", _) => ServiceStatus::Running,
                ("inactive", _) => ServiceStatus::Stopped,
                ("failed", _) => ServiceStatus::Stopped,
                _ => ServiceStatus::Unknown,
            };

            let start_type = Self::get_unit_file_state(&name);

            entries.push(ServiceEntry {
                name: name.clone(),
                display_name: description,
                status,
                start_type,
                description: None,
                can_stop: true,
                can_pause_and_continue: false,
                dependent_services: Vec::new(),
                service_type: Some("systemd".to_string()),
            });
        }

        Ok(entries)
    }

    fn get_unit_file_state(name: &str) -> ServiceStartType {
        let unit = format!("{}.service", name);
        if let Ok(output) = std::process::Command::new("systemctl")
            .args(["is-enabled", &unit])
            .output()
        {
            let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
            match state.as_str() {
                "enabled" => ServiceStartType::Automatic,
                "disabled" => ServiceStartType::Disabled,
                "static" => ServiceStartType::Manual,
                "masked" => ServiceStartType::Disabled,
                "indirect" => ServiceStartType::Manual,
                _ => ServiceStartType::Unknown,
            }
        } else {
            ServiceStartType::Unknown
        }
    }
}
