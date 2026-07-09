use super::LinuxSysMonitor;
use crate::utils::process::run_command_with_timeout;
use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    RouteInspect,
    NicDeepInfo,
    ConnectionLab,
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
pub enum PingProfile {
    Quick,
    Latency,
    Loss,
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
pub struct RouteInspectRequest {
    pub target: Option<String>,
    pub include_policy_rules: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NicDeepInfoRequest {
    pub interface: Option<String>,
    pub include_stats: bool,
    pub include_wifi: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionLabRequest {
    pub protocol_filter: Option<String>,
    pub state_filter: Option<String>,
    pub limit: usize,
    pub include_extended_metrics: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingRequest {
    pub target: String,
    pub profile: PingProfile,
    pub continuous: bool,
    pub count: u32,
    pub timeout_secs: u32,
    pub interval_ms: u32,
    pub deadline_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRequest {
    pub target: String,
    pub protocol: TraceProtocol,
    pub enable_fallback: bool,
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
    RouteInspect(RouteInspectRequest),
    NicDeepInfo(NicDeepInfoRequest),
    ConnectionLab(ConnectionLabRequest),
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
            Self::RouteInspect(_) => DiagnosticsOperation::RouteInspect,
            Self::NicDeepInfo(_) => DiagnosticsOperation::NicDeepInfo,
            Self::ConnectionLab(_) => DiagnosticsOperation::ConnectionLab,
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
    pub port: Option<u16>,
    pub metric: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsExplainResult {
    pub resolver_mode: String,
    pub resolv_conf_path: String,
    pub network_manager_dns_mode: Option<String>,
    pub dns_servers: Vec<DnsServerRecord>,
    pub search_domains: Vec<String>,
    pub split_dns_domains: Vec<String>,
    pub default_gateways: Vec<GatewayRecord>,
    pub conflicts: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRecord {
    pub family: String,
    pub table: String,
    pub destination: String,
    pub gateway: Option<String>,
    pub interface: Option<String>,
    pub metric: Option<u32>,
    pub protocol: Option<String>,
    pub scope: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRuleRecord {
    pub family: String,
    pub priority: Option<u32>,
    pub table: Option<String>,
    pub action: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub iif: Option<String>,
    pub oif: Option<String>,
    pub fwmark: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressPathRecord {
    pub family: String,
    pub target: String,
    pub gateway: Option<String>,
    pub interface: Option<String>,
    pub source: Option<String>,
    pub table: Option<String>,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteInspectResult {
    pub default_routes: Vec<RouteRecord>,
    pub policy_rules: Vec<PolicyRuleRecord>,
    pub egress: Option<EgressPathRecord>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NicOffloadFlag {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiLinkInfo {
    pub ssid: Option<String>,
    pub frequency_mhz: Option<u32>,
    pub signal_dbm: Option<f32>,
    pub tx_bitrate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NicDeepInfo {
    pub interface: String,
    pub status: String,
    pub mtu: u32,
    pub mac_address: String,
    pub driver: Option<String>,
    pub firmware: Option<String>,
    pub bus_info: Option<String>,
    pub speed: Option<String>,
    pub duplex: Option<String>,
    pub rx_errors: Option<u64>,
    pub tx_errors: Option<u64>,
    pub rx_dropped: Option<u64>,
    pub tx_dropped: Option<u64>,
    pub offloads: Vec<NicOffloadFlag>,
    pub wifi: Option<WifiLinkInfo>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NicDeepInfoResult {
    pub interfaces: Vec<NicDeepInfo>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionLabEntry {
    pub protocol: String,
    pub state: String,
    pub local_address: String,
    pub local_port: u16,
    pub remote_address: String,
    pub remote_port: u16,
    pub recv_q: u64,
    pub send_q: u64,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    pub bytes_sent: Option<u64>,
    pub bytes_received: Option<u64>,
    pub retransmits: Option<u64>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionLabResult {
    pub entries: Vec<ConnectionLabEntry>,
    pub permission_limited: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingSummary {
    pub target: String,
    pub profile: PingProfile,
    pub continuous: bool,
    pub transmitted: u32,
    pub received: u32,
    pub packet_loss_percent: f32,
    pub min_latency_ms: Option<f32>,
    pub avg_latency_ms: Option<f32>,
    pub max_latency_ms: Option<f32>,
    pub jitter_ms: Option<f32>,
    pub p50_latency_ms: Option<f32>,
    pub p95_latency_ms: Option<f32>,
    pub p99_latency_ms: Option<f32>,
    pub samples_collected: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceHop {
    pub hop: u8,
    pub host: Option<String>,
    pub address: Option<String>,
    pub rtt_ms: Vec<f32>,
    pub probes_sent: u8,
    pub probes_responded: u8,
    pub timed_out: bool,
    pub blocked_suspected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceAttempt {
    pub protocol: TraceProtocol,
    pub hops_collected: usize,
    pub timeout_hops: usize,
    pub reached_target: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSummary {
    pub target: String,
    pub requested_protocol: TraceProtocol,
    pub used_protocol: TraceProtocol,
    pub fallback_used: bool,
    pub reached_target: bool,
    pub timeout_ratio: f32,
    pub hops: Vec<TraceHop>,
    pub attempts: Vec<TraceAttempt>,
    pub blocked_indicators: Vec<String>,
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
    RouteInspect(RouteInspectResult),
    NicDeepInfo(NicDeepInfoResult),
    ConnectionLab(ConnectionLabResult),
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
            Self::RouteInspect(r) => {
                let egress = r
                    .egress
                    .as_ref()
                    .and_then(|entry| entry.interface.clone())
                    .unwrap_or_else(|| "n/a".to_string());
                format!(
                    "Routes: {} defaults, {} rules, egress {}",
                    r.default_routes.len(),
                    r.policy_rules.len(),
                    egress
                )
            }
            Self::NicDeepInfo(r) => format!("NIC: {} interface(s) inspected", r.interfaces.len()),
            Self::ConnectionLab(r) => {
                let with_pid = r.entries.iter().filter(|entry| entry.pid.is_some()).count();
                format!(
                    "Connections: {} entries (PID visibility: {}/{})",
                    r.entries.len(),
                    with_pid,
                    r.entries.len()
                )
            }
            Self::Ping(r) => format!(
                "Ping: loss {:.1}% avg {} ms p95 {} ms",
                r.packet_loss_percent,
                r.avg_latency_ms
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "n/a".to_string()),
                r.p95_latency_ms
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "n/a".to_string())
            ),
            Self::Trace(r) => format!(
                "Trace: {} hops via {:?}{}",
                r.hops.len(),
                r.used_protocol,
                if r.fallback_used { " (fallback)" } else { "" }
            ),
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

#[derive(Debug, Default)]
struct SsConnectionBlock {
    header: String,
    details: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct PingProfileDefaults {
    count: u32,
    timeout_secs: u32,
    interval_ms: u32,
    deadline_secs: u32,
}

#[derive(Debug, Default)]
struct PingParsed {
    transmitted: u32,
    received: u32,
    packet_loss_percent: Option<f32>,
    rtt_summary: Option<(f32, f32, f32, f32)>,
    samples: Vec<f32>,
}

#[derive(Debug, Clone)]
struct TraceAttemptExecution {
    protocol: TraceProtocol,
    hops: Vec<TraceHop>,
    timeout_hops: usize,
    reached_target: bool,
    blocked_indicators: Vec<String>,
    warnings: Vec<String>,
    warning: Option<String>,
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
            NetworkDiagnosticsRequest::RouteInspect(req) => {
                self.execute_route_inspect(job_id, req).await
            }
            NetworkDiagnosticsRequest::NicDeepInfo(req) => {
                self.execute_nic_deep_info(job_id, req).await
            }
            NetworkDiagnosticsRequest::ConnectionLab(req) => {
                self.execute_connection_lab(job_id, req).await
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
            let (split_dns_domains, split_dns_warning) = read_split_dns_domains();
            Ok((
                dns_servers,
                search_domains,
                gateways,
                detect_resolv_conf_path(),
                detect_networkmanager_dns_mode(),
                split_dns_domains,
                split_dns_warning,
            ))
        })
        .await
        .map_err(map_anyhow_error)?;

        let (
            dns_servers,
            mut search_domains,
            gateways,
            resolv_conf_path,
            network_manager_dns_mode,
            mut split_dns_domains,
            split_dns_warning,
        ) = snapshot;
        let resolver_mode = detect_resolver_mode();
        search_domains.sort();
        search_domains.dedup();
        split_dns_domains.sort();
        split_dns_domains.dedup();

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
        if let Some(warning) = split_dns_warning {
            warnings.push(warning);
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
                    port: gateway.port,
                    metric: gateway.metric,
                })
                .collect()
        } else {
            Vec::new()
        };

        let conflicts = detect_dns_conflicts(
            &resolver_mode,
            &resolv_conf_path,
            &dns_servers,
            &split_dns_domains,
            network_manager_dns_mode.as_deref(),
        );

        Ok(NetworkDiagnosticsResult::DnsExplain(DnsExplainResult {
            resolver_mode,
            resolv_conf_path,
            network_manager_dns_mode,
            dns_servers,
            search_domains,
            split_dns_domains,
            default_gateways,
            conflicts,
            warnings,
        }))
    }

    async fn execute_route_inspect(
        &self,
        job_id: u64,
        request: RouteInspectRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        self.emit_progress(job_id, "Collecting route tables and policy rules");
        let mut warnings = Vec::new();

        let ipv4_route_args = vec![
            "-j".to_string(),
            "-4".to_string(),
            "route".to_string(),
            "show".to_string(),
            "table".to_string(),
            "all".to_string(),
        ];
        let ipv4_routes_raw = run_command("ip", &ipv4_route_args, Duration::from_secs(8)).await?;
        let mut default_routes = parse_route_records_from_json("ipv4", &ipv4_routes_raw.stdout);

        let ipv6_route_args = vec![
            "-j".to_string(),
            "-6".to_string(),
            "route".to_string(),
            "show".to_string(),
            "table".to_string(),
            "all".to_string(),
        ];
        match run_command("ip", &ipv6_route_args, Duration::from_secs(8)).await {
            Ok(raw) => {
                default_routes.extend(parse_route_records_from_json("ipv6", &raw.stdout));
                if !raw.stderr.trim().is_empty() {
                    warnings.push(format!("IPv6 route query: {}", first_line(&raw.stderr)));
                }
            }
            Err(error) => match error.code {
                NetworkDiagnosticsErrorCode::ExecutionFailed
                | NetworkDiagnosticsErrorCode::ParseFailed
                | NetworkDiagnosticsErrorCode::PermissionDenied => {
                    warnings.push(format!("IPv6 route query unavailable: {}", error.message));
                }
                _ => return Err(error),
            },
        }

        default_routes.retain(|route| is_default_route_destination(&route.destination));
        default_routes.sort_by(|a, b| {
            a.family
                .cmp(&b.family)
                .then_with(|| {
                    a.metric
                        .unwrap_or(u32::MAX)
                        .cmp(&b.metric.unwrap_or(u32::MAX))
                })
                .then_with(|| a.interface.cmp(&b.interface))
        });
        if default_routes.is_empty() {
            warnings.push("No default routes were detected".to_string());
        }

        let policy_rules = if request.include_policy_rules {
            let mut rules = Vec::new();

            let rule_args4 = vec![
                "-j".to_string(),
                "-4".to_string(),
                "rule".to_string(),
                "show".to_string(),
            ];
            match run_command("ip", &rule_args4, Duration::from_secs(8)).await {
                Ok(raw) => rules.extend(parse_policy_rules_from_json("ipv4", &raw.stdout)),
                Err(error) => {
                    warnings.push(format!("IPv4 policy rules unavailable: {}", error.message))
                }
            }

            let rule_args6 = vec![
                "-j".to_string(),
                "-6".to_string(),
                "rule".to_string(),
                "show".to_string(),
            ];
            match run_command("ip", &rule_args6, Duration::from_secs(8)).await {
                Ok(raw) => rules.extend(parse_policy_rules_from_json("ipv6", &raw.stdout)),
                Err(error) => {
                    warnings.push(format!("IPv6 policy rules unavailable: {}", error.message))
                }
            }

            rules.sort_by(|a, b| {
                a.family.cmp(&b.family).then_with(|| {
                    a.priority
                        .unwrap_or(u32::MAX)
                        .cmp(&b.priority.unwrap_or(u32::MAX))
                })
            });
            rules
        } else {
            Vec::new()
        };

        let egress = if let Some(target_raw) = request
            .target
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            self.emit_progress(
                job_id,
                format!("Resolving active egress path for {target_raw}"),
            );
            let resolved_target = if target_raw.parse::<IpAddr>().is_ok() {
                target_raw.to_string()
            } else {
                let lookup_target = target_raw.to_string();
                let resolved =
                    run_blocking(move || LinuxSysMonitor::new().resolve_host(&lookup_target))
                        .await
                        .map_err(map_anyhow_error)?;
                resolved.addresses.into_iter().next().ok_or_else(|| {
                    NetworkDiagnosticsError::new(
                        NetworkDiagnosticsErrorCode::ParseFailed,
                        format!("failed to resolve route target: {target_raw}"),
                    )
                })?
            };
            let family = if resolved_target.contains(':') {
                "ipv6"
            } else {
                "ipv4"
            };
            let mut args = vec!["-j".to_string()];
            args.push(if family == "ipv6" {
                "-6".to_string()
            } else {
                "-4".to_string()
            });
            args.push("route".to_string());
            args.push("get".to_string());
            args.push(resolved_target.clone());

            match run_command("ip", &args, Duration::from_secs(8)).await {
                Ok(output) => parse_egress_path_from_route_get_json(
                    family,
                    target_raw,
                    &resolved_target,
                    &output.stdout,
                )
                .or_else(|| {
                    parse_egress_path_from_route_get_text(
                        family,
                        target_raw,
                        &resolved_target,
                        &output.stdout,
                    )
                })
                .or_else(|| {
                    warnings.push("Unable to parse `ip route get` output".to_string());
                    None
                }),
                Err(error) => {
                    warnings.push(format!("Egress path lookup failed: {}", error.message));
                    None
                }
            }
        } else {
            None
        };

        Ok(NetworkDiagnosticsResult::RouteInspect(RouteInspectResult {
            default_routes,
            policy_rules,
            egress,
            warnings,
        }))
    }

    async fn execute_nic_deep_info(
        &self,
        job_id: u64,
        request: NicDeepInfoRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        self.emit_progress(job_id, "Collecting interface inventory");
        let filter_iface = request
            .interface
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let mut interfaces =
            run_blocking(move || LinuxSysMonitor::new().get_network_interfaces_stats())
                .await
                .map_err(map_anyhow_error)?;

        if let Some(filter) = &filter_iface {
            interfaces.retain(|iface| iface.name == *filter);
            if interfaces.is_empty() {
                return Err(NetworkDiagnosticsError::new(
                    NetworkDiagnosticsErrorCode::InvalidInput,
                    format!("interface `{filter}` was not found"),
                ));
            }
        }

        interfaces.sort_by(|a, b| a.name.cmp(&b.name));

        let ethtool_available = command_exists("ethtool");
        let iw_available = command_exists("iw");
        let mut warnings = Vec::new();

        if !ethtool_available {
            warnings.push(
                "`ethtool` is not installed, driver/firmware/offload details are limited"
                    .to_string(),
            );
        }
        if request.include_wifi && !iw_available {
            warnings.push("`iw` is not installed, Wi-Fi link details are unavailable".to_string());
        }

        let mut detailed = Vec::with_capacity(interfaces.len());
        for iface in interfaces {
            self.emit_progress(job_id, format!("Inspecting interface {}", iface.name));
            let mut notes = Vec::new();
            let mut driver = None;
            let mut firmware = None;
            let mut bus_info = None;
            let mut speed = None;
            let mut duplex = None;
            let mut offloads = Vec::new();

            if ethtool_available {
                let info_args = vec!["-i".to_string(), iface.name.clone()];
                match run_command("ethtool", &info_args, Duration::from_secs(6)).await {
                    Ok(output) => {
                        let fields = parse_colon_key_value_map(&output.stdout);
                        driver = fields.get("driver").cloned();
                        firmware = fields.get("firmware-version").cloned();
                        bus_info = fields.get("bus-info").cloned();
                        if output.status_code.unwrap_or(0) != 0 {
                            notes
                                .push(format!("ethtool -i: {}", first_line(&combine_out(&output))));
                        }
                    }
                    Err(error) => {
                        notes.push(format!("ethtool -i unavailable: {}", error.message));
                    }
                }

                let link_args = vec![iface.name.clone()];
                match run_command("ethtool", &link_args, Duration::from_secs(6)).await {
                    Ok(output) => {
                        let fields = parse_colon_key_value_map(&output.stdout);
                        speed = fields
                            .get("speed")
                            .cloned()
                            .filter(|value| !value.eq_ignore_ascii_case("unknown!"));
                        duplex = fields
                            .get("duplex")
                            .cloned()
                            .filter(|value| !value.eq_ignore_ascii_case("unknown"));
                    }
                    Err(error) => {
                        notes.push(format!("ethtool link info unavailable: {}", error.message));
                    }
                }

                if request.include_stats {
                    let offload_args = vec!["-k".to_string(), iface.name.clone()];
                    match run_command("ethtool", &offload_args, Duration::from_secs(8)).await {
                        Ok(output) => {
                            offloads = parse_ethtool_offloads(&output.stdout);
                        }
                        Err(error) => {
                            notes.push(format!("ethtool offloads unavailable: {}", error.message));
                        }
                    }
                }
            }

            if speed.is_none() {
                speed = (!iface.link_speed.eq_ignore_ascii_case("unknown"))
                    .then(|| iface.link_speed.clone());
            }
            if duplex.is_none() {
                duplex =
                    (!iface.duplex.eq_ignore_ascii_case("unknown")).then(|| iface.duplex.clone());
            }

            let wifi = if request.include_wifi && iw_available {
                let iw_args = vec!["dev".to_string(), iface.name.clone(), "link".to_string()];
                match run_command("iw", &iw_args, Duration::from_secs(6)).await {
                    Ok(output) => {
                        let (wifi_info, wifi_note) = parse_iw_link_info(&output.stdout);
                        if let Some(note) = wifi_note {
                            notes.push(note);
                        }
                        if !output.stderr.trim().is_empty() {
                            notes.push(format!("iw: {}", first_line(&output.stderr)));
                        }
                        wifi_info
                    }
                    Err(error) => {
                        notes.push(format!("Wi-Fi details unavailable: {}", error.message));
                        None
                    }
                }
            } else {
                None
            };

            let (rx_errors, tx_errors, rx_dropped, tx_dropped) = if request.include_stats {
                (
                    read_interface_stat(&iface.name, "rx_errors"),
                    read_interface_stat(&iface.name, "tx_errors"),
                    read_interface_stat(&iface.name, "rx_dropped"),
                    read_interface_stat(&iface.name, "tx_dropped"),
                )
            } else {
                (None, None, None, None)
            };

            detailed.push(NicDeepInfo {
                interface: iface.name.clone(),
                status: iface.status.clone(),
                mtu: iface.mtu,
                mac_address: iface.mac_address.clone(),
                driver,
                firmware,
                bus_info,
                speed,
                duplex,
                rx_errors,
                tx_errors,
                rx_dropped,
                tx_dropped,
                offloads,
                wifi,
                notes,
            });
        }

        Ok(NetworkDiagnosticsResult::NicDeepInfo(NicDeepInfoResult {
            interfaces: detailed,
            warnings,
        }))
    }

    async fn execute_connection_lab(
        &self,
        job_id: u64,
        request: ConnectionLabRequest,
    ) -> std::result::Result<NetworkDiagnosticsResult, NetworkDiagnosticsError> {
        self.emit_progress(job_id, "Collecting live connection table from ss");

        let protocol_filter = request
            .protocol_filter
            .as_ref()
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty());
        let state_filter = request
            .state_filter
            .as_ref()
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty());

        let limit = request.limit.clamp(10, 512);
        let mut args = vec![
            "-H".to_string(),
            "-n".to_string(),
            "-t".to_string(),
            "-u".to_string(),
            "-p".to_string(),
        ];
        if request.include_extended_metrics {
            args.extend(["-i", "-m", "-e"].into_iter().map(ToString::to_string));
        }

        let output = run_command("ss", &args, Duration::from_secs(12)).await?;
        let mut entries = parse_ss_connections(&output.stdout);
        let mut warnings = Vec::new();

        if !output.stderr.trim().is_empty() {
            warnings.push(first_line(&output.stderr));
        }
        if output.status_code.unwrap_or(1) != 0 {
            warnings.push("`ss` exited with non-zero status".to_string());
        }
        if entries.is_empty() {
            warnings.push("No connections were parsed from `ss` output".to_string());
        }

        if let Some(filter) = protocol_filter.as_ref() {
            entries.retain(|entry| entry.protocol.eq_ignore_ascii_case(filter));
        }
        if let Some(filter) = state_filter.as_ref() {
            entries.retain(|entry| entry.state.eq_ignore_ascii_case(filter));
        }

        entries.sort_by(|a, b| {
            let a_load = a
                .bytes_received
                .unwrap_or(a.recv_q)
                .saturating_add(a.bytes_sent.unwrap_or(a.send_q));
            let b_load = b
                .bytes_received
                .unwrap_or(b.recv_q)
                .saturating_add(b.bytes_sent.unwrap_or(b.send_q));
            b_load
                .cmp(&a_load)
                .then_with(|| a.protocol.cmp(&b.protocol))
                .then_with(|| a.local_port.cmp(&b.local_port))
        });
        entries.truncate(limit);

        let pid_visible_count = entries.iter().filter(|entry| entry.pid.is_some()).count();
        let permission_limited = pid_visible_count == 0
            || contains_case_insensitive(&output.stderr, "permission denied")
            || contains_case_insensitive(&output.stdout, "permission denied");
        if permission_limited {
            warnings.push(
                "Process/PID visibility appears limited by permissions (expected on non-root)."
                    .to_string(),
            );
        }

        Ok(NetworkDiagnosticsResult::ConnectionLab(
            ConnectionLabResult {
                entries,
                permission_limited,
                warnings,
            },
        ))
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
        let defaults = ping_profile_defaults(request.profile);
        let continuous = request.continuous;
        let count = if request.count == 0 {
            defaults.count
        } else {
            request.count
        }
        .clamp(1, 200);
        let timeout_secs = if request.timeout_secs == 0 {
            defaults.timeout_secs
        } else {
            request.timeout_secs
        }
        .clamp(1, 30);
        let interval_ms = if request.interval_ms == 0 {
            defaults.interval_ms
        } else {
            request.interval_ms
        }
        .clamp(200, 5000);
        let mut deadline_secs = if request.deadline_secs == 0 {
            defaults.deadline_secs
        } else {
            request.deadline_secs
        }
        .clamp(2, 900);
        if !continuous {
            deadline_secs = deadline_secs.max(count.saturating_mul(timeout_secs).saturating_add(2));
        }

        let mut args = vec![
            "-n".to_string(),
            "-W".to_string(),
            timeout_secs.to_string(),
            "-i".to_string(),
            format!("{:.3}", interval_ms as f32 / 1000.0),
        ];
        if continuous {
            args.extend(["-w".to_string(), deadline_secs.to_string()]);
        } else {
            args.extend(["-c".to_string(), count.to_string()]);
        }
        args.push(target.clone());

        self.emit_progress(
            job_id,
            format!(
                "Running ping {:?} profile to {target} ({}, timeout {}s, interval {}ms)",
                request.profile,
                if continuous {
                    format!("deadline {}s", deadline_secs)
                } else {
                    format!("{} probes", count)
                },
                timeout_secs,
                interval_ms
            ),
        );

        let cmd_timeout = Duration::from_secs(deadline_secs as u64 + timeout_secs as u64 + 4);
        let output = run_command("ping", &args, cmd_timeout).await?;
        let parsed = parse_ping_output(&output.stdout);
        let fallback_parsed = if parsed.transmitted == 0 && parsed.samples.is_empty() {
            parse_ping_output(&output.stderr)
        } else {
            PingParsed::default()
        };

        let transmitted = parsed.transmitted.max(fallback_parsed.transmitted);
        let received = parsed.received.max(fallback_parsed.received);
        let packet_loss_percent = parsed
            .packet_loss_percent
            .or(fallback_parsed.packet_loss_percent)
            .unwrap_or_else(|| {
                if transmitted == 0 {
                    100.0
                } else {
                    ((transmitted.saturating_sub(received) as f32) * 100.0) / transmitted as f32
                }
            });
        let mut samples = if parsed.samples.is_empty() {
            fallback_parsed.samples
        } else {
            parsed.samples
        };
        if samples.len() > 1024 {
            samples.truncate(1024);
        }

        let mut min_latency_ms = None;
        let mut avg_latency_ms = None;
        let mut max_latency_ms = None;
        let mut jitter_ms = None;

        if !samples.is_empty() {
            min_latency_ms = samples.iter().copied().reduce(f32::min);
            max_latency_ms = samples.iter().copied().reduce(f32::max);
            avg_latency_ms = Some(samples.iter().sum::<f32>() / samples.len() as f32);
            jitter_ms = compute_jitter(&samples);
        } else if let Some((min_v, avg_v, max_v, mdev_v)) =
            parsed.rtt_summary.or(fallback_parsed.rtt_summary)
        {
            min_latency_ms = Some(min_v);
            avg_latency_ms = Some(avg_v);
            max_latency_ms = Some(max_v);
            jitter_ms = Some(mdev_v);
        }

        let status_code = output.status_code.unwrap_or(1);
        if status_code >= 2 {
            let message = first_line(if !output.stderr.trim().is_empty() {
                &output.stderr
            } else {
                &output.stdout
            });
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::ExecutionFailed,
                format!("ping failed for {target}: {message}"),
            ));
        }
        if transmitted == 0 && samples.is_empty() {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::ParseFailed,
                "unable to parse ping output",
            )
            .with_hint("Verify `ping` output format and target reachability"));
        }

        let p50_latency_ms = percentile(&samples, 0.50);
        let p95_latency_ms = percentile(&samples, 0.95);
        let p99_latency_ms = percentile(&samples, 0.99);

        Ok(NetworkDiagnosticsResult::Ping(PingSummary {
            target,
            profile: request.profile,
            continuous,
            transmitted,
            received,
            packet_loss_percent,
            min_latency_ms,
            avg_latency_ms,
            max_latency_ms,
            jitter_ms,
            p50_latency_ms,
            p95_latency_ms,
            p99_latency_ms,
            samples_collected: samples.len(),
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
        let requested_protocol = request.protocol;
        let protocol_chain = trace_protocol_chain(requested_protocol, request.enable_fallback);
        let mut warnings = Vec::new();
        let mut blocked_indicators = Vec::new();
        let mut attempts = Vec::new();
        let mut selected: Option<TraceAttemptExecution> = None;

        for (idx, protocol) in protocol_chain.into_iter().enumerate() {
            let fallback_label = if idx == 0 { "" } else { " (fallback)" };
            self.emit_progress(
                job_id,
                format!(
                    "Tracing route to {target} via {:?}{fallback_label}",
                    protocol
                ),
            );

            let attempt = self
                .run_trace_attempt(&target, &request, protocol, max_hops, timeout_secs, probes)
                .await?;

            warnings.extend(attempt.warnings.clone());
            blocked_indicators.extend(attempt.blocked_indicators.clone());
            attempts.push(TraceAttempt {
                protocol,
                hops_collected: attempt.hops.len(),
                timeout_hops: attempt.timeout_hops,
                reached_target: attempt.reached_target,
                warning: attempt.warning.clone(),
            });

            let current_score = trace_attempt_score(&attempt);
            let better_than_selected = selected
                .as_ref()
                .map(|existing| current_score > trace_attempt_score(existing))
                .unwrap_or(true);
            if better_than_selected {
                selected = Some(attempt.clone());
            }

            if attempt.reached_target {
                selected = Some(attempt);
                break;
            }
            if !request.enable_fallback || !should_trace_fallback(&attempt) {
                break;
            }
        }

        let Some(selected_attempt) = selected else {
            return Err(NetworkDiagnosticsError::new(
                NetworkDiagnosticsErrorCode::ParseFailed,
                "unable to collect traceroute hops",
            ));
        };

        let timeout_ratio = if selected_attempt.hops.is_empty() {
            1.0
        } else {
            selected_attempt.timeout_hops as f32 / selected_attempt.hops.len() as f32
        };
        if !selected_attempt.reached_target {
            warnings.push("Trace did not reach target within configured hops/timeouts".to_string());
        }
        blocked_indicators.sort();
        blocked_indicators.dedup();
        warnings.sort();
        warnings.dedup();

        Ok(NetworkDiagnosticsResult::Trace(TraceSummary {
            target,
            requested_protocol,
            used_protocol: selected_attempt.protocol,
            fallback_used: selected_attempt.protocol != requested_protocol || attempts.len() > 1,
            reached_target: selected_attempt.reached_target,
            timeout_ratio,
            hops: selected_attempt.hops,
            attempts,
            blocked_indicators,
            warnings,
        }))
    }

    async fn run_trace_attempt(
        &self,
        target: &str,
        request: &TraceRequest,
        protocol: TraceProtocol,
        max_hops: u8,
        timeout_secs: u8,
        probes: u8,
    ) -> std::result::Result<TraceAttemptExecution, NetworkDiagnosticsError> {
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
        match protocol {
            TraceProtocol::Icmp => args.push("-I".to_string()),
            TraceProtocol::Udp => args.push("-U".to_string()),
            TraceProtocol::Tcp => {
                args.push("-T".to_string());
                let port = request.port.unwrap_or(443).clamp(1, u16::MAX);
                args.push("-p".to_string());
                args.push(port.to_string());
            }
        }
        args.push(target.to_string());

        let timeout = Duration::from_secs(
            (timeout_secs as u64)
                .saturating_mul(max_hops as u64)
                .saturating_add(8),
        );
        let output = run_command("traceroute", &args, timeout).await?;
        let hops = parse_traceroute_output(&output.stdout);
        let timeout_hops = hops.iter().filter(|hop| hop.timed_out).count();
        let header_target_ip = parse_traceroute_target_ip(&output.stdout);
        let reached_target = traceroute_reached_target(&hops, target, header_target_ip.as_deref());
        let mut warnings = Vec::new();
        if hops.is_empty() {
            warnings.push("No hops parsed from traceroute output".to_string());
        }
        if !output.stderr.trim().is_empty() {
            warnings.push(first_line(&output.stderr));
        }
        if output.status_code.unwrap_or(0) != 0 && warnings.is_empty() {
            warnings.push("traceroute exited with non-zero status".to_string());
        }

        let blocked_indicators = detect_traceroute_blocked_indicators(
            &output.stdout,
            &output.stderr,
            &hops,
            reached_target,
        );
        let warning = if warnings.is_empty() {
            None
        } else {
            Some(warnings.join(" | "))
        };

        Ok(TraceAttemptExecution {
            protocol,
            hops,
            timeout_hops,
            reached_target,
            blocked_indicators,
            warnings,
            warning,
        })
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

fn detect_resolv_conf_path() -> String {
    let resolv_path = Path::new("/etc/resolv.conf");
    if let Ok(real_path) = fs::canonicalize(resolv_path) {
        return normalize_path_text(&real_path);
    }
    if let Ok(link_target) = fs::read_link(resolv_path) {
        if link_target.is_absolute() {
            return normalize_path_text(&link_target);
        }
        let joined = resolv_path
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .join(link_target);
        return normalize_path_text(&joined);
    }
    normalize_path_text(resolv_path)
}

fn detect_networkmanager_dns_mode() -> Option<String> {
    let mut candidates = vec![std::path::PathBuf::from(
        "/etc/NetworkManager/NetworkManager.conf",
    )];
    if let Ok(entries) = fs::read_dir("/etc/NetworkManager/conf.d") {
        let mut conf_files = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|x| x.to_str()) == Some("conf"))
            .collect::<Vec<_>>();
        conf_files.sort();
        candidates.extend(conf_files);
    }

    let mut mode = None;
    for path in candidates {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let mut in_main = false;
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let section = line.trim_start_matches('[').trim_end_matches(']');
                in_main = section.eq_ignore_ascii_case("main");
                continue;
            }
            if !in_main {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                if key.trim().eq_ignore_ascii_case("dns") {
                    let cleaned = value
                        .split(['#', ';'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !cleaned.is_empty() {
                        mode = Some(cleaned);
                    }
                }
            }
        }
    }

    mode
}

fn read_split_dns_domains() -> (Vec<String>, Option<String>) {
    if !command_exists("resolvectl") {
        return (
            Vec::new(),
            Some("`resolvectl` not found, split-DNS domains are unavailable".to_string()),
        );
    }

    let output = run_command_with_timeout("resolvectl", ["domain"], Duration::from_secs(5));
    let Ok(output) = output else {
        return (
            Vec::new(),
            Some("failed to execute `resolvectl domain`".to_string()),
        );
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        let reason = if !stderr.trim().is_empty() {
            first_line(&stderr)
        } else {
            first_line(&stdout)
        };
        return (
            Vec::new(),
            Some(format!("`resolvectl domain` failed: {reason}")),
        );
    }

    let mut domains = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tail = if let Some(rest) = trimmed.strip_prefix("Global:") {
            rest.trim()
        } else if let Some((_, rest)) = trimmed.split_once(':') {
            rest.trim()
        } else {
            trimmed
        };
        for token in tail.split_whitespace() {
            let token = token.trim_matches(',').trim_matches(';');
            let Some(domain) = token.strip_prefix('~') else {
                continue;
            };
            if !domain.is_empty() && domain != "." {
                domains.push(domain.to_string());
            }
        }
    }
    domains.sort();
    domains.dedup();
    (domains, None)
}

fn detect_dns_conflicts(
    resolver_mode: &str,
    resolv_conf_path: &str,
    dns_servers: &[DnsServerRecord],
    split_dns_domains: &[String],
    network_manager_dns_mode: Option<&str>,
) -> Vec<String> {
    let mut conflicts = Vec::new();
    let mut resolv_conf_ips = dns_servers
        .iter()
        .filter(|server| server.source == "resolv.conf")
        .map(|server| server.address.clone())
        .collect::<Vec<_>>();
    resolv_conf_ips.sort();
    resolv_conf_ips.dedup();

    let mut resolved_ips = dns_servers
        .iter()
        .filter(|server| server.source.starts_with("resolvectl"))
        .map(|server| server.address.clone())
        .collect::<Vec<_>>();
    resolved_ips.sort();
    resolved_ips.dedup();

    if !resolv_conf_ips.is_empty()
        && !resolved_ips.is_empty()
        && !same_set(&resolv_conf_ips, &resolved_ips)
    {
        conflicts.push(
            "DNS servers from resolv.conf and resolvectl differ; active resolver backend may override manual changes."
                .to_string(),
        );
    }

    if resolver_mode.starts_with("plain")
        && dns_servers
            .iter()
            .any(|server| is_stub_resolver_address(&server.address))
    {
        conflicts.push(
            "Stub DNS address detected in plain resolv.conf mode; upstream DNS may be hidden."
                .to_string(),
        );
    }

    if !split_dns_domains.is_empty() && !resolver_mode.contains("systemd-resolved") {
        conflicts.push(
            "Split-DNS domains are configured but resolver mode is not systemd-resolved."
                .to_string(),
        );
    }

    if let Some(mode) = network_manager_dns_mode {
        if mode.eq_ignore_ascii_case("none")
            && resolv_conf_path.contains("NetworkManager")
            && dns_servers.is_empty()
        {
            conflicts.push(
                "NetworkManager dns=none is set but no DNS servers were discovered.".to_string(),
            );
        }
    }

    conflicts
}

fn parse_route_records_from_json(family: &str, json: &str) -> Vec<RouteRecord> {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(entries) = value.as_array() else {
        return Vec::new();
    };

    entries
        .iter()
        .map(|entry| RouteRecord {
            family: family.to_string(),
            table: json_value_to_string(entry.get("table")).unwrap_or_else(|| "main".to_string()),
            destination: json_value_to_string(entry.get("dst"))
                .unwrap_or_else(|| "default".to_string()),
            gateway: json_value_to_string(entry.get("gateway")),
            interface: json_value_to_string(entry.get("dev")),
            metric: json_value_to_u32(entry.get("metric")),
            protocol: json_value_to_string(entry.get("protocol")),
            scope: json_value_to_string(entry.get("scope")),
            source: json_value_to_string(entry.get("prefsrc").or_else(|| entry.get("src"))),
        })
        .collect()
}

fn parse_policy_rules_from_json(family: &str, json: &str) -> Vec<PolicyRuleRecord> {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(entries) = value.as_array() else {
        return Vec::new();
    };

    entries
        .iter()
        .map(|entry| PolicyRuleRecord {
            family: family.to_string(),
            priority: json_value_to_u32(entry.get("priority").or_else(|| entry.get("pref"))),
            table: json_value_to_string(entry.get("table").or_else(|| entry.get("lookup"))),
            action: json_value_to_string(entry.get("action").or_else(|| entry.get("type"))),
            from: json_value_to_string(entry.get("from").or_else(|| entry.get("src"))),
            to: json_value_to_string(entry.get("to").or_else(|| entry.get("dst"))),
            iif: json_value_to_string(entry.get("iif")),
            oif: json_value_to_string(entry.get("oif")),
            fwmark: json_value_to_string(entry.get("fwmark")),
        })
        .collect()
}

fn parse_egress_path_from_route_get_json(
    family: &str,
    requested_target: &str,
    resolved_target: &str,
    json: &str,
) -> Option<EgressPathRecord> {
    let value = serde_json::from_str::<Value>(json).ok()?;
    let record = value.as_array()?.first()?;
    let gateway = json_value_to_string(record.get("gateway"));
    let interface = json_value_to_string(record.get("dev"));
    let source = json_value_to_string(record.get("prefsrc").or_else(|| record.get("src")));
    let table = json_value_to_string(record.get("table"));

    Some(EgressPathRecord {
        family: family.to_string(),
        target: requested_target.to_string(),
        gateway: gateway.clone(),
        interface: interface.clone(),
        source: source.clone(),
        table,
        output: format!(
            "{} via {} dev {} src {}",
            resolved_target,
            gateway.unwrap_or_else(|| "direct".to_string()),
            interface.unwrap_or_else(|| "unknown".to_string()),
            source.unwrap_or_else(|| "n/a".to_string())
        ),
    })
}

fn parse_egress_path_from_route_get_text(
    family: &str,
    requested_target: &str,
    resolved_target: &str,
    text: &str,
) -> Option<EgressPathRecord> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|value| !value.is_empty())?
        .to_string();

    let mut gateway = None;
    let mut interface = None;
    let mut source = None;
    let mut table = None;

    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < tokens.len() {
        match tokens[idx] {
            "via" if idx + 1 < tokens.len() => {
                gateway = Some(tokens[idx + 1].to_string());
                idx += 1;
            }
            "dev" if idx + 1 < tokens.len() => {
                interface = Some(tokens[idx + 1].to_string());
                idx += 1;
            }
            "src" if idx + 1 < tokens.len() => {
                source = Some(tokens[idx + 1].to_string());
                idx += 1;
            }
            "table" if idx + 1 < tokens.len() => {
                table = Some(tokens[idx + 1].to_string());
                idx += 1;
            }
            _ => {}
        }
        idx += 1;
    }

    Some(EgressPathRecord {
        family: family.to_string(),
        target: requested_target.to_string(),
        gateway,
        interface,
        source,
        table,
        output: if requested_target == resolved_target {
            line
        } else {
            format!("{line} (resolved from {requested_target} -> {resolved_target})")
        },
    })
}

fn parse_colon_key_value_map(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            if !key.is_empty() && !value.is_empty() {
                map.insert(key, value.to_string());
            }
        }
    }
    map
}

fn parse_ethtool_offloads(text: &str) -> Vec<NicOffloadFlag> {
    let mut flags = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("Features for ") {
            continue;
        }
        let Some((name, raw_value)) = line.split_once(':') else {
            continue;
        };
        let value = raw_value.trim();
        let enabled = if value.starts_with("on") {
            Some(true)
        } else if value.starts_with("off") {
            Some(false)
        } else {
            None
        };
        if let Some(state) = enabled {
            flags.push(NicOffloadFlag {
                name: name.trim().to_string(),
                enabled: state,
            });
            if flags.len() >= 256 {
                break;
            }
        }
    }
    flags
}

fn parse_iw_link_info(output: &str) -> (Option<WifiLinkInfo>, Option<String>) {
    if output.trim().is_empty() {
        return (None, Some("`iw link` produced no output".to_string()));
    }
    if contains_case_insensitive(output, "not connected") {
        return (None, Some("Wi-Fi interface is not connected".to_string()));
    }

    let mut ssid = None;
    let mut frequency_mhz = None;
    let mut signal_dbm = None;
    let mut tx_bitrate = None;

    for raw in output.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("SSID:") {
            let value = rest.trim();
            if !value.is_empty() {
                ssid = Some(value.to_string());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("freq:") {
            frequency_mhz = rest.trim().parse::<u32>().ok();
            continue;
        }
        if let Some(rest) = line.strip_prefix("signal:") {
            signal_dbm = rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f32>().ok());
            continue;
        }
        if let Some(rest) = line.strip_prefix("tx bitrate:") {
            let value = rest.trim();
            if !value.is_empty() {
                tx_bitrate = Some(value.to_string());
            }
            continue;
        }
    }

    if ssid.is_none() && frequency_mhz.is_none() && signal_dbm.is_none() && tx_bitrate.is_none() {
        return (
            None,
            Some("Unable to parse Wi-Fi details from `iw link`".to_string()),
        );
    }

    (
        Some(WifiLinkInfo {
            ssid,
            frequency_mhz,
            signal_dbm,
            tx_bitrate,
        }),
        None,
    )
}

fn read_interface_stat(interface: &str, stat_name: &str) -> Option<u64> {
    fs::read_to_string(format!("/sys/class/net/{interface}/statistics/{stat_name}"))
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn is_default_route_destination(destination: &str) -> bool {
    destination == "default" || destination == "0.0.0.0/0" || destination == "::/0"
}

fn json_value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn json_value_to_u32(value: Option<&Value>) -> Option<u32> {
    match value? {
        Value::Number(n) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
        Value::String(s) => s.parse::<u32>().ok(),
        _ => None,
    }
}

fn same_set(left: &[String], right: &[String]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right.iter()).all(|(a, b)| a == b)
}

fn is_stub_resolver_address(address: &str) -> bool {
    matches!(address, "127.0.0.53" | "127.0.0.1" | "::1")
}

fn parse_ss_connections(output: &str) -> Vec<ConnectionLabEntry> {
    let blocks = parse_ss_connection_blocks(output);
    let mut entries = Vec::new();
    for block in blocks {
        if let Some(mut entry) = parse_ss_connection_header(&block.header) {
            let extended = parse_ss_extended_metrics(&block.details);
            entry.bytes_sent = extended.bytes_sent;
            entry.bytes_received = extended.bytes_received;
            entry.retransmits = extended.retransmits;
            if !extended.notes.is_empty() {
                entry.notes.extend(extended.notes);
            }
            entries.push(entry);
        }
    }
    entries
}

fn parse_ss_connection_blocks(output: &str) -> Vec<SsConnectionBlock> {
    let mut blocks = Vec::new();
    let mut current = SsConnectionBlock::default();

    for raw in output.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let is_detail = raw.starts_with(' ') || raw.starts_with('\t');
        if is_detail {
            if !current.header.is_empty() {
                current.details.push(raw.trim().to_string());
            }
            continue;
        }
        if !current.header.is_empty() {
            blocks.push(std::mem::take(&mut current));
        }
        current.header = raw.trim().to_string();
    }

    if !current.header.is_empty() {
        blocks.push(current);
    }

    blocks
}

fn parse_ss_connection_header(line: &str) -> Option<ConnectionLabEntry> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    if tokens.len() < 6 {
        return None;
    }

    let proto_raw = tokens.first()?.to_ascii_uppercase();
    if !matches!(proto_raw.as_str(), "TCP" | "UDP" | "TCP6" | "UDP6") {
        return None;
    }

    let state = tokens.get(1)?.to_string();
    let recv_q = tokens
        .get(2)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let send_q = tokens
        .get(3)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let (local_address, local_port) = parse_ss_endpoint(tokens.get(4)?);
    let (remote_address, remote_port) = parse_ss_endpoint(tokens.get(5)?);

    let owner_blob = if tokens.len() > 6 {
        tokens[6..].join(" ")
    } else {
        String::new()
    };
    let (pid, process_name) = parse_ss_owner_blob(&owner_blob);

    let mut notes = Vec::new();
    if owner_blob.contains("users:((") && pid.is_none() {
        notes.push("PID field is hidden for this socket".to_string());
    }

    Some(ConnectionLabEntry {
        protocol: proto_raw,
        state,
        local_address,
        local_port,
        remote_address,
        remote_port,
        recv_q,
        send_q,
        pid,
        process_name,
        bytes_sent: None,
        bytes_received: None,
        retransmits: None,
        notes,
    })
}

fn parse_ss_owner_blob(blob: &str) -> (Option<u32>, Option<String>) {
    let mut process_name = None;
    let mut pid = None;

    if let Some(after_prefix) = blob.split("users:((\"").nth(1) {
        process_name = after_prefix
            .split('"')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
    }

    if let Some(after_pid) = blob.split("pid=").nth(1) {
        let digits = after_pid
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if !digits.is_empty() {
            pid = digits.parse::<u32>().ok();
        }
    }

    (pid, process_name)
}

fn parse_ss_endpoint(value: &str) -> (String, u16) {
    let raw = value.trim();
    if raw == "*" || raw == "*:*" {
        return ("*".to_string(), 0);
    }

    if raw.starts_with('[') {
        if let Some(close_idx) = raw.rfind("]:") {
            let host = raw
                .trim_start_matches('[')
                .get(..close_idx.saturating_sub(1))
                .unwrap_or(raw);
            let port_str = raw.get(close_idx + 2..).unwrap_or_default();
            return (host.to_string(), parse_port_token(port_str).unwrap_or(0));
        }
    }

    if let Some((host, port)) = raw.rsplit_once(':') {
        return (host.to_string(), parse_port_token(port).unwrap_or(0));
    }

    (raw.to_string(), 0)
}

fn parse_port_token(token: &str) -> Option<u16> {
    let trimmed = token.trim().trim_matches(']');
    if trimmed == "*" {
        return Some(0);
    }
    trimmed.parse::<u16>().ok()
}

#[derive(Debug, Default)]
struct SsExtendedMetrics {
    bytes_sent: Option<u64>,
    bytes_received: Option<u64>,
    retransmits: Option<u64>,
    notes: Vec<String>,
}

fn parse_ss_extended_metrics(lines: &[String]) -> SsExtendedMetrics {
    let mut out = SsExtendedMetrics::default();
    if lines.is_empty() {
        return out;
    }

    let joined = lines.join(" ");
    out.bytes_sent = extract_prefixed_number(&joined, "bytes_sent:")
        .or_else(|| extract_prefixed_number(&joined, "bytes_acked:"));
    out.bytes_received = extract_prefixed_number(&joined, "bytes_received:");
    out.retransmits = extract_prefixed_retransmits(&joined);

    if out.retransmits.unwrap_or(0) > 0 {
        out.notes.push(format!(
            "retransmits detected: {}",
            out.retransmits.unwrap_or(0)
        ));
    }
    if contains_case_insensitive(&joined, "timer:(on") {
        out.notes
            .push("TCP retransmission timer is active".to_string());
    }
    if contains_case_insensitive(&joined, "zero-window")
        || contains_case_insensitive(&joined, "rwnd_limited")
    {
        out.notes
            .push("receiver/window limitation detected".to_string());
    }

    out
}

fn extract_prefixed_number(text: &str, prefix: &str) -> Option<u64> {
    let after = text.split(prefix).nth(1)?;
    let token = after
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if token.is_empty() {
        None
    } else {
        token.parse::<u64>().ok()
    }
}

fn extract_prefixed_retransmits(text: &str) -> Option<u64> {
    let after = text.split("retrans:").nth(1)?;
    let token = after
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '/')
        .collect::<String>();
    if token.is_empty() {
        return None;
    }
    token
        .split('/')
        .next()
        .and_then(|value| value.parse::<u64>().ok())
}

fn ping_profile_defaults(profile: PingProfile) -> PingProfileDefaults {
    match profile {
        PingProfile::Quick => PingProfileDefaults {
            count: 4,
            timeout_secs: 2,
            interval_ms: 250,
            deadline_secs: 8,
        },
        PingProfile::Latency => PingProfileDefaults {
            count: 20,
            timeout_secs: 2,
            interval_ms: 200,
            deadline_secs: 16,
        },
        PingProfile::Loss => PingProfileDefaults {
            count: 40,
            timeout_secs: 2,
            interval_ms: 250,
            deadline_secs: 32,
        },
    }
}

fn parse_ping_output(output: &str) -> PingParsed {
    let mut parsed = PingParsed::default();
    for raw_line in output.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(time_ms) = extract_ping_time_ms(line) {
            parsed.samples.push(time_ms);
        }

        if contains_case_insensitive(line, "packets transmitted")
            && contains_case_insensitive(line, "received")
        {
            if let Some(value) = extract_number_before(line, "packets transmitted") {
                parsed.transmitted = value;
            }
            if let Some(value) = extract_number_before(line, "received") {
                parsed.received = value;
            }
            if let Some(loss) = extract_loss_percent(line) {
                parsed.packet_loss_percent = Some(loss);
            }
            continue;
        }

        if (contains_case_insensitive(line, "min/avg/max/mdev")
            || contains_case_insensitive(line, "min/avg/max/stddev"))
            && line.contains('=')
        {
            parsed.rtt_summary = parse_rtt_summary_line(line);
        }
    }

    if parsed.received == 0 && !parsed.samples.is_empty() {
        parsed.received = parsed.samples.len() as u32;
    }
    if parsed.transmitted < parsed.received {
        parsed.transmitted = parsed.received;
    }
    if parsed.packet_loss_percent.is_none() && parsed.transmitted > 0 {
        parsed.packet_loss_percent = Some(
            ((parsed.transmitted.saturating_sub(parsed.received) as f32) * 100.0)
                / parsed.transmitted as f32,
        );
    }
    parsed
}

fn extract_ping_time_ms(line: &str) -> Option<f32> {
    let token = line
        .split_whitespace()
        .find(|part| part.starts_with("time=") || part.starts_with("time<"))?;
    let mut value = token
        .trim_start_matches("time=")
        .trim_start_matches("time<")
        .trim_end_matches("ms")
        .trim()
        .to_string();
    if value.ends_with("ms") {
        value = value.trim_end_matches("ms").trim().to_string();
    }
    value.parse::<f32>().ok()
}

fn extract_number_before(line: &str, marker: &str) -> Option<u32> {
    let before = line.split(marker).next()?;
    before
        .split_whitespace()
        .rev()
        .find_map(|token| token.parse::<u32>().ok())
}

fn extract_loss_percent(line: &str) -> Option<f32> {
    let before = line.split("packet loss").next()?;
    let token = before.split_whitespace().last()?;
    let normalized = token.trim_end_matches('%');
    normalized.parse::<f32>().ok()
}

fn parse_rtt_summary_line(line: &str) -> Option<(f32, f32, f32, f32)> {
    let values = line.split('=').nth(1)?.trim();
    let value_token = values
        .split_whitespace()
        .find(|segment| segment.contains('/'))
        .unwrap_or(values);
    let metrics = value_token
        .split('/')
        .take(4)
        .filter_map(|segment| segment.trim().parse::<f32>().ok())
        .collect::<Vec<_>>();
    if metrics.len() == 4 {
        Some((metrics[0], metrics[1], metrics[2], metrics[3]))
    } else {
        None
    }
}

fn compute_jitter(samples: &[f32]) -> Option<f32> {
    if samples.len() < 2 {
        return None;
    }
    let mut sum = 0.0f32;
    for idx in 1..samples.len() {
        sum += (samples[idx] - samples[idx - 1]).abs();
    }
    Some(sum / (samples.len() - 1) as f32)
}

fn percentile(samples: &[f32], percentile: f32) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    let p = percentile.clamp(0.0, 1.0);
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if sorted.len() == 1 {
        return sorted.first().copied();
    }
    let max_idx = (sorted.len() - 1) as f32;
    let pos = p * max_idx;
    let lower = pos.floor() as usize;
    let upper = pos.ceil() as usize;
    if lower == upper {
        sorted.get(lower).copied()
    } else {
        let lower_v = sorted[lower];
        let upper_v = sorted[upper];
        let frac = pos - lower as f32;
        Some(lower_v + (upper_v - lower_v) * frac)
    }
}

fn trace_protocol_chain(requested: TraceProtocol, enable_fallback: bool) -> Vec<TraceProtocol> {
    if !enable_fallback {
        return vec![requested];
    }
    let mut chain = vec![requested];
    for protocol in [TraceProtocol::Icmp, TraceProtocol::Udp, TraceProtocol::Tcp] {
        if !chain.contains(&protocol) {
            chain.push(protocol);
        }
    }
    chain
}

fn should_trace_fallback(attempt: &TraceAttemptExecution) -> bool {
    if attempt.reached_target {
        return false;
    }
    if attempt.hops.is_empty() {
        return true;
    }
    let timeout_ratio = attempt.timeout_hops as f32 / attempt.hops.len() as f32;
    if timeout_ratio >= 0.5 {
        return true;
    }
    !attempt.blocked_indicators.is_empty()
}

fn trace_attempt_score(attempt: &TraceAttemptExecution) -> i64 {
    let responded_hops = attempt
        .hops
        .iter()
        .filter(|hop| hop.probes_responded > 0)
        .count() as i64;
    (if attempt.reached_target { 1_000_000 } else { 0 }) + responded_hops * 1_000
        - attempt.timeout_hops as i64 * 100
        - attempt.blocked_indicators.len() as i64 * 10
}

fn parse_traceroute_target_ip(output: &str) -> Option<String> {
    let header = output.lines().find(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("traceroute to") || trimmed.starts_with("tracepath to")
    })?;
    extract_first_ip(header)
}

