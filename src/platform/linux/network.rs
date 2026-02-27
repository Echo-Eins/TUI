use super::LinuxSysMonitor;
use anyhow::{Context, Result};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct NetworkInterfaceIpInfo {
    pub name: String,
    pub ipv4: String,
    pub ipv6: String,
    pub mac: String,
}

#[derive(Debug, Clone)]
pub struct NetworkInterfaceStats {
    pub name: String,
    pub description: String,
    pub status: String,
    pub link_speed: String,
    pub mac_address: String,
    pub mtu: u32,
    pub duplex: String,
    pub ipv4_address: String,
    pub ipv6_address: String,
    pub gateway: Option<String>,
    pub gateway_port: Option<u16>,
    pub dns_servers: Vec<String>,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub download_speed: f64,
    pub upload_speed: f64,
    pub peak_download: f64,
    pub peak_upload: f64,
}

#[derive(Debug, Clone)]
pub struct NetworkConnectionInfo {
    pub process_name: String,
    pub pid: u32,
    pub protocol: String,
    pub local_address: String,
    pub local_port: u16,
    pub remote_address: String,
    pub remote_port: u16,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct ProcessBandwidthInfo {
    pub pid: u32,
    pub name: String,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub estimated: bool,
}

#[derive(Debug, Clone)]
pub struct GatewayInfo {
    pub interface: String,
    pub address: String,
    pub port: Option<u16>,
    pub metric: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DnsServerInfo {
    pub address: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedAddressInfo {
    pub query: String,
    pub host: String,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PingResult {
    pub transmitted: u32,
    pub received: u32,
    pub packet_loss_percent: f32,
    pub avg_latency_ms: Option<f32>,
}

#[derive(Debug)]
struct InterfaceDetails {
    description: String,
    status: String,
    link_speed: String,
    mac_address: String,
    mtu: u32,
    duplex: String,
}

impl LinuxSysMonitor {
    pub fn get_network_interfaces(&self) -> Result<Vec<NetworkInterfaceIpInfo>> {
        let ipv4 = self.read_interface_ipv4_map();
        let mut ipv6 = self.read_interface_ipv6_map_from_ip();
        if ipv6.is_empty() {
            ipv6 = self.read_interface_ipv6_map_from_proc();
        }

        let mut names: BTreeSet<String> = BTreeSet::new();
        names.extend(ipv4.keys().cloned());
        names.extend(ipv6.keys().cloned());
        names.extend(self.read_interface_names_from_sysfs());

        let mut out = Vec::with_capacity(names.len());
        for name in names {
            out.push(NetworkInterfaceIpInfo {
                name: name.clone(),
                ipv4: ipv4.get(&name).cloned().unwrap_or_default(),
                ipv6: ipv6.get(&name).cloned().unwrap_or_default(),
                mac: read_trimmed(Path::new("/sys/class/net").join(&name).join("address"))
                    .unwrap_or_default(),
            });
        }

        Ok(out)
    }

    pub fn get_network_interfaces_stats(&self) -> Result<Vec<NetworkInterfaceStats>> {
        let traffic = self.read_network_traffic_stats()?;
        let ips = self.get_network_interfaces()?;
        let gateways = self.get_default_gateways().unwrap_or_default();
        let dns_servers: Vec<String> = self
            .get_dns_servers()
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.address)
            .collect();

        let mut ip_map: HashMap<String, NetworkInterfaceIpInfo> = HashMap::with_capacity(ips.len());
        for ip in ips {
            ip_map.insert(ip.name.clone(), ip);
        }

        let mut gw_map: HashMap<String, GatewayInfo> = HashMap::new();
        for gw in gateways {
            gw_map
                .entry(gw.interface.clone())
                .and_modify(|existing| {
                    let existing_metric = existing.metric.unwrap_or(u32::MAX);
                    let candidate_metric = gw.metric.unwrap_or(u32::MAX);
                    if candidate_metric < existing_metric {
                        *existing = gw.clone();
                    }
                })
                .or_insert(gw);
        }

        let mut names: BTreeSet<String> = BTreeSet::new();
        names.extend(traffic.keys().cloned());
        names.extend(ip_map.keys().cloned());
        names.extend(self.read_interface_names_from_sysfs());

        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let (rx, tx) = traffic.get(&name).copied().unwrap_or((0, 0));
            let details = self.read_interface_details(&name);
            let ip = ip_map.get(&name);
            let gw = gw_map.get(&name);

            out.push(NetworkInterfaceStats {
                name: name.clone(),
                description: details.description,
                status: details.status,
                link_speed: details.link_speed,
                mac_address: details.mac_address,
                mtu: details.mtu,
                duplex: details.duplex,
                ipv4_address: ip.map(|x| x.ipv4.clone()).unwrap_or_default(),
                ipv6_address: ip.map(|x| x.ipv6.clone()).unwrap_or_default(),
                gateway: gw.map(|x| x.address.clone()),
                gateway_port: gw.and_then(|x| x.port),
                dns_servers: dns_servers.clone(),
                bytes_received: rx,
                bytes_sent: tx,
                download_speed: 0.0,
                upload_speed: 0.0,
                peak_download: 0.0,
                peak_upload: 0.0,
            });
        }

        Ok(out)
    }

    pub fn get_default_gateway(&self) -> Result<Option<GatewayInfo>> {
        Ok(self.get_default_gateways()?.into_iter().next())
    }

    pub fn get_default_gateways(&self) -> Result<Vec<GatewayInfo>> {
        let mut gateways = self.read_ipv4_default_gateways()?;
        gateways.extend(self.read_ipv6_default_gateways());
        gateways.sort_by_key(|g| g.metric.unwrap_or(u32::MAX));
        Ok(gateways)
    }

    pub fn get_dns_servers(&self) -> Result<Vec<DnsServerInfo>> {
        let mut servers = self.read_dns_from_resolv_conf()?;
        let is_stub_only = !servers.is_empty()
            && servers
                .iter()
                .all(|s| matches!(s.address.as_str(), "127.0.0.53" | "127.0.0.1" | "::1"));

        if servers.is_empty() || is_stub_only {
            let resolved = self.read_dns_from_resolvectl();
            if !resolved.is_empty() {
                servers = resolved;
            }
        }

        Ok(dedup_dns_servers(servers))
    }

    pub fn get_dns_search_domains(&self) -> Result<Vec<String>> {
        let mut domains = Vec::new();
        let content = fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("search ") {
                domains.extend(rest.split_whitespace().map(|s| s.to_string()));
            } else if let Some(rest) = trimmed.strip_prefix("domain ") {
                domains.extend(rest.split_whitespace().map(|s| s.to_string()));
            }
        }
        domains.sort();
        domains.dedup();
        Ok(domains)
    }

