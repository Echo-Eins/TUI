use crate::integrations::LinuxSysMonitor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use anyhow::Result;

pub struct LinuxServiceMonitor {
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
        let services = self.linux_sys.get_services()?;
        let entries = services
            .into_iter()
            .map(|s| ServiceEntry {
                name: s.name,
                display_name: s.display_name,
                status: map_service_status(&s.active_state, &s.sub_state),
                start_type: map_startup_type(s.unit_file_state.as_deref()),
                description: s.description,
                can_stop: s.can_stop,
                can_pause_and_continue: false,
                dependent_services: s.dependent_services,
                service_type: s.service_type,
            })
            .collect();

        Ok(ServiceData { services: entries })
    }

    async fn start_service(&self, service_name: &str) -> Result<()> {
        let output = std::process::Command::new("systemctl")
            .args(["start", service_name])
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to start service: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn stop_service(&self, service_name: &str) -> Result<()> {
        let output = std::process::Command::new("systemctl")
            .args(["stop", service_name])
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to stop service: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn restart_service(&self, service_name: &str) -> Result<()> {
        let output = std::process::Command::new("systemctl")
            .args(["restart", service_name])
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to restart service: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn set_startup_type(
        &self,
        service_name: &str,
        startup_type: ServiceStartType,
    ) -> Result<()> {
        let cmd = match startup_type {
            ServiceStartType::Automatic | ServiceStartType::AutomaticDelayedStart => "enable",
            ServiceStartType::Disabled => "disable",
            // Manual usually means disabling it from automatic start but it can still be started
            ServiceStartType::Manual => "disable",
            ServiceStartType::Unknown => anyhow::bail!("Invalid startup type"),
        };

        let output = std::process::Command::new("systemctl")
            .args([cmd, service_name])
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to change startup type: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
}

fn map_service_status(active: &str, sub: &str) -> ServiceStatus {
    match active {
        "active" => {
            if sub == "running" || sub == "listening" || sub == "exited" {
                ServiceStatus::Running
            } else {
                ServiceStatus::Running
            }
        }
        "inactive" => ServiceStatus::Stopped,
        "failed" => ServiceStatus::Stopped,
        "reloading" => ServiceStatus::ContinuePending,
        "activating" => ServiceStatus::StartPending,
        "deactivating" => ServiceStatus::StopPending,
        _ => ServiceStatus::Unknown,
    }
}

fn map_startup_type(unit_file_state: Option<&str>) -> ServiceStartType {
    match unit_file_state.unwrap_or_default() {
        "enabled" | "enabled-runtime" => ServiceStartType::Automatic,
        "disabled" | "masked" | "masked-runtime" => ServiceStartType::Disabled,
        "static" | "indirect" | "linked" | "linked-runtime" | "generated" | "transient"
        | "alias" => ServiceStartType::Manual,
        _ => ServiceStartType::Unknown,
    }
}
