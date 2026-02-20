//! Cardputer Remote Desktop Server - Main Entry Point
//!
//! Usage:
//!   cardputer-remote [OPTIONS]
//!
//! Options:
//!   -c, --config <FILE>  Path to config file (default: config.toml)
//!   -v, --verbose        Enable verbose logging
//!   -h, --help           Show help

use cardputer_remote::{
    config::Config,
    network::{DiscoveryService, Server, Session},
    VERSION,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn executable_name() -> String {
    std::env::args()
        .next()
        .and_then(|p| {
            std::path::Path::new(&p)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "cardputer-remote".to_string())
}

/// Command line arguments
struct Args {
    config_path: PathBuf,
    verbose: bool,
}

impl Args {
    fn parse() -> Self {
        let mut args = std::env::args().skip(1);
        let mut config_path = PathBuf::from("config.toml");
        let mut verbose = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-c" | "--config" => {
                    if let Some(path) = args.next() {
                        config_path = PathBuf::from(path);
                    }
                }
                "-v" | "--verbose" => {
                    verbose = true;
                }
                "-h" | "--help" => {
                    Self::print_help();
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("Unknown argument: {}", arg);
                    Self::print_help();
                    std::process::exit(1);
                }
            }
        }

        Self {
            config_path,
            verbose,
        }
    }

    fn print_help() {
        println!("Cardputer Remote Desktop Server v{}", VERSION);
        println!();
        println!("Usage: {} [OPTIONS]", executable_name());
        println!();
        println!("Options:");
        println!("  -c, --config <FILE>  Path to config file (default: config.toml)");
        println!("  -v, --verbose        Enable verbose logging");
        println!("  -h, --help           Show this help");
    }
}

/// Initialize logging based on config
fn init_logging(config: &Config, verbose: bool) {
    let level = if verbose {
        Level::DEBUG
    } else {
        match config.logging.level.to_lowercase().as_str() {
            "trace" => Level::TRACE,
            "debug" => Level::DEBUG,
            "info" => Level::INFO,
            "warn" => Level::WARN,
            "error" => Level::ERROR,
            _ => Level::INFO,
        }
    };

    let filter = EnvFilter::new(format!("cardputer_remote={}", level));

    if config.logging.json_format {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_target(false).with_thread_ids(false))
            .init();
    }

    if let Some(ref log_file) = config.logging.log_file {
        info!("Logging to file: {}", log_file);
    }
}

struct App {
    config: Arc<Config>,
    discovery: DiscoveryService,
    active_sessions: Vec<tokio::task::JoinHandle<()>>,
}

impl App {
    async fn new(config: Config) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Arc::new(config);
        let discovery = DiscoveryService::new(&config)?;

        Ok(Self {
            config,
            discovery,
            active_sessions: Vec::new(),
        })
    }

    async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Cardputer Remote Desktop Server v{}", VERSION);
        info!("Protocol version: {}", cardputer_remote::PROTOCOL_VERSION);
        info!("Listening on port {}", self.config.server.port);

        self.discovery.start().await?;
        info!("mDNS discovery started");

        let (session_tx, mut session_rx) = mpsc::channel::<Session>(4);

        let server_handle = {
            let config = self.config.clone();
            tokio::spawn(async move {
                match Server::new(config).await {
                    Ok(server) => {
                        if let Err(e) = server.run(session_tx).await {
                            error!("Server error: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to create server: {}", e);
                    }
                }
            })
        };

        let shutdown = tokio::signal::ctrl_c();
        tokio::pin!(shutdown);

        info!("Server running. Press Ctrl+C to stop.");

        loop {
            tokio::select! {
                Some(mut session) = session_rx.recv() => {
                    let addr = session.addr();
                    info!("New session from {}", addr);

                    self.log_event("session_start", &format!("Client: {}", addr));

                    let handle = tokio::spawn(async move {
                        if let Err(e) = session.run().await {
                            warn!("Session {} error: {}", addr, e);
                        }
                        info!("Session {} ended", addr);
                    });

                    self.active_sessions.push(handle);
                    self.active_sessions.retain(|h| !h.is_finished());
                }
                _ = &mut shutdown => {
                    info!("Shutting down...");
                    break;
                }
            }
        }

        self.discovery.stop();
        server_handle.abort();

        for handle in self.active_sessions.drain(..) {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }

        info!("Server stopped");
        Ok(())
    }

    fn log_event(&self, event_type: &str, details: &str) {
        let timestamp = chrono::Utc::now().to_rfc3339();
        info!(
            event_type = event_type,
            details = details,
            timestamp = timestamp,
            "Event"
        );
    }
}

fn find_config_in_project(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                let name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();

                if matches!(name, ".git" | "target") {
                    continue;
                }

                stack.push(path);
                continue;
            }

            if path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("config.toml"))
            {
                return Some(path);
            }
        }
    }

    None
}

fn resolve_config_path(path: &Path) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }

    if path.is_absolute() {
        return path.to_path_buf();
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cwd_candidate = cwd.join(path);
    if cwd_candidate.exists() {
        return cwd_candidate;
    }

    if let Some(project_root) = find_config_in_project(&cwd) {
        return project_root;
    }

    cwd_candidate
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let config_path = resolve_config_path(&args.config_path);

    let config = match Config::load(&config_path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to load config from {:?}: {}", config_path, e);
            std::process::exit(1);
        }
    };

    init_logging(&config, args.verbose);

    info!("Configuration loaded from {:?}", config_path);

    let mut app = match App::new(config).await {
        Ok(app) => app,
        Err(e) => {
            error!("Failed to initialize application: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = app.run().await {
        error!("Application error: {}", e);
        std::process::exit(1);
    }
}