    pub fn resolve_host(&self, query: &str) -> Result<ResolvedAddressInfo> {
        let host = sanitize_resolve_target(query);
        let socket_target = format!("{host}:0");
        let addrs = socket_target
            .to_socket_addrs()
            .with_context(|| format!("failed to resolve host: {host}"))?;

        let mut uniq = BTreeSet::new();
        for addr in addrs {
            uniq.insert(addr.ip().to_string());
        }

        Ok(ResolvedAddressInfo {
            query: query.to_string(),
            host,
            addresses: uniq.into_iter().collect(),
        })
    }

    pub fn ping_host(&self, target: &str, count: u32, timeout_secs: u32) -> Result<PingResult> {
        let output = Command::new("ping")
            .args([
                "-n",
                "-c",
                &count.max(1).to_string(),
                "-W",
                &timeout_secs.max(1).to_string(),
                target,
            ])
            .output()
            .with_context(|| format!("failed to execute ping for target {target}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut transmitted = 0u32;
        let mut received = 0u32;
        let mut packet_loss_percent = 100.0f32;
        let mut avg_latency_ms = None;

        for line in stdout.lines() {
            if line.contains("packets transmitted") && line.contains("received") {
                let parts: Vec<&str> = line.split(',').collect();
                if let Some(v) = parts.first() {
                    transmitted = v
                        .split_whitespace()
                        .next()
                        .and_then(|x| x.parse::<u32>().ok())
                        .unwrap_or(0);
                }
                if let Some(v) = parts.get(1) {
                    received = v
                        .split_whitespace()
                        .next()
                        .and_then(|x| x.parse::<u32>().ok())
                        .unwrap_or(0);
                }
                if let Some(v) = parts.get(2) {
                    packet_loss_percent = v
                        .split_whitespace()
                        .next()
                        .and_then(|x| x.trim_end_matches('%').parse::<f32>().ok())
                        .unwrap_or(100.0);
                }
            }
            if line.starts_with("rtt min/avg/max/")
                || line.starts_with("round-trip min/avg/max/")
            {
                if let Some(after_eq) = line.split('=').nth(1) {
                    let value_part = after_eq.split_whitespace().next().unwrap_or("");
                    let mut pieces = value_part.split('/');
                    let _ = pieces.next();
                    avg_latency_ms = pieces.next().and_then(|x| x.parse::<f32>().ok());
                }
            }
        }

        Ok(PingResult {
            transmitted,
            received,
            packet_loss_percent,
            avg_latency_ms,
        })
    }

    pub fn detect_path_mtu(&self, target: &str) -> Result<Option<u32>> {
        let overhead = 28u32; // 20 (IP) + 8 (ICMP)
        let mut low = 576u32.saturating_sub(overhead);
        let mut high = 8972u32.saturating_sub(overhead);

        if !self.ping_df_payload(target, low)? {
            return Ok(None);
        }

        while low < high {
            let mid = (low + high + 1) / 2;
            if self.ping_df_payload(target, mid)? {
                low = mid;
            } else {
                high = mid.saturating_sub(1);
            }
        }

        Ok(Some(low + overhead))
    }

    pub fn scan_tcp_ports(
        &self,
        target: &str,
        ports: &[u16],
        timeout: Duration,
    ) -> Result<Vec<u16>> {
        let mut open = Vec::new();
        for port in ports {
            let mut resolved: Vec<SocketAddr> = format!("{target}:{port}")
                .to_socket_addrs()
                .with_context(|| format!("failed to resolve {target}:{port}"))?
                .collect();
            resolved.sort_by_key(|a| match a.ip() {
                IpAddr::V4(_) => 0,
                IpAddr::V6(_) => 1,
            });

            let mut is_open = false;
            for addr in resolved {
                if TcpStream::connect_timeout(&addr, timeout).is_ok() {
                    is_open = true;
                    break;
                }
            }
            if is_open {
                open.push(*port);
            }
        }
        Ok(open)
    }

    pub fn get_network_connections(&self) -> Result<Vec<NetworkConnectionInfo>> {
        let owners = self.collect_socket_owners();
        let mut connections = Vec::new();

        self.append_connections_from_proc(
            "/proc/net/tcp",
            "TCP",
            false,
            &owners,
            &mut connections,
        )?;
        self.append_connections_from_proc(
            "/proc/net/tcp6",
            "TCP6",
            true,
            &owners,
            &mut connections,
        )?;
        self.append_connections_from_proc(
            "/proc/net/udp",
            "UDP",
            false,
            &owners,
            &mut connections,
        )?;
        self.append_connections_from_proc(
            "/proc/net/udp6",
            "UDP6",
            true,
            &owners,
            &mut connections,
        )?;

        connections.sort_by(|a, b| {
            let rank_a = state_rank(&a.state);
            let rank_b = state_rank(&b.state);
            rank_a
                .cmp(&rank_b)
                .then_with(|| a.pid.cmp(&b.pid))
                .then_with(|| a.local_port.cmp(&b.local_port))
        });
        connections.truncate(512);

        Ok(connections)
    }

    pub fn get_process_bandwidth(&self) -> Result<Vec<ProcessBandwidthInfo>> {
        let mut usage = self.read_process_bandwidth_from_ss();
        if usage.is_empty() {
            usage = self.read_process_bandwidth_from_socket_queues();
        }

        usage.sort_by(|a, b| {
            (b.bytes_received + b.bytes_sent)
                .cmp(&(a.bytes_received + a.bytes_sent))
                .then_with(|| a.pid.cmp(&b.pid))
        });
        usage.truncate(256);
        Ok(usage)
    }

    fn ping_df_payload(&self, target: &str, payload: u32) -> Result<bool> {
        let output = Command::new("ping")
            .args(["-n", "-c", "1", "-W", "1", "-M", "do", "-s", &payload.to_string(), target])
            .output()
            .with_context(|| format!("failed to execute MTU probe ping for {target}"))?;
        Ok(output.status.success())
    }

    fn read_interface_names_from_sysfs(&self) -> Vec<String> {
        let mut names = Vec::new();
        if let Ok(entries) = fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_string());
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    fn read_interface_ipv4_map(&self) -> HashMap<String, String> {
        self.read_interface_ip_map_from_ip("-4", "inet", |addr| !addr.starts_with("127."))
    }

    fn read_interface_ipv6_map_from_ip(&self) -> HashMap<String, String> {
        self.read_interface_ip_map_from_ip("-6", "inet6", |addr| !addr.starts_with("fe80:"))
    }

    fn read_interface_ip_map_from_ip<F>(
        &self,
        family_flag: &str,
        keyword: &str,
        preferred: F,
    ) -> HashMap<String, String>
    where
        F: Fn(&str) -> bool,
    {
        let mut map = HashMap::new();
        let output = Command::new("ip")
            .args(["-o", family_flag, "addr", "show"])
            .output();

        let Ok(output) = output else {
            return map;
        };
        if !output.status.success() {
            return map;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 || parts[2] != keyword {
                continue;
            }

            let iface = normalize_iface_name(parts[1]);
            let addr = parts[3].split('/').next().unwrap_or("").to_string();
            if addr.is_empty() {
                continue;
            }

            let should_replace = match map.get(&iface) {
                Some(current) => !preferred(current) && preferred(&addr),
                None => true,
            };
            if should_replace {
                map.insert(iface, addr);
            }
        }
        map
    }

    fn read_interface_ipv6_map_from_proc(&self) -> HashMap<String, String> {
        let mut map: HashMap<String, String> = HashMap::new();
        let content = fs::read_to_string("/proc/net/if_inet6").unwrap_or_default();
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 6 {
                continue;
            }
            let raw = parts[0];
            let iface = parts[5].to_string();
            if let Some(ipv6) = parse_ipv6_network_order(raw) {
                let is_link_local = ipv6.starts_with("fe80:");
                let replace = match map.get(&iface) {
                    Some(current) => current.starts_with("fe80:") && !is_link_local,
                    None => true,
                };
                if replace {
                    map.insert(iface, ipv6);
                }
            }
        }
        map
    }

    fn read_network_traffic_stats(&self) -> Result<HashMap<String, (u64, u64)>> {
        let content = fs::read_to_string("/proc/net/dev").context("failed to read /proc/net/dev")?;
        let mut map = HashMap::new();
        for line in content.lines().skip(2) {
            let Some((iface, data)) = line.split_once(':') else {
                continue;
            };
            let iface = iface.trim().to_string();
            let values: Vec<&str> = data.split_whitespace().collect();
            if values.len() < 16 {
                continue;
            }
            let rx = values[0].parse::<u64>().unwrap_or(0);
            let tx = values[8].parse::<u64>().unwrap_or(0);
            map.insert(iface, (rx, tx));
        }
        Ok(map)
    }

    fn read_ipv4_default_gateways(&self) -> Result<Vec<GatewayInfo>> {
        let content = fs::read_to_string("/proc/net/route").context("failed to read /proc/net/route")?;
        let mut gateways = Vec::new();

        for line in content.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 8 {
                continue;
            }
            if parts[1] != "00000000" {
                continue;
            }

            let flags = u16::from_str_radix(parts[3], 16).unwrap_or(0);
            if flags & 0x2 == 0 {
                continue;
            }

            let Some(gateway_ip) = parse_ipv4_hex_le(parts[2]) else {
                continue;
            };
            let metric = parts[6].parse::<u32>().ok();
            gateways.push(GatewayInfo {
                interface: parts[0].to_string(),
                address: gateway_ip,
                port: None,
                metric,
            });
        }

        gateways.sort_by_key(|g| g.metric.unwrap_or(u32::MAX));
        Ok(gateways)
    }

