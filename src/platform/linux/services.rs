use super::LinuxSysMonitor;
use anyhow::Result;
use std::collections::HashMap;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct LinuxServiceInfo {
    pub name: String,
    pub display_name: String,
    pub active_state: String,
    pub sub_state: String,
    pub unit_file_state: Option<String>,
    pub description: Option<String>,
    pub can_stop: bool,
    pub can_reload: bool,
    pub dependent_services: Vec<String>,
    pub service_type: Option<String>,
}

impl LinuxSysMonitor {
    pub fn get_services(&self) -> Result<Vec<LinuxServiceInfo>> {
        if let Ok(services) = self.get_services_from_systemctl_show() {
            if !services.is_empty() {
                return Ok(services);
            }
        }

        self.get_services_from_list_units()
    }

    fn get_services_from_systemctl_show(&self) -> Result<Vec<LinuxServiceInfo>> {
        let output = Command::new("systemctl")
            .args([
                "show",
                "--type=service",
                "--all",
                "--no-pager",
                "--property=Id,Description,ActiveState,SubState,UnitFileState,CanStop,CanReload,Type,Wants,Requires",
            ])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("systemctl show failed with status: {}", output.status);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut services = Vec::new();
        let mut block: HashMap<String, String> = HashMap::new();

        let flush_block = |block: &mut HashMap<String, String>, services: &mut Vec<LinuxServiceInfo>| {
            let Some(id) = block.get("Id").cloned() else {
                block.clear();
                return;
            };
            if !id.ends_with(".service") {
                block.clear();
                return;
            }

            let name = id.trim_end_matches(".service").to_string();
            let active_state = block.get("ActiveState").cloned().unwrap_or_default();
            let sub_state = block.get("SubState").cloned().unwrap_or_default();
            let unit_file_state = block.get("UnitFileState").cloned();
            let description = block.get("Description").cloned().filter(|v| !v.is_empty());
            let can_stop = block
                .get("CanStop")
                .map(|v| v.eq_ignore_ascii_case("yes"))
                .unwrap_or(false);
            let can_reload = block
                .get("CanReload")
                .map(|v| v.eq_ignore_ascii_case("yes"))
                .unwrap_or(false);
            let service_type = block.get("Type").cloned().filter(|v| !v.is_empty());
            let dependent_services = parse_dependencies(
                block.get("Wants").map(|s| s.as_str()).unwrap_or(""),
                block.get("Requires").map(|s| s.as_str()).unwrap_or(""),
            );

            services.push(LinuxServiceInfo {
                name: name.clone(),
                display_name: name,
                active_state,
                sub_state,
                unit_file_state,
                description,
                can_stop,
                can_reload,
                dependent_services,
                service_type,
            });

            block.clear();
        };

        for line in stdout.lines() {
            if line.trim().is_empty() {
                flush_block(&mut block, &mut services);
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                block.insert(k.to_string(), v.to_string());
            }
        }
        flush_block(&mut block, &mut services);

        Ok(services)
    }

    fn get_services_from_list_units(&self) -> Result<Vec<LinuxServiceInfo>> {
        let mut services = Vec::new();
        let unit_file_states = self.read_unit_file_states().unwrap_or_default();

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

            let unit_name = parts[0];
            if !unit_name.ends_with(".service") {
                continue;
            }

            let name = unit_name.trim_end_matches(".service").to_string();
            let active_state = parts[2].to_string();
            let sub_state = parts[3].to_string();

            let description = if parts.len() > 4 {
                Some(parts[4..].join(" "))
            } else {
                None
            };

            let can_stop = active_state == "active" && sub_state == "running";
            let unit_file_state = unit_file_states
                .get(unit_name)
                .cloned()
                .or_else(|| unit_file_states.get(&name).cloned());

            services.push(LinuxServiceInfo {
                name: name.clone(),
                display_name: name,
                active_state,
                sub_state,
                unit_file_state,
                description,
                can_stop,
                can_reload: false,
                dependent_services: Vec::new(),
                service_type: Some("systemd".to_string()),
            });
        }

        Ok(services)
    }

    fn read_unit_file_states(&self) -> Result<HashMap<String, String>> {
        let output = Command::new("systemctl")
            .args([
                "list-unit-files",
                "--type=service",
                "--no-pager",
                "--no-legend",
            ])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("systemctl list-unit-files failed with status: {}", output.status);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut states = HashMap::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }

            let unit_name = parts[0].to_string();
            let state = parts[1].to_string();
            states.insert(unit_name.clone(), state.clone());
            states.insert(
                unit_name.trim_end_matches(".service").to_string(),
                state.clone(),
            );
        }

        Ok(states)
    }
}

fn parse_dependencies(wants: &str, requires: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for raw in [wants, requires] {
        for dep in raw.split_whitespace() {
            let dep = dep.trim_end_matches(".service").trim();
            if dep.is_empty() {
                continue;
            }
            if !deps.iter().any(|x| x == dep) {
                deps.push(dep.to_string());
            }
        }
    }
    deps
}
