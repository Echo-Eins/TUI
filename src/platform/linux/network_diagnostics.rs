use super::LinuxSysMonitor;
use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::net::IpAddr;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticsOperation {
    Resolve,
    DnsExplain,
    Ping,
    Trace,
    MtuProbe,
    PortScan,
    NatCapabilityCheck,
    MappingTest,
    ExportReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceProtocol {
    Icmp,
    Udp,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MappingProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportFormat {
    Json,
    Markdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveRequest {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsExplainRequest {
    pub include_gateways: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingRequest {
    pub target: String,
    pub count: u32,
    pub timeout_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRequest {
    pub target: String,
    pub protocol: TraceProtocol,
    pub max_hops: u8,
    pub timeout_secs: u8,
    pub per_hop_queries: u8,
    pub port: Option<u16>,
    pub resolve_names: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtuProbeRequest {
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortScanRequest {
    pub target: String,
    pub ports: Vec<u16>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatCapabilityRequest {
    pub timeout_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatMappingTestRequest {
    pub protocol: MappingProtocol,
    pub internal_port: u16,
    pub external_port: u16,
    pub ttl_seconds: u32,
    pub require_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReportRequest {
    pub format: ReportFormat,
    pub max_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkDiagnosticsRequest {
    Resolve(ResolveRequest),
    DnsExplain(DnsExplainRequest),
    Ping(PingRequest),
    Trace(TraceRequest),
    MtuProbe(MtuProbeRequest),
    PortScan(PortScanRequest),
    NatCapabilityCheck(NatCapabilityRequest),
    MappingTest(NatMappingTestRequest),
    ExportReport(ExportReportRequest),
}

impl NetworkDiagnosticsRequest {
    pub fn operation(&self) -> DiagnosticsOperation {
        match self {
            Self::Resolve(_) => DiagnosticsOperation::Resolve,
            Self::DnsExplain(_) => DiagnosticsOperation::DnsExplain,
            Self::Ping(_) => DiagnosticsOperation::Ping,
            Self::Trace(_) => DiagnosticsOperation::Trace,
            Self::MtuProbe(_) => DiagnosticsOperation::MtuProbe,
            Self::PortScan(_) => DiagnosticsOperation::PortScan,
            Self::NatCapabilityCheck(_) => DiagnosticsOperation::NatCapabilityCheck,
            Self::MappingTest(_) => DiagnosticsOperation::MappingTest,
            Self::ExportReport(_) => DiagnosticsOperation::ExportReport,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveResult {
    pub query: String,
    pub host: String,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsServerRecord {
    pub address: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayRecord {
    pub interface: String,
    pub address: String,
    pub metric: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsExplainResult {
    pub resolver_mode: String,
    pub dns_servers: Vec<DnsServerRecord>,
    pub search_domains: Vec<String>,
    pub default_gateways: Vec<GatewayRecord>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingSummary {
    pub target: String,
    pub transmitted: u32,
    pub received: u32,
    pub packet_loss_percent: f32,
    pub avg_latency_ms: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceHop {
    pub hop: u8,
    pub host: Option<String>,
    pub address: Option<String>,
    pub rtt_ms: Vec<f32>,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSummary {
    pub target: String,
    pub protocol: TraceProtocol,
    pub hops: Vec<TraceHop>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceMtuRecord {
    pub interface: String,
    pub status: String,
    pub ipv4: String,
    pub mtu: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtuProbeResult {
    pub target: String,
    pub path_mtu: Option<u32>,
    pub interfaces: Vec<InterfaceMtuRecord>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortScanResult {
    pub target: String,
    pub scanned_ports: Vec<u16>,
    pub open_ports: Vec<u16>,
    pub timeout_ms: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityState {
    Supported,
    Unavailable,
    PermissionDenied,
    MissingDependency,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatMethodCapability {
    pub method: String,
    pub state: CapabilityState,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatCapabilityResult {
    pub external_ip: Option<String>,
    pub methods: Vec<NatMethodCapability>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatMappingTestResult {
    pub protocol: MappingProtocol,
    pub local_address: Option<String>,
    pub internal_port: u16,
    pub external_port: u16,
    pub created: bool,
    pub visible_in_gateway_table: bool,
    pub removed: bool,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReportResult {
    pub format: ReportFormat,
    pub content: String,
    pub entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkDiagnosticsResult {
    Resolve(ResolveResult),
    DnsExplain(DnsExplainResult),
    Ping(PingSummary),
    Trace(TraceSummary),
    MtuProbe(MtuProbeResult),
    PortScan(PortScanResult),
    NatCapabilityCheck(NatCapabilityResult),
    MappingTest(NatMappingTestResult),
    ExportReport(ExportReportResult),
}

impl NetworkDiagnosticsResult {
    pub fn summary(&self) -> String {
        match self {
            Self::Resolve(r) => {
                if r.addresses.is_empty() {
                    format!("Resolve: {} -> no addresses", r.host)
                } else {
                    format!(
                        "Resolve: {} -> {}",
                        r.host,
                        r.addresses
                            .iter()
                            .take(3)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            Self::DnsExplain(r) => format!(
                "DNS: {} servers, {} domains, mode {}",
                r.dns_servers.len(),
                r.search_domains.len(),
                r.resolver_mode
            ),
            Self::Ping(r) => format!(
                "Ping: loss {:.1}% avg {} ms",
                r.packet_loss_percent,
                r.avg_latency_ms
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "n/a".to_string())
            ),
            Self::Trace(r) => format!("Trace: {} hops collected", r.hops.len()),
            Self::MtuProbe(r) => match r.path_mtu {
                Some(v) => format!("MTU: path MTU to {} is {}", r.target, v),
                None => format!("MTU: could not determine path MTU to {}", r.target),
            },
            Self::PortScan(r) => format!(
                "Port scan: {} open of {} checked",
                r.open_ports.len(),
                r.scanned_ports.len()
            ),
            Self::NatCapabilityCheck(r) => format!(
                "NAT: methods {}, external IP {}",
                r.methods.len(),
                r.external_ip
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            Self::MappingTest(r) => format!(
                "Mapping test: created={} listed={} removed={}",
                r.created, r.visible_in_gateway_table, r.removed
            ),
            Self::ExportReport(r) => {
                format!("Export: {} entries ({:?})", r.entries, r.format)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkDiagnosticsErrorCode {
    InvalidInput,
    PermissionDenied,
    DependencyMissing,
    Timeout,
    Cancelled,
    ExecutionFailed,
    ParseFailed,
    Unsupported,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkDiagnosticsError {
    pub code: NetworkDiagnosticsErrorCode,
    pub message: String,
    pub hint: Option<String>,
}

impl NetworkDiagnosticsError {
    fn new(code: NetworkDiagnosticsErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
        }
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkDiagnosticsJob {
    pub id: u64,
    pub operation: DiagnosticsOperation,
    pub started_unix_ms: u64,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkDiagnosticsEvent {
    Started {
        job: NetworkDiagnosticsJob,
    },
    Progress {
        job_id: u64,
        message: String,
    },
    Completed {
        job_id: u64,
        result: NetworkDiagnosticsResult,
    },
    Failed {
        job_id: u64,
        error: NetworkDiagnosticsError,
    },
    Cancelled {
        job_id: u64,
    },
}

#[derive(Debug, Serialize)]
struct ReportEntry {
    job_id: u64,
    result: NetworkDiagnosticsResult,
}

#[derive(Debug, Serialize)]
struct DiagnosticsReport {
    generated_unix_ms: u64,
    entries: Vec<ReportEntry>,
}

struct CommandRunResult {
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

pub struct NetworkDiagnosticsEngine {
    next_job_id: AtomicU64,
    subscribers: Mutex<Vec<UnboundedSender<NetworkDiagnosticsEvent>>>,
    running_jobs: Mutex<HashMap<u64, tokio::task::JoinHandle<()>>>,
    completed_results: Mutex<VecDeque<(u64, NetworkDiagnosticsResult)>>,
}

impl Default for NetworkDiagnosticsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkDiagnosticsEngine {
    pub fn new() -> Self {
        Self {
            next_job_id: AtomicU64::new(1),
            subscribers: Mutex::new(Vec::new()),
            running_jobs: Mutex::new(HashMap::new()),
            completed_results: Mutex::new(VecDeque::with_capacity(128)),
        }
    }

    pub fn subscribe(&self) -> UnboundedReceiver<NetworkDiagnosticsEvent> {
        let (tx, rx) = unbounded_channel();
        self.subscribers.lock().push(tx);
        rx
    }

    pub fn start(self: &Arc<Self>, request: NetworkDiagnosticsRequest, timeout: Duration) -> u64 {
        let job_id = self.next_job_id.fetch_add(1, Ordering::Relaxed);
        let job = NetworkDiagnosticsJob {
            id: job_id,
            operation: request.operation(),
            started_unix_ms: now_unix_ms(),
            timeout_ms: timeout.as_millis().min(u64::MAX as u128) as u64,
        };
        self.emit(NetworkDiagnosticsEvent::Started { job });

        let engine = Arc::clone(self);
        let handle = tokio::spawn(async move {
            let output =
                tokio::time::timeout(timeout, engine.execute_request(job_id, request)).await;
            match output {
                Ok(Ok(result)) => {
                    engine.push_completed(job_id, result.clone());
                    engine.emit(NetworkDiagnosticsEvent::Completed { job_id, result });
                }
                Ok(Err(error)) => {
                    engine.emit(NetworkDiagnosticsEvent::Failed { job_id, error });
                }
                Err(_) => {
                    engine.emit(NetworkDiagnosticsEvent::Failed {
                        job_id,
                        error: NetworkDiagnosticsError::new(
                            NetworkDiagnosticsErrorCode::Timeout,
                            "diagnostic task timed out",
                        )
                        .with_hint("Try lower scope (fewer ports/hops) or increase timeout"),
                    });
                }
            }
        });

        self.reap_finished_jobs();
        self.running_jobs.lock().insert(job_id, handle);
        job_id
    }

    pub fn cancel(&self, job_id: u64) -> bool {
        self.reap_finished_jobs();
        if let Some(handle) = self.running_jobs.lock().remove(&job_id) {
            handle.abort();
            self.emit(NetworkDiagnosticsEvent::Cancelled { job_id });
            true
        } else {
            false
        }
    }

    pub fn active_jobs(&self) -> Vec<u64> {
        self.reap_finished_jobs();
        self.running_jobs.lock().keys().copied().collect()
    }

    pub fn recent_results(&self, max_entries: usize) -> Vec<(u64, NetworkDiagnosticsResult)> {
        let limit = max_entries.max(1);
        self.completed_results
            .lock()
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
    }

    fn reap_finished_jobs(&self) {
        self.running_jobs
            .lock()
            .retain(|_, handle| !handle.is_finished());
    }

    fn emit(&self, event: NetworkDiagnosticsEvent) {
        let mut subs = self.subscribers.lock();
        subs.retain(|sender| sender.send(event.clone()).is_ok());
    }

    fn emit_progress(&self, job_id: u64, message: impl Into<String>) {
        self.emit(NetworkDiagnosticsEvent::Progress {
            job_id,
            message: message.into(),
        });
    }

    fn push_completed(&self, job_id: u64, result: NetworkDiagnosticsResult) {
        let mut completed = self.completed_results.lock();
        if completed.len() >= 128 {
            completed.pop_front();
        }
        completed.push_back((job_id, result));
    }

    async fn execute_request(
        &self,
        job_id: u64,
        request: NetworkDiagnosticsRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        match request {
            NetworkDiagnosticsRequest::Resolve(req) => self.execute_resolve(job_id, req).await,
            NetworkDiagnosticsRequest::DnsExplain(req) => {
                self.execute_dns_explain(job_id, req).await
            }
            NetworkDiagnosticsRequest::Ping(req) => self.execute_ping(job_id, req).await,
            NetworkDiagnosticsRequest::Trace(req) => self.execute_trace(job_id, req).await,
            NetworkDiagnosticsRequest::MtuProbe(req) => self.execute_mtu_probe(job_id, req).await,
            NetworkDiagnosticsRequest::PortScan(req) => self.execute_port_scan(job_id, req).await,
            NetworkDiagnosticsRequest::NatCapabilityCheck(req) => {
                self.execute_nat_capability(job_id, req).await
            }
            NetworkDiagnosticsRequest::MappingTest(req) => {
                self.execute_mapping_test(job_id, req).await
            }
            NetworkDiagnosticsRequest::ExportReport(req) => {
                self.execute_export_report(job_id, req).await
            }
        }
    }

    async fn execute_resolve(
        &self,
        job_id: u64,
        request: ResolveRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        let query = request.query.trim().to_string();
        if query.is_empty() {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::InvalidInput,
                "resolve target is empty",
            ));
        }
        self.emit_progress(job_id, format!("Resolving {query}"));

        let resolved = run_blocking(move || LinuxSysMonitor::new().resolve_host(&query))
            .await
            .map_err(map_anyhow_error)?;

        Ok(NetworkDiagnosticsResult::Resolve(ResolveResult {
            query: resolved.query,
            host: resolved.host,
            addresses: resolved.addresses,
        }))
    }

    async fn execute_dns_explain(
        &self,
        job_id: u64,
        request: DnsExplainRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        self.emit_progress(job_id, "Collecting DNS and resolver details");

        let snapshot = run_blocking(move || {
            let monitor = LinuxSysMonitor::new();
            let dns_servers = monitor.get_dns_servers().unwrap_or_default();
            let search_domains = monitor.get_dns_search_domains().unwrap_or_default();
            let gateways = monitor.get_default_gateways().unwrap_or_default();
            Ok((dns_servers, search_domains, gateways))
        })
        .await
        .map_err(map_anyhow_error)?;

        let (dns_servers, search_domains, gateways) = snapshot;
        let resolver_mode = detect_resolver_mode();
        let mut warnings = Vec::new();

        if dns_servers.is_empty() {
            warnings.push("No DNS servers detected from resolv.conf/resolvectl".to_string());
        }
        if resolver_mode.contains("systemd-resolved")
            && dns_servers
                .iter()
                .all(|server| matches!(server.address.as_str(), "127.0.0.53" | "127.0.0.1" | "::1"))
        {
            warnings.push(
                "Stub resolver detected (127.0.0.53/127.0.0.1). Upstream DNS may be managed by resolvectl/NetworkManager."
                    .to_string(),
            );
        }

        let dns_servers = dns_servers
            .into_iter()
            .map(|server| DnsServerRecord {
                address: server.address,
                source: server.source,
            })
            .collect::<Vec<_>>();

        let default_gateways = if request.include_gateways {
            gateways
                .into_iter()
                .map(|gateway| GatewayRecord {
                    interface: gateway.interface,
                    address: gateway.address,
                    metric: gateway.metric,
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(NetworkDiagnosticsResult::DnsExplain(DnsExplainResult {
            resolver_mode,
            dns_servers,
            search_domains,
            default_gateways,
            warnings,
        }))
    }

    async fn execute_ping(
        &self,
        job_id: u64,
        request: PingRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        let target = request.target.trim().to_string();
        if target.is_empty() {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::InvalidInput,
                "ping target is empty",
            ));
        }
        let count = request.count.clamp(1, 20);
        let timeout_secs = request.timeout_secs.clamp(1, 10);

        self.emit_progress(
            job_id,
            format!("Running ping to {target} ({count} probes, {timeout_secs}s timeout)"),
        );

        let target_clone = target.clone();
        let ping = run_blocking(move || {
            LinuxSysMonitor::new().ping_host(&target_clone, count, timeout_secs)
        })
        .await
        .map_err(map_anyhow_error)?;

        Ok(NetworkDiagnosticsResult::Ping(PingSummary {
            target,
            transmitted: ping.transmitted,
            received: ping.received,
            packet_loss_percent: ping.packet_loss_percent,
            avg_latency_ms: ping.avg_latency_ms,
        }))
    }

    async fn execute_trace(
        &self,
        job_id: u64,
        request: TraceRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        let target = request.target.trim().to_string();
        if target.is_empty() {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::InvalidInput,
                "trace target is empty",
            ));
        }
        let max_hops = request.max_hops.clamp(1, 64);
        let timeout_secs = request.timeout_secs.clamp(1, 10);
        let probes = request.per_hop_queries.clamp(1, 5);

        self.emit_progress(job_id, format!("Tracing route to {target}"));

        let mut args = vec![
            "-m".to_string(),
            max_hops.to_string(),
            "-w".to_string(),
            timeout_secs.to_string(),
            "-q".to_string(),
            probes.to_string(),
        ];
        if !request.resolve_names {
            args.push("-n".to_string());
        }
        match request.protocol {
            TraceProtocol::Icmp => args.push("-I".to_string()),
            TraceProtocol::Udp => args.push("-U".to_string()),
            TraceProtocol::Tcp => {
                args.push("-T".to_string());
                let port = request.port.unwrap_or(443).clamp(1, u16::MAX);
                args.push("-p".to_string());
                args.push(port.to_string());
            }
        }
        args.push(target.clone());

        let timeout =
            Duration::from_secs((timeout_secs as u64).saturating_mul(max_hops as u64).max(8));
        let output = run_command("traceroute", &args, timeout).await?;
        let hops = parse_traceroute_output(&output.stdout);
        let mut warnings = Vec::new();

        if hops.is_empty() {
            warnings.push("No hops parsed from traceroute output".to_string());
        }
        if !output.stderr.trim().is_empty() {
            warnings.push(first_line(&output.stderr));
        }
        if output.status_code.unwrap_or(1) != 0 && warnings.is_empty() {
            warnings.push("traceroute exited with non-zero status".to_string());
        }

        Ok(NetworkDiagnosticsResult::Trace(TraceSummary {
            target,
            protocol: request.protocol,
            hops,
            warnings,
        }))
    }

    async fn execute_mtu_probe(
        &self,
        job_id: u64,
        request: MtuProbeRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        let target = request.target.trim().to_string();
        if target.is_empty() {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::InvalidInput,
                "MTU probe target is empty",
            ));
        }
        self.emit_progress(job_id, format!("Probing MTU path to {target}"));

        let target_clone = target.clone();
        let snapshot = run_blocking(move || {
            let monitor = LinuxSysMonitor::new();
            let path_mtu = monitor.detect_path_mtu(&target_clone)?;
            let interfaces = monitor.get_network_interfaces_stats().unwrap_or_default();
            Ok((path_mtu, interfaces))
        })
        .await
        .map_err(map_anyhow_error)?;

        let (path_mtu, interfaces) = snapshot;
        let mut interface_mtu = interfaces
            .into_iter()
            .map(|iface| InterfaceMtuRecord {
                interface: iface.name,
                status: iface.status,
                ipv4: iface.ipv4_address,
                mtu: iface.mtu,
            })
            .collect::<Vec<_>>();
        interface_mtu.sort_by(|a, b| a.interface.cmp(&b.interface));

        let warning = if path_mtu.is_none() {
            Some(
                "Path MTU probe failed. Target may block ICMP fragmentation-needed replies."
                    .to_string(),
            )
        } else {
            None
        };

        Ok(NetworkDiagnosticsResult::MtuProbe(MtuProbeResult {
            target,
            path_mtu,
            interfaces: interface_mtu,
            warning,
        }))
    }

    async fn execute_port_scan(
        &self,
        job_id: u64,
        request: PortScanRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        let target = request.target.trim().to_string();
        if target.is_empty() {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::InvalidInput,
                "port scan target is empty",
            ));
        }
        let mut ports = request
            .ports
            .into_iter()
            .filter(|port| *port > 0)
            .collect::<Vec<_>>();
        ports.sort_unstable();
        ports.dedup();
        if ports.is_empty() {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::InvalidInput,
                "port scan ports list is empty",
            ));
        }
        if ports.len() > 256 {
            ports.truncate(256);
        }

        let timeout_ms = request.timeout_ms.clamp(100, 5000);
        self.emit_progress(
            job_id,
            format!("Scanning {} TCP ports on {target}", ports.len()),
        );

        let start = Instant::now();
        let target_clone = target.clone();
        let ports_for_scan = ports.clone();
        let open = run_blocking(move || {
            LinuxSysMonitor::new().scan_tcp_ports(
                &target_clone,
                &ports_for_scan,
                Duration::from_millis(timeout_ms),
            )
        })
        .await
        .map_err(map_anyhow_error)?;
        let duration_ms = start.elapsed().as_millis().min(u64::MAX as u128) as u64;

        Ok(NetworkDiagnosticsResult::PortScan(PortScanResult {
            target,
            scanned_ports: ports,
            open_ports: open,
            timeout_ms,
            duration_ms,
        }))
    }

    async fn execute_nat_capability(
        &self,
        job_id: u64,
        request: NatCapabilityRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        let timeout = Duration::from_secs(request.timeout_secs.clamp(2, 20) as u64);
        self.emit_progress(job_id, "Checking NAT gateway capabilities");

        let mut methods = Vec::new();
        let mut warnings = Vec::new();
        let mut external_ip = None;

        let upnp = self.check_upnp(timeout).await;
        if external_ip.is_none() {
            external_ip = upnp.1;
        }
        methods.push(upnp.0);

        let nat_pmp = self.check_nat_pmp(timeout).await;
        if external_ip.is_none() {
            external_ip = nat_pmp.1;
        }
        methods.push(nat_pmp.0);

        let pcp = self.check_pcp(timeout).await;
        methods.push(pcp);

        if methods
            .iter()
            .all(|method| method.state == CapabilityState::MissingDependency)
        {
            warnings.push("No NAT client tools found (upnpc/natpmpc/pcp-client).".to_string());
        }

        Ok(NetworkDiagnosticsResult::NatCapabilityCheck(
            NatCapabilityResult {
                external_ip,
                methods,
                warnings,
            },
        ))
    }

    async fn execute_mapping_test(
        &self,
        job_id: u64,
        request: NatMappingTestRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        if !request.require_confirmation {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::InvalidInput,
                "active NAT mapping test requires explicit confirmation",
            )
            .with_hint(
                "Set require_confirmation=true only when you are ready to create and remove a temporary mapping",
            ));
        }
        if !command_exists("upnpc") {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::DependencyMissing,
                "upnpc tool is not available",
            )
            .with_hint("Install miniupnpc to enable active UPnP mapping tests"));
        }

        self.emit_progress(job_id, "Preparing temporary NAT mapping test");

        let interfaces =
            run_blocking(move || LinuxSysMonitor::new().get_network_interfaces_stats())
                .await
                .map_err(map_anyhow_error)?;
        let local_ip = pick_primary_ipv4(&interfaces).ok_or_else(|| {
            NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::Unsupported,
                "no primary IPv4 interface found for mapping test",
            )
            .with_hint("Ensure interface has IPv4 + default gateway")
        })?;

        let proto = match request.protocol {
            MappingProtocol::Tcp => "TCP",
            MappingProtocol::Udp => "UDP",
        };
        let ttl = request.ttl_seconds.clamp(30, 3600);
        let add_args = vec![
            "-e".to_string(),
            "cardputer-remote-netdiag".to_string(),
            "-a".to_string(),
            local_ip.clone(),
            request.internal_port.to_string(),
            request.external_port.to_string(),
            proto.to_string(),
            ttl.to_string(),
        ];

        self.emit_progress(
            job_id,
            format!(
                "Creating temporary {proto} mapping {} -> {}",
                request.external_port, request.internal_port
            ),
        );
        let add = run_command("upnpc", &add_args, Duration::from_secs(15)).await?;
        let created = add.status_code == Some(0)
            || contains_case_insensitive(&add.stdout, "is redirected")
            || contains_case_insensitive(&add.stdout, "AddPortMapping");

        let mut details = Vec::new();
        details.push(format!("add: {}", first_line(&combine_out(&add))));

        let list = run_command("upnpc", &["-l".to_string()], Duration::from_secs(12)).await?;
        let needle = format!("{} {} ", request.external_port, proto);
        let visible = contains_case_insensitive(&list.stdout, &needle);
        details.push(format!("list: {}", first_line(&combine_out(&list))));

        let del_args = vec![
            "-d".to_string(),
            request.external_port.to_string(),
            proto.to_string(),
        ];
        let del = run_command("upnpc", &del_args, Duration::from_secs(12)).await?;
        let removed = del.status_code == Some(0)
            || contains_case_insensitive(&del.stdout, "DeletePortMapping")
            || contains_case_insensitive(&del.stdout, "removed");
        details.push(format!("delete: {}", first_line(&combine_out(&del))));

        Ok(NetworkDiagnosticsResult::MappingTest(
            NatMappingTestResult {
                protocol: request.protocol,
                local_address: Some(local_ip),
                internal_port: request.internal_port,
                external_port: request.external_port,
                created,
                visible_in_gateway_table: visible,
                removed,
                details,
            },
        ))
    }

    async fn execute_export_report(
        &self,
        job_id: u64,
        request: ExportReportRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        let max_entries = request.max_entries.clamp(1, 128);
        self.emit_progress(
            job_id,
            format!("Exporting diagnostics report ({max_entries} entries)"),
        );

        let content = self
            .render_report(request.format, max_entries)
            .map_err(map_anyhow_error)?;
        let entries = self.recent_results(max_entries).len();

        Ok(NetworkDiagnosticsResult::ExportReport(ExportReportResult {
            format: request.format,
            content,
            entries,
        }))
    }

    fn render_report(&self, format: ReportFormat, max_entries: usize) -> Result<String> {
        let entries = self
            .recent_results(max_entries)
            .into_iter()
            .map(|(job_id, result)| ReportEntry { job_id, result })
            .collect::<Vec<_>>();

        let report = DiagnosticsReport {
            generated_unix_ms: now_unix_ms(),
            entries,
        };

        match format {
            ReportFormat::Json => {
                serde_json::to_string_pretty(&report).context("failed to serialize JSON report")
            }
            ReportFormat::Markdown => {
                let mut out = String::new();
                out.push_str("# Network Diagnostics Report\n\n");
                out.push_str(&format!("Generated: `{}`\n\n", report.generated_unix_ms));
                for entry in &report.entries {
                    out.push_str(&format!("## Job {}\n\n", entry.job_id));
                    out.push_str(&format!("Summary: `{}`\n\n", entry.result.summary()));
                    out.push_str("```json\n");
                    out.push_str(
                        &serde_json::to_string_pretty(&entry.result)
                            .context("failed to render markdown report JSON block")?,
                    );
                    out.push_str("\n```\n\n");
                }
                Ok(out)
            }
        }
    }

    async fn check_upnp(&self, timeout: Duration) -> (NatMethodCapability, Option<String>) {
        if !command_exists("upnpc") {
            return (
                NatMethodCapability {
                    method: "UPnP IGD".to_string(),
                    state: CapabilityState::MissingDependency,
                    details: "upnpc not found".to_string(),
                },
                None,
            );
        }

        match run_command("upnpc", &["-s".to_string()], timeout).await {
            Ok(output) => {
                let text = combine_out(&output);
                let external_ip = extract_first_ip(&text);
                let state = if output.status_code == Some(0) {
                    CapabilityState::Supported
                } else if contains_case_insensitive(&text, "permission denied")
                    || contains_case_insensitive(&text, "operation not permitted")
                {
                    CapabilityState::PermissionDenied
                } else {
                    CapabilityState::Unavailable
                };
                (
                    NatMethodCapability {
                        method: "UPnP IGD".to_string(),
                        state,
                        details: first_line(&text),
                    },
                    external_ip,
                )
            }
            Err(error) => (
                NatMethodCapability {
                    method: "UPnP IGD".to_string(),
                    state: match error.code {
                        NetworkDiagnosticsErrorCode::PermissionDenied => {
                            CapabilityState::PermissionDenied
                        }
                        NetworkDiagnosticsErrorCode::DependencyMissing => {
                            CapabilityState::MissingDependency
                        }
                        _ => CapabilityState::Unavailable,
                    },
                    details: error.message,
                },
                None,
            ),
        }
    }

    async fn check_nat_pmp(&self, timeout: Duration) -> (NatMethodCapability, Option<String>) {
        if !command_exists("natpmpc") {
            return (
                NatMethodCapability {
                    method: "NAT-PMP".to_string(),
                    state: CapabilityState::MissingDependency,
                    details: "natpmpc not found".to_string(),
                },
                None,
            );
        }

        match run_command("natpmpc", &["-g".to_string()], timeout).await {
            Ok(output) => {
                let text = combine_out(&output);
                let external_ip = extract_first_ip(&text);
                let state = if output.status_code == Some(0) {
                    CapabilityState::Supported
                } else if contains_case_insensitive(&text, "permission denied")
                    || contains_case_insensitive(&text, "operation not permitted")
                {
                    CapabilityState::PermissionDenied
                } else {
                    CapabilityState::Unavailable
                };
                (
                    NatMethodCapability {
                        method: "NAT-PMP".to_string(),
                        state,
                        details: first_line(&text),
                    },
                    external_ip,
                )
            }
            Err(error) => (
                NatMethodCapability {
                    method: "NAT-PMP".to_string(),
                    state: match error.code {
                        NetworkDiagnosticsErrorCode::PermissionDenied => {
                            CapabilityState::PermissionDenied
                        }
                        NetworkDiagnosticsErrorCode::DependencyMissing => {
                            CapabilityState::MissingDependency
                        }
                        _ => CapabilityState::Unavailable,
                    },
                    details: error.message,
                },
                None,
            ),
        }
    }

    async fn check_pcp(&self, timeout: Duration) -> NatMethodCapability {
        let pcp_cmd = ["pcp-client", "pcp"]
            .into_iter()
            .find(|candidate| command_exists(candidate));

        let Some(program) = pcp_cmd else {
            return NatMethodCapability {
                method: "PCP".to_string(),
                state: CapabilityState::MissingDependency,
                details: "pcp-client/pcp tool not found".to_string(),
            };
        };

        let args = vec!["--help".to_string()];
        match run_command(program, &args, timeout).await {
            Ok(output) => {
                let text = combine_out(&output);
                NatMethodCapability {
                    method: "PCP".to_string(),
                    state: CapabilityState::Unknown,
                    details: format!("client available ({program}), active probe not executed"),
                }
                .tap_if(
                    contains_case_insensitive(&text, "permission denied"),
                    |cap| {
                        cap.state = CapabilityState::PermissionDenied;
                        cap.details = "permission denied while invoking PCP client".to_string();
                    },
                )
            }
            Err(error) => NatMethodCapability {
                method: "PCP".to_string(),
                state: match error.code {
                    NetworkDiagnosticsErrorCode::PermissionDenied => {
                        CapabilityState::PermissionDenied
                    }
                    NetworkDiagnosticsErrorCode::DependencyMissing => {
                        CapabilityState::MissingDependency
                    }
                    _ => CapabilityState::Unavailable,
                },
                details: error.message,
            },
        }
    }
}

async fn run_blocking<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|error| anyhow!("blocking task join error: {error}"))?
}

async fn run_command(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> std::result::Result<CommandRunResult, NetworkDiagnosticsError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output_future = command.output();
    let output = tokio::time::timeout(timeout, output_future)
        .await
        .map_err(|_| {
            NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::Timeout,
                format!("command timed out: {program} {}", args.join(" ")),
            )
        })?
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                NetworkDiagnosticsError::new(
                    NetworkDiagnosticsErrorCode::DependencyMissing,
                    format!("required tool not found: {program}"),
                )
                .with_hint(format!("Install `{program}` package on the target system"))
            } else if error.kind() == std::io::ErrorKind::PermissionDenied {
                NetworkDiagnosticsError::new(
                    NetworkDiagnosticsErrorCode::PermissionDenied,
                    format!("permission denied while starting `{program}`"),
                )
            } else {
                NetworkDiagnosticsError::new(
                    NetworkDiagnosticsErrorCode::ExecutionFailed,
                    format!("failed to run `{program}`: {error}"),
                )
            }
        })?;

    Ok(CommandRunResult {
        status_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn map_anyhow_error(error: anyhow::Error) -> NetworkDiagnosticsError {
    let message = error.to_string();
    if contains_case_insensitive(&message, "permission denied")
        || contains_case_insensitive(&message, "operation not permitted")
    {
        NetworkDiagnosticsError::new(NetworkDiagnosticsErrorCode::PermissionDenied, message)
            .with_hint("Try running with elevated privileges/capabilities")
    } else if contains_case_insensitive(&message, "not found")
        || contains_case_insensitive(&message, "No such file")
    {
        NetworkDiagnosticsError::new(NetworkDiagnosticsErrorCode::DependencyMissing, message)
    } else {
        NetworkDiagnosticsError::new(NetworkDiagnosticsErrorCode::ExecutionFailed, message)
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn detect_resolver_mode() -> String {
    let resolv_path = Path::new("/etc/resolv.conf");
    if let Ok(link) = fs::read_link(resolv_path) {
        let link_txt = normalize_path_text(&link);
        if link_txt.contains("systemd/resolve/stub-resolv.conf") {
            return "systemd-resolved-stub".to_string();
        }
        if link_txt.contains("systemd/resolve/resolv.conf") {
            return "systemd-resolved-direct".to_string();
        }
        if link_txt.contains("NetworkManager") {
            return "networkmanager-managed".to_string();
        }
    }

    if let Ok(content) = fs::read_to_string(resolv_path) {
        if content.contains("127.0.0.53") {
            return "systemd-resolved-stub-inline".to_string();
        }
        if content.contains("Generated by NetworkManager") {
            return "networkmanager-generated".to_string();
        }
    }

    "plain-resolv.conf".to_string()
}

fn normalize_path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn parse_traceroute_output(output: &str) -> Vec<TraceHop> {
    let mut hops = Vec::new();
    for line in output.lines() {
        if let Some(hop) = parse_traceroute_hop_line(line) {
            hops.push(hop);
        }
    }
    hops
}

fn parse_traceroute_hop_line(line: &str) -> Option<TraceHop> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut tokens = trimmed.split_whitespace();
    let hop_num = tokens.next()?.parse::<u8>().ok()?;

    let rest = tokens.collect::<Vec<_>>();
    if rest.is_empty() {
        return Some(TraceHop {
            hop: hop_num,
            host: None,
            address: None,
            rtt_ms: Vec::new(),
            timed_out: true,
        });
    }
    if rest.iter().all(|token| *token == "*") {
        return Some(TraceHop {
            hop: hop_num,
            host: None,
            address: None,
            rtt_ms: Vec::new(),
            timed_out: true,
        });
    }

    let mut host = None;
    let mut address = None;
    let mut idx = 0usize;

    if let Some(first) = rest.first() {
        if first.starts_with('(') && first.ends_with(')') {
            address = Some(first.trim_matches(['(', ')']).to_string());
            idx = 1;
        } else if let Ok(parsed) = first.parse::<IpAddr>() {
            address = Some(parsed.to_string());
            idx = 1;
        } else {
            host = Some((*first).to_string());
            idx = 1;
            if let Some(second) = rest.get(1) {
                if second.starts_with('(') && second.ends_with(')') {
                    address = Some(second.trim_matches(['(', ')']).to_string());
                    idx = 2;
                } else if let Ok(parsed) = second.parse::<IpAddr>() {
                    address = Some(parsed.to_string());
                    idx = 2;
                }
            }
        }
    }

    let mut rtt_ms = Vec::new();
    while idx < rest.len() {
        if let Ok(value) = rest[idx].parse::<f32>() {
            if rest.get(idx + 1).copied() == Some("ms") {
                rtt_ms.push(value);
                idx += 2;
                continue;
            }
            if rest[idx].ends_with("ms") {
                let stripped = rest[idx].trim_end_matches("ms");
                if let Ok(v) = stripped.parse::<f32>() {
                    rtt_ms.push(v);
                }
            }
        } else if rest[idx].ends_with("ms") {
            let stripped = rest[idx].trim_end_matches("ms");
            if let Ok(v) = stripped.parse::<f32>() {
                rtt_ms.push(v);
            }
        }
        idx += 1;
    }

    Some(TraceHop {
        hop: hop_num,
        host,
        address,
        rtt_ms,
        timed_out: false,
    })
}

fn pick_primary_ipv4(interfaces: &[super::network::NetworkInterfaceStats]) -> Option<String> {
    interfaces
        .iter()
        .filter(|iface| !iface.ipv4_address.is_empty())
        .max_by(|a, b| {
            let a_score = interface_score(a);
            let b_score = interface_score(b);
            a_score
                .partial_cmp(&b_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|iface| iface.ipv4_address.clone())
}

fn interface_score(iface: &super::network::NetworkInterfaceStats) -> f64 {
    let mut score = 0.0;
    if iface.name != "lo" {
        score += 1000.0;
    }
    if iface.status.eq_ignore_ascii_case("connected") {
        score += 300.0;
    }
    if iface.gateway.is_some() {
        score += 200.0;
    }
    if !iface.ipv4_address.is_empty() {
        score += 100.0;
    }
    score
}

fn command_exists(program: &str) -> bool {
    let program_path = Path::new(program);
    if program_path.is_absolute() || program.contains(std::path::MAIN_SEPARATOR) {
        return program_path.is_file();
    }

    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(program))
        .any(|candidate| candidate.is_file())
}

fn combine_out(result: &CommandRunResult) -> String {
    let mut out = String::new();
    if !result.stdout.trim().is_empty() {
        out.push_str(result.stdout.trim());
    }
    if !result.stderr.trim().is_empty() {
        if !out.is_empty() {
            out.push_str(" | ");
        }
        out.push_str(result.stderr.trim());
    }
    if out.is_empty() {
        "no output".to_string()
    } else {
        out
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .to_string()
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn extract_first_ip(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        let candidate =
            token.trim_matches(|c: char| c == '[' || c == ']' || c == '(' || c == ')' || c == ',');
        if let Ok(ip) = candidate.parse::<IpAddr>() {
            return Some(ip.to_string());
        }
    }
    None
}

trait TapIf: Sized {
    fn tap_if<F>(self, cond: bool, f: F) -> Self
    where
        F: FnOnce(&mut Self);
}

impl<T> TapIf for T {
    fn tap_if<F>(mut self, cond: bool, f: F) -> Self
    where
        F: FnOnce(&mut Self),
    {
        if cond {
            f(&mut self);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_traceroute_hop_timeout() {
        let hop = parse_traceroute_hop_line(" 3  * * *").expect("hop");
        assert_eq!(hop.hop, 3);
        assert!(hop.timed_out);
        assert!(hop.rtt_ms.is_empty());
    }

    #[test]
    fn parse_traceroute_hop_ipv4() {
        let hop =
            parse_traceroute_hop_line("1  192.168.1.1  1.12 ms  1.03 ms  1.00 ms").expect("hop");
        assert_eq!(hop.hop, 1);
        assert_eq!(hop.address.as_deref(), Some("192.168.1.1"));
        assert_eq!(hop.rtt_ms.len(), 3);
        assert!(!hop.timed_out);
    }

    #[test]
    fn parse_traceroute_hop_host_ip() {
        let hop =
            parse_traceroute_hop_line("2  router.local (10.0.0.1)  2.45 ms  2.35 ms  2.40 ms")
                .expect("hop");
        assert_eq!(hop.host.as_deref(), Some("router.local"));
        assert_eq!(hop.address.as_deref(), Some("10.0.0.1"));
        assert_eq!(hop.rtt_ms.len(), 3);
    }

    #[test]
    fn extract_first_ip_works() {
        assert_eq!(
            extract_first_ip("ExternalIPAddress = 203.0.113.5").as_deref(),
            Some("203.0.113.5")
        );
        assert_eq!(extract_first_ip("no ip here"), None);
    }

    #[test]
    fn command_exists_handles_missing_binary() {
        assert!(!command_exists("__definitely_missing_binary__"));
    }
}