    fn read_ipv6_default_gateways(&self) -> Vec<GatewayInfo> {
        let output = Command::new("ip")
            .args(["-6", "route", "show", "default"])
            .output();
        let Ok(output) = output else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut gateways = Vec::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.first().copied() != Some("default") {
                continue;
            }

            let mut address = None;
            let mut iface = None;
            let mut metric = None;
            let mut i = 0usize;
            while i < parts.len() {
                match parts[i] {
                    "via" if i + 1 < parts.len() => {
                        address = Some(parts[i + 1].to_string());
                        i += 1;
                    }
                    "dev" if i + 1 < parts.len() => {
                        iface = Some(parts[i + 1].to_string());
                        i += 1;
                    }
                    "metric" if i + 1 < parts.len() => {
                        metric = parts[i + 1].parse::<u32>().ok();
                        i += 1;
                    }
                    _ => {}
                }
                i += 1;
            }

            if let (Some(interface), Some(address)) = (iface, address) {
                gateways.push(GatewayInfo {
                    interface,
                    address,
                    port: None,
                    metric,
                });
            }
        }

        gateways
    }

    fn read_dns_from_resolv_conf(&self) -> Result<Vec<DnsServerInfo>> {
        let content = fs::read_to_string("/etc/resolv.conf")
            .context("failed to read /etc/resolv.conf for DNS servers")?;
        let mut servers = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("nameserver") {
                if let Some(addr) = rest.split_whitespace().next() {
                    if parse_ip_candidate(addr).is_some() {
                        servers.push(DnsServerInfo {
                            address: addr.to_string(),
                            source: "resolv.conf".to_string(),
                        });
                    }
                }
            }
        }
        Ok(servers)
    }

    fn read_dns_from_resolvectl(&self) -> Vec<DnsServerInfo> {
        let output = Command::new("resolvectl").arg("dns").output();
        let Ok(output) = output else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut out = Vec::new();
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let (source, tail) = if let Some(rest) = trimmed.strip_prefix("Global:") {
                ("resolvectl-global".to_string(), rest.trim().to_string())
            } else if trimmed.starts_with("Link ") {
                let source = trimmed
                    .split(':')
                    .next()
                    .map(|s| format!("resolvectl-{}", s.replace(' ', "_")))
                    .unwrap_or_else(|| "resolvectl-link".to_string());
                let tail = trimmed
                    .split(':')
                    .nth(1)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                (source, tail)
            } else {
                ("resolvectl".to_string(), trimmed.to_string())
            };

            for token in tail.split_whitespace() {
                if let Some(ip) = parse_ip_candidate(token) {
                    out.push(DnsServerInfo {
                        address: ip.to_string(),
                        source: source.clone(),
                    });
                }
            }
        }
        out
    }

    fn read_interface_details(&self, iface: &str) -> InterfaceDetails {
        let base = Path::new("/sys/class/net").join(iface);
        let device = base.join("device");
        let status_raw = read_trimmed(base.join("operstate")).unwrap_or_else(|| "unknown".to_string());
        let status = map_operstate(&status_raw);

        let link_speed = read_trimmed(base.join("speed"))
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|v| *v > 0)
            .map(format_link_speed)
            .unwrap_or_else(|| "Unknown".to_string());

        let duplex = read_trimmed(base.join("duplex"))
            .map(|d| title_case_ascii(&d))
            .unwrap_or_else(|| "Unknown".to_string());

        let mtu = read_trimmed(base.join("mtu"))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1500);

        let mac_address = read_trimmed(base.join("address")).unwrap_or_default();

        let driver = fs::read_link(device.join("driver"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));

        let vendor = read_trimmed(device.join("vendor")).map(normalize_hex_id);
        let model = read_trimmed(device.join("device")).map(normalize_hex_id);
        let bus_info = read_uevent_value(device.join("uevent"), "PCI_SLOT_NAME")
            .or_else(|| read_uevent_value(device.join("uevent"), "OF_FULLNAME"));
        let iface_kind = if base.join("wireless").exists() {
            "Wi-Fi".to_string()
        } else if iface == "lo" {
            "Loopback".to_string()
        } else {
            "Ethernet".to_string()
        };

        let mut fields = vec![iface_kind];
        if let Some(d) = &driver {
            fields.push(format!("driver {d}"));
        }
        if let (Some(v), Some(m)) = (&vendor, &model) {
            fields.push(format!("pci {v}:{m}"));
        }
        if let Some(bus) = &bus_info {
            fields.push(format!("bus {bus}"));
        }

        InterfaceDetails {
            description: fields.join(", "),
            status,
            link_speed,
            mac_address,
            mtu,
            duplex,
        }
    }

    fn collect_socket_owners(&self) -> HashMap<u64, (u32, String)> {
        let mut owners = HashMap::new();
        let Ok(proc_entries) = fs::read_dir("/proc") else {
            return owners;
        };

        for entry in proc_entries.flatten() {
            let pid = entry
                .file_name()
                .to_string_lossy()
                .parse::<u32>()
                .ok();
            let Some(pid) = pid else {
                continue;
            };

            let proc_name = fs::read_to_string(format!("/proc/{pid}/comm"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "unknown".to_string());

            let fd_dir = format!("/proc/{pid}/fd");
            let Ok(fds) = fs::read_dir(fd_dir) else {
                continue;
            };

            for fd in fds.flatten() {
                let Ok(target) = fs::read_link(fd.path()) else {
                    continue;
                };
                let target = target.to_string_lossy();
                if let Some(inode) = parse_socket_inode(&target) {
                    owners.entry(inode).or_insert((pid, proc_name.clone()));
                }
            }
        }

        owners
    }

    fn append_connections_from_proc(
        &self,
        path: &str,
        protocol: &str,
        is_ipv6: bool,
        owners: &HashMap<u64, (u32, String)>,
        out: &mut Vec<NetworkConnectionInfo>,
    ) -> Result<()> {
        let content = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
        for line in content.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 {
                continue;
            }

            let Some((local_address, local_port)) = parse_endpoint(fields[1], is_ipv6) else {
                continue;
            };
            let Some((remote_address, remote_port)) = parse_endpoint(fields[2], is_ipv6) else {
                continue;
            };

            let inode = fields[9].parse::<u64>().unwrap_or(0);
            let (pid, process_name) = owners
                .get(&inode)
                .cloned()
                .unwrap_or((0, "Unknown".to_string()));

            let state = if protocol.starts_with("TCP") {
                map_tcp_state(fields[3]).to_string()
            } else {
                map_udp_state(fields[3]).to_string()
            };

            out.push(NetworkConnectionInfo {
                process_name,
                pid,
                protocol: protocol.to_string(),
                local_address,
                local_port,
                remote_address,
                remote_port,
                state,
            });
        }
        Ok(())
    }

    fn read_process_bandwidth_from_ss(&self) -> Vec<ProcessBandwidthInfo> {
        let output = Command::new("ss").args(["-tinpH"]).output();
        let Ok(output) = output else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut current_owner: Option<(u32, String)> = None;
        let mut usage: HashMap<u32, ProcessBandwidthInfo> = HashMap::new();

        for raw in stdout.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }

            let is_header_line = !raw.starts_with(' ') && !raw.starts_with('\t');
            if is_header_line {
                current_owner = parse_ss_owner(line);
            }

            if let Some((pid, name)) = current_owner.clone() {
                let sent = parse_number_after(line, "bytes_sent:")
                    .or_else(|| parse_number_after(line, "bytes_acked:"))
                    .unwrap_or(0);
                let recv = parse_number_after(line, "bytes_received:").unwrap_or(0);

                if sent > 0 || recv > 0 {
                    let entry = usage.entry(pid).or_insert(ProcessBandwidthInfo {
                        pid,
                        name: name.clone(),
                        bytes_received: 0,
                        bytes_sent: 0,
                        estimated: false,
                    });
                    entry.bytes_sent = entry.bytes_sent.saturating_add(sent);
                    entry.bytes_received = entry.bytes_received.saturating_add(recv);
                }
            }
        }

        usage.into_values().collect()
    }

    fn read_process_bandwidth_from_socket_queues(&self) -> Vec<ProcessBandwidthInfo> {
        let owners = self.collect_socket_owners();
        let mut per_pid: HashMap<u32, ProcessBandwidthInfo> = HashMap::new();

        for path in ["/proc/net/tcp", "/proc/net/tcp6", "/proc/net/udp", "/proc/net/udp6"] {
            let Ok(content) = fs::read_to_string(path) else {
                continue;
            };

            for line in content.lines().skip(1) {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.len() < 10 {
                    continue;
                }

                let inode = fields[9].parse::<u64>().unwrap_or(0);
                let Some((pid, name)) = owners.get(&inode).cloned() else {
                    continue;
                };

                let (txq, rxq) = parse_tx_rx_queue(fields[4]).unwrap_or((0, 0));
                let entry = per_pid.entry(pid).or_insert(ProcessBandwidthInfo {
                    pid,
                    name,
                    bytes_received: 0,
                    bytes_sent: 0,
                    estimated: true,
                });
                entry.bytes_sent = entry.bytes_sent.saturating_add(txq as u64);
                entry.bytes_received = entry.bytes_received.saturating_add(rxq as u64);
            }
        }

        per_pid.into_values().collect()
    }
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_uevent_value(path: impl AsRef<Path>, key: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k == key {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn normalize_iface_name(raw: &str) -> String {
    raw.split('@').next().unwrap_or(raw).trim().to_string()
}

fn normalize_hex_id(raw: String) -> String {
    raw.trim_start_matches("0x").to_lowercase()
}

fn parse_ipv4_hex_le(hex: &str) -> Option<String> {
    if hex.len() != 8 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    let b1 = (v & 0xff) as u8;
    let b2 = ((v >> 8) & 0xff) as u8;
    let b3 = ((v >> 16) & 0xff) as u8;
    let b4 = ((v >> 24) & 0xff) as u8;
    Some(format!("{b1}.{b2}.{b3}.{b4}"))
}

fn parse_ipv6_network_order(hex: &str) -> Option<String> {
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&hex[(i * 2)..(i * 2 + 2)], 16).ok()?;
    }
    Some(std::net::Ipv6Addr::from(out).to_string())
}

