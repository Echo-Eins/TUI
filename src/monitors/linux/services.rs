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
                status: s.status,
                start_type: s.start_type,
                description: s.description,
                can_stop: true, // Generally true for systemctl, depends on root
                can_pause_and_continue: false, // systemd doesn't natively map to pause/continue like Windows
                dependent_services: Vec::new(), // Could be parsed from `systemctl show -p WantedBy` if needed
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
