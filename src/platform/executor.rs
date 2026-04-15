use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::mpsc::Receiver;

/// Message sent through the streaming channel from command execution.
#[derive(Debug, Clone)]
pub enum StreamMessage {
    Stdout(String),
    Stderr(String),
    Interrupted,
    Exit(i32),
}

/// Generic interface for executing platform-specific commands.
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Human-readable name of the shell/executor (e.g. "bash", "powershell").
    fn name(&self) -> &str;

    /// Execute a command and return its entire stdout output.
    async fn execute(&self, cmd: &str) -> Result<String>;

    /// Execute a command and return its output as a stream of `StreamMessage`.
    ///
    /// - `cwd`: Optional working directory for the command.
    /// - `env`: Optional extra environment variables to inject.
    /// - `terminal_size`: Optional (cols, rows) for COLUMNS/LINES env vars.
    async fn execute_stream(
        &self,
        cmd: &str,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
        terminal_size: Option<(u16, u16)>,
    ) -> Result<Receiver<StreamMessage>>;

    /// Get autocompletion suggestions for a given input.
    async fn suggest(&self, input: &str) -> Result<Vec<String>>;

    /// Validate the syntax of a command.
    async fn validate(&self, cmd: &str) -> Result<bool>;

    /// Interrupt the currently running command, if any.
    async fn interrupt(&self) -> Result<()>;
}