fn parse_ipv6_proc_order(hex: &str) -> Option<String> {
    if hex.len() != 32 {
        return None;
    }
    let mut raw = [0u8; 16];
    for i in 0..16 {
        raw[i] = u8::from_str_radix(&hex[(i * 2)..(i * 2 + 2)], 16).ok()?;
    }

    let mut out = [0u8; 16];
    for (chunk_idx, chunk) in raw.chunks_exact(4).enumerate() {
        let target = &mut out[(chunk_idx * 4)..(chunk_idx * 4 + 4)];
        target[0] = chunk[3];
        target[1] = chunk[2];
        target[2] = chunk[1];
        target[3] = chunk[0];
    }

    Some(std::net::Ipv6Addr::from(out).to_string())
}

fn parse_endpoint(endpoint: &str, is_ipv6: bool) -> Option<(String, u16)> {
    let (raw_ip, raw_port) = endpoint.split_once(':')?;
    let port = u16::from_str_radix(raw_port, 16).ok()?;
    let ip = if is_ipv6 {
        parse_ipv6_proc_order(raw_ip)?
    } else {
        parse_ipv4_hex_le(raw_ip)?
    };
    Some((ip, port))
}

fn parse_socket_inode(link_target: &str) -> Option<u64> {
    let inner = link_target
        .strip_prefix("socket:[")?
        .strip_suffix(']')?;
    inner.parse::<u64>().ok()
}

