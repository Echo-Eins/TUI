use super::LinuxSysMonitor;
use crate::monitors::types::{ServiceStartType, ServiceStatus};
use anyhow::Result;
use std::process::Command;

impl LinuxSysMonitor {
    pub fn get_services(&self) -> Result<Vec<LinuxServiceInfo>> {
        let mut services = Vec::new();

        // Get all units (running and loaded)
        let output = Command::new("systemctl")
            .args([
                "list-units",
                "--type=service",
                "--all",
                "--no-pager",
                "--no-legend",
                "--plain",
            ])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut unit_names: Vec<String> = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }
            let unit = parts[0].trim_end_matches(".service").to_string();
            // parts layout: UNIT LOAD ACTIVE SUB DESCRIPTION...
            let active = parts.get(2).copied().unwrap_or("");
            let sub = parts.get(3).copied().unwrap_or("");
            let description: String = if parts.len() > 4 {
                parts[4..].join(" ")
            } else {
                String::new()
            };

            let status = match (active, sub) {
                ("active", "running") => ServiceStatus::Running,
                ("active", "exited") => ServiceStatus::Stopped,
                ("inactive", _) => ServiceStatus::Stopped,
                ("activating", _) => ServiceStatus::StartPending,
                ("deactivating", _) => ServiceStatus::StopPending,
                ("failed", _) => ServiceStatus::Stopped,
                _ => ServiceStatus::Unknown,
            };

            unit_names.push(unit.clone());

            services.push(LinuxServiceInfo {
                name: unit.clone(),
                display_name: if description.is_empty() {
                    unit
                } else {
                    description
                },
                status,
                start_type: ServiceStartType::Unknown, // filled below
                description: None,
                service_type: None,
            });
        }

        // Batch get enable state from list-unit-files
        let unit_files_output = Command::new("systemctl")
            .args([
                "list-unit-files",
                "--type=service",
                "--no-pager",
                "--no-legend",
                "--plain",
            ])
            .output();

        if let Ok(out) = unit_files_output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut enable_map = std::collections::HashMap::new();
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let name = parts[0].trim_end_matches(".service").to_string();
                    let state = parts[1];
                    let start_type = match state {
                        "enabled" => ServiceStartType::Automatic,
                        "disabled" => ServiceStartType::Disabled,
                        "static" => ServiceStartType::Manual,
                        "masked" => ServiceStartType::Disabled,
                        "indirect" => ServiceStartType::Manual,
                        "generated" => ServiceStartType::Automatic,
                        _ => ServiceStartType::Unknown,
                    };
                    enable_map.insert(name, start_type);
                }
            }

            for svc in &mut services {
                if let Some(st) = enable_map.get(&svc.name) {
                    svc.start_type = *st;
                }
            }
        }

        // Batch get descriptions for first N services using systemctl show
        // We do this in a single call for all units
        if !unit_names.is_empty() {
            let batch_size = 50;
            for chunk in unit_names.chunks(batch_size) {
                let mut args = vec!["show", "--no-pager", "--property=Id,Description,Type"];
                let suffixed: Vec<String> =
                    chunk.iter().map(|n| format!("{}.service", n)).collect();
                let arg_refs: Vec<&str> = suffixed.iter().map(|s| s.as_str()).collect();
                args.extend(arg_refs);

                let show_output = Command::new("systemctl").args(&args).output();
                if let Ok(out) = show_output {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let mut current_id = String::new();
                    let mut current_desc = None;
                    let mut current_type = None;

                    for line in stdout.lines() {
                        if line.is_empty() {
                            // Record boundary
                            if !current_id.is_empty() {
                                let svc_name =
                                    current_id.trim_end_matches(".service").to_string();
                                if let Some(svc) =
                                    services.iter_mut().find(|s| s.name == svc_name)
                                {
                                    svc.description = current_desc.take();
                                    svc.service_type = current_type.take();
                                }
                            }
                            current_id.clear();
                            current_desc = None;
                            current_type = None;
                        } else if let Some(val) = line.strip_prefix("Id=") {
                            current_id = val.to_string();
                        } else if let Some(val) = line.strip_prefix("Description=") {
                            let desc = val.trim().to_string();
                            if !desc.is_empty() {
                                current_desc = Some(desc);
                            }
                        } else if let Some(val) = line.strip_prefix("Type=") {
                            let t = val.trim().to_string();
                            if !t.is_empty() {
                                current_type = Some(t);
                            }
                        }
                    }

                    // Handle last record
                    if !current_id.is_empty() {
                        let svc_name = current_id.trim_end_matches(".service").to_string();
                        if let Some(svc) = services.iter_mut().find(|s| s.name == svc_name) {
                            svc.description = current_desc;
                            svc.service_type = current_type;
                        }
                    }
                }
            }
        }

        Ok(services)
    }
}

#[derive(Debug)]
pub struct LinuxServiceInfo {
    pub name: String,
    pub display_name: String,
    pub status: ServiceStatus,
    pub start_type: ServiceStartType,
    pub description: Option<String>,
    pub service_type: Option<String>,
}
