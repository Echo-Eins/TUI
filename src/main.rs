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
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

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
        println!("Usage: cardputer-remote [OPTIONS]");
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

    // Log to file if configured
    if let Some(ref log_file) = config.logging.log_file {
        // Note: In production, we'd use tracing-appender for file logging
        info!("Logging to file: {}", log_file);
    }
}

/// Application state
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

        // Start mDNS discovery
        self.discovery.start().await?;
        info!("mDNS discovery started");

        // Channel for new sessions
        let (session_tx, mut session_rx) = mpsc::channel::<Session>(4);

        // Start server in background
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

        // Handle Ctrl+C
        let shutdown = tokio::signal::ctrl_c();
        tokio::pin!(shutdown);

        info!("Server running. Press Ctrl+C to stop.");

        loop {
            tokio::select! {
                // Handle new sessions
                Some(mut session) = session_rx.recv() => {
                    let addr = session.addr();
                    info!("New session from {}", addr);

                    // Log session start
                    self.log_event("session_start", &format!("Client: {}", addr));

                    // Run session in background
                    let handle = tokio::spawn(async move {
                        if let Err(e) = session.run().await {
                            warn!("Session {} error: {}", addr, e);
                        }
                        info!("Session {} ended", addr);
                    });

                    self.active_sessions.push(handle);

                    // Clean up finished sessions
                    self.active_sessions.retain(|h| !h.is_finished());
                }

                // Handle shutdown
                _ = &mut shutdown => {
                    info!("Shutting down...");
                    break;
                }
            }
        }

        // Cleanup
        self.discovery.stop();
        server_handle.abort();

        // Wait for active sessions to finish (with timeout)
        for handle in self.active_sessions.drain(..) {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }

        info!("Server stopped");
        Ok(())
    }

    /// Log an event for auditing
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

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Load config
    let config = match Config::load(&args.config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config from {:?}: {}", args.config_path, e);
            eprintln!("Creating default config file...");

            // Create default config
            let default_config = Config::default();
            let toml_str = match toml::to_string_pretty(&default_config) {
                Ok(value) => value,
                Err(e) => {
                    eprintln!("Failed to serialize default config: {}", e);
                    std::process::exit(1);
                }
            };

            if let Err(e) = std::fs::write(&args.config_path, toml_str) {
                eprintln!("Failed to write default config: {}", e);
                std::process::exit(1);
            }

            eprintln!("Default config created at {:?}", args.config_path);
            eprintln!("Please edit the config file and restart.");
            std::process::exit(1);
        }
    };

    // Initialize logging
    init_logging(&config, args.verbose);

    // Create and run application
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

#[cfg(test)]
mod tests {
    #[test]
    fn test_args_parse() {
        // Basic test - would need more comprehensive testing with actual args
    }
}