fn parse_tx_rx_queue(raw: &str) -> Option<(u32, u32)> {
    let (tx, rx) = raw.split_once(':')?;
    let tx = u32::from_str_radix(tx, 16).ok()?;
    let rx = u32::from_str_radix(rx, 16).ok()?;
    Some((tx, rx))
}

fn map_tcp_state(code: &str) -> &'static str {
    match code {
        "01" => "ESTABLISHED",
        "02" => "SYN_SENT",
        "03" => "SYN_RECV",
        "04" => "FIN_WAIT1",
        "05" => "FIN_WAIT2",
        "06" => "TIME_WAIT",
        "07" => "CLOSE",
        "08" => "CLOSE_WAIT",
        "09" => "LAST_ACK",
        "0A" => "LISTEN",
        "0B" => "CLOSING",
        "0C" => "NEW_SYN_RECV",
        _ => "UNKNOWN",
    }
}

fn map_udp_state(code: &str) -> &'static str {
    match code {
        "07" => "UNCONN",
        "0A" => "LISTEN",
        _ => "UNKNOWN",
    }
}

fn state_rank(state: &str) -> u8 {
    match state {
        "ESTABLISHED" => 0,
        "SYN_SENT" | "SYN_RECV" => 1,
        "LISTEN" => 2,
        "TIME_WAIT" | "CLOSE_WAIT" => 3,
        _ => 4,
    }
}

