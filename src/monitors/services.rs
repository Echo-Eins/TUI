use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::integrations::PowerShellExecutor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceData {
    pub services: Vec<ServiceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub name: String,
    pub display_name: String,
    pub status: ServiceStatus,
    pub start_type: ServiceStartType,
    pub description: Option<String>,
    pub can_stop: bool,
    pub can_pause_and_continue: bool,
    pub dependent_services: Vec<String>,
    pub service_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Paused,
    StartPending,
    StopPending,
    ContinuePending,
    PausePending,
    Unknown,
}

impl ServiceStatus {
    pub fn as_str(&self) -> &str {
        match self {
            ServiceStatus::Running => "Running",
            ServiceStatus::Stopped => "Stopped",
            ServiceStatus::Paused => "Paused",
            ServiceStatus::StartPending => "Starting",
            ServiceStatus::StopPending => "Stopping",
            ServiceStatus::ContinuePending => "Continuing",
            ServiceStatus::PausePending => "Pausing",
            ServiceStatus::Unknown => "Unknown",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Running" => ServiceStatus::Running,
            "Stopped" => ServiceStatus::Stopped,
            "Paused" => ServiceStatus::Paused,
            "StartPending" => ServiceStatus::StartPending,
            "StopPending" => ServiceStatus::StopPending,
            "ContinuePending" => ServiceStatus::ContinuePending,
            "PausePending" => ServiceStatus::PausePending,
            _ => ServiceStatus::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStartType {
    Automatic,
    Manual,
    Disabled,
    AutomaticDelayedStart,
    Unknown,
}

impl ServiceStartType {
    pub fn as_str(&self) -> &str {
        match self {
            ServiceStartType::Automatic => "Automatic",
            ServiceStartType::Manual => "Manual",
            ServiceStartType::Disabled => "Disabled",
            ServiceStartType::AutomaticDelayedStart => "Auto (Delayed)",
            ServiceStartType::Unknown => "Unknown",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Automatic" => ServiceStartType::Automatic,
            "Manual" => ServiceStartType::Manual,
            "Disabled" => ServiceStartType::Disabled,
            "AutomaticDelayedStart" => ServiceStartType::AutomaticDelayedStart,
            _ => ServiceStartType::Unknown,
        }
    }
}

pub struct ServiceMonitor {
    ps: PowerShellExecutor,
}

impl ServiceMonitor {
    pub fn new(ps: PowerShellExecutor) -> Result<Self> {
        Ok(Self { ps })
    }

    pub async fn collect_data(&self) -> Result<ServiceData> {
        let services = self.get_services().await?;
        Ok(ServiceData { services })
    }

    async fn get_services(&self) -> Result<Vec<ServiceEntry>> {
        let script = r#"
            Get-Service | Select-Object -Property `
                Name,
                DisplayName,
                Status,
                StartType,
                @{Name='Description';Expression={
                    try {
                        $svc = Get-WmiObject -Class Win32_Service -Filter "Name='$($_.Name)'"
                        $svc.Description
                    } catch {
                        $null
                    }
                }},
                CanStop,
                CanPauseAndContinue,
                @{Name='DependentServices';Expression={
                    ($_.DependentServices | ForEach-Object { $_.Name }) -join ','
                }},
                ServiceType | ConvertTo-Json
        "#;

        let output = self.ps.execute(script).await?;
        if output.trim().is_empty() || output.trim() == "[]" {
            return Ok(Vec::new());
        }

        let services: Vec<ServiceSample> = serde_json::from_str(&output)
            .context("Failed to parse service data")?;

        Ok(services
            .into_iter()
            .map(|s| ServiceEntry {
                name: s.Name,
                display_name: s.DisplayName,
                status: ServiceStatus::from_str(&s.Status),
                start_type: ServiceStartType::from_str(&s.StartType),
                description: s.Description.filter(|d| !d.is_empty()),
                can_stop: s.CanStop.unwrap_or(false),
                can_pause_and_continue: s.CanPauseAndContinue.unwrap_or(false),
                dependent_services: s.DependentServices
                    .filter(|d| !d.is_empty())
                    .map(|d| d.split(',').map(|s| s.to_string()).collect())
                    .unwrap_or_default(),
                service_type: s.ServiceType,
            })
            .collect())
    }

    pub async fn start_service(&self, service_name: &str) -> Result<()> {
        let script = format!("Start-Service -Name '{}'", service_name);
        self.ps.execute(&script).await?;
        Ok(())
    }

    pub async fn stop_service(&self, service_name: &str) -> Result<()> {
        let script = format!("Stop-Service -Name '{}'", service_name);
        self.ps.execute(&script).await?;
        Ok(())
    }

    pub async fn restart_service(&self, service_name: &str) -> Result<()> {
        let script = format!("Restart-Service -Name '{}'", service_name);
        self.ps.execute(&script).await?;
        Ok(())
    }

    pub async fn set_startup_type(&self, service_name: &str, startup_type: ServiceStartType) -> Result<()> {
        let startup_str = match startup_type {
            ServiceStartType::Automatic => "Automatic",
            ServiceStartType::Manual => "Manual",
            ServiceStartType::Disabled => "Disabled",
            ServiceStartType::AutomaticDelayedStart => "AutomaticDelayedStart",
            _ => return Err(anyhow::anyhow!("Invalid startup type")),
        };
        let script = format!("Set-Service -Name '{}' -StartupType {}", service_name, startup_str);
        self.ps.execute(&script).await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct ServiceSample {
    Name: String,
    DisplayName: String,
    Status: String,
    StartType: String,
    Description: Option<String>,
    CanStop: Option<bool>,
    CanPauseAndContinue: Option<bool>,
    DependentServices: Option<String>,
    ServiceType: Option<String>,
}