fn traceroute_reached_target(hops: &[TraceHop], target: &str, target_ip: Option<&str>) -> bool {
    let Some(last_response) = hops.iter().rev().find(|hop| hop.probes_responded > 0) else {
        return false;
    };
    if let Some(expected_ip) = target_ip {
        if last_response.address.as_deref() == Some(expected_ip) {
            return true;
        }
    }
    if let Some(address) = last_response.address.as_ref() {
        if address.eq_ignore_ascii_case(target) {
            return true;
        }
    }
    if let Some(host) = last_response.host.as_ref() {
        if host.eq_ignore_ascii_case(target) {
            return true;
        }
    }
    false
}

fn detect_traceroute_blocked_indicators(
    stdout: &str,
    stderr: &str,
    hops: &[TraceHop],
    reached_target: bool,
) -> Vec<String> {
    let mut indicators = Vec::new();
    if !reached_target && !hops.is_empty() {
        let timeout_ratio =
            hops.iter().filter(|hop| hop.timed_out).count() as f32 / hops.len() as f32;
        if timeout_ratio >= 0.7 {
            indicators.push("High timeout ratio suggests filtered or blocked probes".to_string());
        }
    }
    if hops.iter().any(|hop| hop.blocked_suspected) {
        indicators.push("Some hops reported timeout/unreachable markers".to_string());
    }

    let combined = format!("{stdout}\n{stderr}");
    for marker in [
        "operation not permitted",
        "permission denied",
        "network is unreachable",
        "administratively prohibited",
        "!x",
        "!h",
        "!n",
        "!p",
        "!f",
    ] {
        if contains_case_insensitive(&combined, marker) {
            indicators.push(format!("Traceroute output indicates `{marker}`"));
        }
    }
    indicators.sort();
    indicators.dedup();
    indicators
}