fn map_operstate(raw: &str) -> String {
    match raw {
        "up" => "Connected".to_string(),
        "down" => "Disconnected".to_string(),
        "dormant" => "Dormant".to_string(),
        "lowerlayerdown" => "LowerLayerDown".to_string(),
        "testing" => "Testing".to_string(),
        _ => "Unknown".to_string(),
    }
}

fn format_link_speed(mbps: i64) -> String {
    if mbps >= 1000 {
        let gbps = mbps as f64 / 1000.0;
        if (gbps - gbps.round()).abs() < 0.01 {
            format!("{:.0} Gbps", gbps)
        } else {
            format!("{:.1} Gbps", gbps)
        }
    } else {
        format!("{mbps} Mbps")
    }
}

fn title_case_ascii(raw: &str) -> String {
    if raw.is_empty() {
        return "Unknown".to_string();
    }
    let mut chars = raw.chars();
    let first = chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_default();
    let rest = chars.as_str().to_ascii_lowercase();
    format!("{first}{rest}")
}

fn parse_ip_candidate(token: &str) -> Option<IpAddr> {
    let stripped = token
        .trim()
        .trim_matches('[')
        .trim_matches(']')
        .trim_end_matches(',');
    stripped.parse::<IpAddr>().ok()
}

fn dedup_dns_servers(servers: Vec<DnsServerInfo>) -> Vec<DnsServerInfo> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for s in servers {
        if seen.insert(s.address.clone()) {
            out.push(s);
        }
    }
    out
}

