use crate::integrations::PowerShellExecutor;
use crate::monitors::traits::*;
use crate::monitors::types::*;
use crate::platform::executor::CommandExecutor;
use crate::utils::parse_json_array;
use anyhow::{Context, Result};
use serde::Deserialize;

pub struct WindowsServiceMonitor {
    ps: PowerShellExecutor,
}

const SERVICES_SCRIPT: &str = r#"
    try {
        $services = Get-Service -ErrorAction SilentlyContinue
        $wmiServices = Get-CimInstance Win32_Service -ErrorAction SilentlyContinue

        $wmiMap = @{}
        if ($wmiServices) {
            foreach ($s in $wmiServices) {
                $wmiMap[$s.Name] = $s
            }
        }

        $result = foreach ($s in $services) {
            $wmi = if ($wmiMap.ContainsKey($s.Name)) { $wmiMap[$s.Name] } else { $null }

            $statusStr = $s.Status.ToString()
            $startTypeStr = "Unknown"
            if ($wmi) {
                $startTypeStr = $wmi.StartMode
            } elseif ($s.StartType) {
                $startTypeStr = $s.StartType.ToString()
            }

            [PSCustomObject]@{
                Name = $s.Name
                DisplayName = if ($s.DisplayName) { $s.DisplayName } else { $s.Name }
                Status = $statusStr
                StartType = $startTypeStr
                Description = if ($wmi -and $wmi.Description) { $wmi.Description } else { $null }
                CanStop = if ($s.CanStop) { $true } else { $false }
                CanPauseAndContinue = if ($s.CanPauseAndContinue) { $true } else { $false }
                DependentServices = if ($s.DependentServices) { @($s.DependentServices | Select-Object -ExpandProperty Name) } else { @() }
                ServiceType = if ($s.ServiceType) { $s.ServiceType.ToString() } else { $null }
            }
        }

        $result | ConvertTo-Json -Depth 3
    } catch {
        "[]"
    }
"#;

impl WindowsServiceMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    fn parse_services(output: &str) -> Result<Vec<ServiceEntry>> {
        let trimmed = output.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Ok(Vec::new());
        }

        let samples: Vec<ServiceSampleWindows> = if trimmed.starts_with('[') {
            parse_json_array(trimmed).context("Failed to parse services array")?
        } else {
            let single: ServiceSampleWindows =
                serde_json::from_str(trimmed).context("Failed to parse single service")?;
            vec![single]
        };

        Ok(samples
            .into_iter()
            .map(|s| ServiceEntry {
                name: s.Name,
                display_name: s.DisplayName,
                status: ServiceStatus::from_str(&s.Status),
                start_type: ServiceStartType::from_str(&s.StartType),
                description: s.Description,
                can_stop: s.CanStop,
                can_pause_and_continue: s.CanPauseAndContinue,
                dependent_services: s.DependentServices,
                service_type: s.ServiceType,
            })
            .collect())
    }
}

impl ServiceMonitorTrait for WindowsServiceMonitor {
    async fn collect_data(&self) -> Result<ServiceData> {
        let output = self.ps.execute(SERVICES_SCRIPT).await?;
        let services = Self::parse_services(&output)?;
        Ok(ServiceData { services })
    }

    async fn start_service(&self, service_name: &str) -> Result<()> {
        let script = format!("Start-Service -Name '{}' -ErrorAction Stop", service_name);
        self.ps.execute(&script).await?;
        Ok(())
    }

    async fn stop_service(&self, service_name: &str) -> Result<()> {
        let script = format!(
            "Stop-Service -Name '{}' -Force -ErrorAction Stop",
            service_name
        );
        self.ps.execute(&script).await?;
        Ok(())
    }

    async fn restart_service(&self, service_name: &str) -> Result<()> {
        let script = format!(
            "Restart-Service -Name '{}' -Force -ErrorAction Stop",
            service_name
        );
        self.ps.execute(&script).await?;
        Ok(())
    }

    async fn set_startup_type(
        &self,
        service_name: &str,
        startup_type: ServiceStartType,
    ) -> Result<()> {
        let type_str = match startup_type {
            ServiceStartType::Automatic => "Automatic",
            ServiceStartType::Manual => "Manual",
            ServiceStartType::Disabled => "Disabled",
            ServiceStartType::AutomaticDelayedStart => "Automatic", // Requires registry tweak for delayed on Set-Service
            ServiceStartType::Unknown => anyhow::bail!("Invalid startup type"),
        };

        let script = format!(
            "Set-Service -Name '{}' -StartupType {} -ErrorAction Stop",
            service_name, type_str
        );
        self.ps.execute(&script).await?;

        // If delayed start, try to set it via WMI or registry if needed
        if startup_type == ServiceStartType::AutomaticDelayedStart {
            let delay_script = format!(
                "Set-ItemProperty -Path 'HKLM:\\System\\CurrentControlSet\\Services\\{}' -Name 'DelayedAutoStart' -Value 1 -ErrorAction SilentlyContinue",
                service_name
            );
            let _ = self.ps.execute(&delay_script).await;
        } else {
            let delay_script = format!(
                "Set-ItemProperty -Path 'HKLM:\\System\\CurrentControlSet\\Services\\{}' -Name 'DelayedAutoStart' -Value 0 -ErrorAction SilentlyContinue",
                service_name
            );
            let _ = self.ps.execute(&delay_script).await;
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct ServiceSampleWindows {
    Name: String,
    DisplayName: String,
    Status: String,
    StartType: String,
    Description: Option<String>,
    CanStop: bool,
    CanPauseAndContinue: bool,
    #[serde(default)]
    DependentServices: Vec<String>,
    ServiceType: Option<String>,
}