fn is_traceroute_blocked_marker(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "!x" | "!h" | "!n" | "!p" | "!f" | "!a" | "!s" | "!c"
    ) || lower.contains("unreachable")
        || lower.contains("prohibited")
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
            probes_sent: 1,
            probes_responded: 0,
            timed_out: true,
            blocked_suspected: true,
        });
    }
    if rest.iter().all(|token| *token == "*") {
        return Some(TraceHop {
            hop: hop_num,
            host: None,
            address: None,
            rtt_ms: Vec::new(),
            probes_sent: rest.len().min(u8::MAX as usize) as u8,
            probes_responded: 0,
            timed_out: true,
            blocked_suspected: true,
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
    let mut probes_sent: u8 = 0;
    let mut probes_responded: u8 = 0;
    let mut blocked_marker = false;
    while idx < rest.len() {
        let token = rest[idx];
        if token == "*" {
            probes_sent = probes_sent.saturating_add(1);
            idx += 1;
            continue;
        }
        if is_traceroute_blocked_marker(token) {
            blocked_marker = true;
            idx += 1;
            continue;
        }

        if let Ok(value) = token.parse::<f32>() {
            if rest.get(idx + 1).copied() == Some("ms") {
                rtt_ms.push(value);
                probes_sent = probes_sent.saturating_add(1);
                probes_responded = probes_responded.saturating_add(1);
                idx += 2;
                continue;
            }
            if token.ends_with("ms") {
                let stripped = token.trim_end_matches("ms");
                if let Ok(v) = stripped.parse::<f32>() {
                    rtt_ms.push(v);
                    probes_sent = probes_sent.saturating_add(1);
                    probes_responded = probes_responded.saturating_add(1);
                    idx += 1;
                    continue;
                }
            }
        } else if token.ends_with("ms") {
            let stripped = token.trim_end_matches("ms");
            if let Ok(v) = stripped.parse::<f32>() {
                rtt_ms.push(v);
                probes_sent = probes_sent.saturating_add(1);
                probes_responded = probes_responded.saturating_add(1);
                idx += 1;
                continue;
            }
        }
        idx += 1;
    }

    if probes_sent == 0 {
        probes_sent = rtt_ms.len().max(1).min(u8::MAX as usize) as u8;
    }
    if probes_responded == 0 && !rtt_ms.is_empty() {
        probes_responded = rtt_ms.len().min(u8::MAX as usize) as u8;
    }
    let timed_out = probes_responded == 0;

    Some(TraceHop {
        hop: hop_num,
        host,
        address,
        rtt_ms,
        probes_sent,
        probes_responded,
        timed_out,
        blocked_suspected: timed_out || blocked_marker,
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
        assert_eq!(hop.probes_sent, 3);
        assert_eq!(hop.probes_responded, 0);
        assert!(hop.blocked_suspected);
    }

    #[test]
    fn parse_traceroute_hop_ipv4() {
        let hop =
            parse_traceroute_hop_line("1  192.168.1.1  1.12 ms  1.03 ms  1.00 ms").expect("hop");
        assert_eq!(hop.hop, 1);
        assert_eq!(hop.address.as_deref(), Some("192.168.1.1"));
        assert_eq!(hop.rtt_ms.len(), 3);
        assert!(!hop.timed_out);
        assert_eq!(hop.probes_sent, 3);
        assert_eq!(hop.probes_responded, 3);
    }

    #[test]
    fn parse_traceroute_hop_marks_unreachable_probe() {
        let hop =
            parse_traceroute_hop_line("5  10.0.0.1  52.10 ms !N  52.21 ms !N  *").expect("hop");
        assert_eq!(hop.hop, 5);
        assert_eq!(hop.probes_sent, 3);
        assert_eq!(hop.probes_responded, 2);
        assert!(hop.blocked_suspected);
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
    fn parse_ping_output_extracts_summary_and_samples() {
        let fixture = r#"
PING 1.1.1.1 (1.1.1.1) 56(84) bytes of data.
64 bytes from 1.1.1.1: icmp_seq=1 ttl=56 time=12.8 ms
64 bytes from 1.1.1.1: icmp_seq=2 ttl=56 time=11.9 ms
64 bytes from 1.1.1.1: icmp_seq=3 ttl=56 time=14.2 ms
64 bytes from 1.1.1.1: icmp_seq=4 ttl=56 time=13.1 ms

--- 1.1.1.1 ping statistics ---
4 packets transmitted, 4 received, 0% packet loss, time 3004ms
rtt min/avg/max/mdev = 11.900/13.000/14.200/0.901 ms
"#;
        let parsed = parse_ping_output(fixture);
        assert_eq!(parsed.transmitted, 4);
        assert_eq!(parsed.received, 4);
        assert_eq!(parsed.packet_loss_percent, Some(0.0));
        assert_eq!(parsed.samples.len(), 4);
        assert_eq!(
            percentile(&parsed.samples, 0.95).map(|v| v.round()),
            Some(14.0)
        );
        assert!(compute_jitter(&parsed.samples).is_some());
    }

    #[test]
    fn parse_ping_output_handles_loss_only() {
        let fixture = r#"
--- 198.51.100.1 ping statistics ---
6 packets transmitted, 0 received, 100% packet loss, time 5107ms
"#;
        let parsed = parse_ping_output(fixture);
        assert_eq!(parsed.transmitted, 6);
        assert_eq!(parsed.received, 0);
        assert_eq!(parsed.packet_loss_percent, Some(100.0));
        assert!(parsed.samples.is_empty());
    }

    #[test]
    fn command_exists_handles_missing_binary() {
        assert!(!command_exists("__definitely_missing_binary__"));
    }

    #[test]
    fn parse_route_records_from_json_extracts_defaults() {
        let fixture = r#"
[
  {"dst":"default","gateway":"192.168.1.1","dev":"eth0","table":"main","metric":100},
  {"dst":"10.0.0.0/24","dev":"eth0","table":"main","metric":100}
]
"#;
        let routes = parse_route_records_from_json("ipv4", fixture);
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].destination, "default");
        assert_eq!(routes[0].gateway.as_deref(), Some("192.168.1.1"));
        assert_eq!(routes[0].interface.as_deref(), Some("eth0"));
    }

    #[test]
    fn parse_policy_rules_from_json_extracts_priority_and_table() {
        let fixture = r#"
[
  {"priority":1000,"table":"100","from":"10.1.0.0/16"},
  {"priority":32766,"table":"main"}
]
"#;
        let rules = parse_policy_rules_from_json("ipv4", fixture);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].priority, Some(1000));
        assert_eq!(rules[0].table.as_deref(), Some("100"));
        assert_eq!(rules[0].from.as_deref(), Some("10.1.0.0/16"));
    }

    #[test]
    fn parse_egress_path_from_route_get_json_extracts_core_fields() {
        let fixture = r#"
[
  {"dst":"1.1.1.1","gateway":"192.168.1.1","dev":"wlan0","prefsrc":"192.168.1.10","table":"main"}
]
"#;
        let egress =
            parse_egress_path_from_route_get_json("ipv4", "one.one.one.one", "1.1.1.1", fixture)
                .expect("egress");
        assert_eq!(egress.family, "ipv4");
        assert_eq!(egress.interface.as_deref(), Some("wlan0"));
        assert_eq!(egress.gateway.as_deref(), Some("192.168.1.1"));
        assert_eq!(egress.source.as_deref(), Some("192.168.1.10"));
    }

    #[test]
    fn parse_ss_connections_extracts_process_and_metrics() {
        let fixture = r#"
tcp ESTAB 0 0 192.168.1.20:22 192.168.1.40:51234 users:(("sshd",pid=1234,fd=4))
 cubic wscale:7,7 rto:201 rtt:1.42/0.12 ato:40 mss:1460 bytes_sent:2048 bytes_received:1024 retrans:1/20
udp UNCONN 0 0 127.0.0.53:53 0.0.0.0:* users:(("systemd-resolve",pid=777,fd=13))
"#;
        let parsed = parse_ss_connections(fixture);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].protocol, "TCP");
        assert_eq!(parsed[0].pid, Some(1234));
        assert_eq!(parsed[0].process_name.as_deref(), Some("sshd"));
        assert_eq!(parsed[0].bytes_sent, Some(2048));
        assert_eq!(parsed[0].bytes_received, Some(1024));
        assert_eq!(parsed[0].retransmits, Some(1));
        assert_eq!(parsed[1].protocol, "UDP");
        assert_eq!(parsed[1].local_port, 53);
    }

    #[test]
    fn parse_ethtool_offloads_reads_boolean_flags() {
        let fixture = r#"
Features for eth0:
rx-checksumming: on
tx-checksum-ipv4: off [fixed]
scatter-gather: on
"#;
        let flags = parse_ethtool_offloads(fixture);
        assert_eq!(flags.len(), 3);
        assert_eq!(flags[0].name, "rx-checksumming");
        assert!(flags[0].enabled);
        assert!(!flags[1].enabled);
    }
}