fn sanitize_resolve_target(query: &str) -> String {
    let mut raw = query.trim().to_string();
    if let Some(pos) = raw.find("://") {
        raw = raw[(pos + 3)..].to_string();
    }
    if let Some(pos) = raw.find('/') {
        raw = raw[..pos].to_string();
    }

    if raw.starts_with('[') && raw.ends_with(']') {
        return raw.trim_matches(&['[', ']'][..]).to_string();
    }

    if raw.matches(':').count() == 1 {
        if let Some((host, port)) = raw.split_once(':') {
            if !host.is_empty() && port.parse::<u16>().is_ok() {
                return host.to_string();
            }
        }
    }

    raw
}

fn parse_ss_owner(line: &str) -> Option<(u32, String)> {
    let users_pos = line.find("users:(")?;
    let users = &line[users_pos..];

    let pid_pos = users.find("pid=")?;
    let pid_raw = &users[(pid_pos + 4)..];
    let pid: u32 = pid_raw
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;

    let name = users
        .find("((")
        .and_then(|start| {
            let s = &users[(start + 2)..];
            if let Some(rest) = s.strip_prefix('"') {
                let end = rest.find('"')?;
                Some(rest[..end].to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "Unknown".to_string());

    Some((pid, name))
}

fn parse_number_after(line: &str, key: &str) -> Option<u64> {
    let pos = line.find(key)?;
    let after = &line[(pos + key.len())..];
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ipv4_hex_le() {
        assert_eq!(parse_ipv4_hex_le("0100007F").as_deref(), Some("127.0.0.1"));
        assert_eq!(parse_ipv4_hex_le("C0A80101").as_deref(), Some("1.1.168.192"));
    }

    #[test]
    fn test_parse_socket_inode() {
        assert_eq!(parse_socket_inode("socket:[12345]"), Some(12345));
        assert_eq!(parse_socket_inode("pipe:[12345]"), None);
    }

    #[test]
    fn test_format_link_speed() {
        assert_eq!(format_link_speed(100), "100 Mbps");
        assert_eq!(format_link_speed(1000), "1 Gbps");
        assert_eq!(format_link_speed(2500), "2.5 Gbps");
    }

    #[test]
    fn test_sanitize_resolve_target() {
        assert_eq!(sanitize_resolve_target("https://example.com/path"), "example.com");
        assert_eq!(sanitize_resolve_target("example.com:443"), "example.com");
        assert_eq!(sanitize_resolve_target("[2001:db8::1]"), "2001:db8::1");
    }
}
