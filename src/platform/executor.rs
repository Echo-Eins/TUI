use anyhow::Result;
use tokio::sync::mpsc::Receiver;
use async_trait::async_trait;

/// Message sent through the streaming channel from command execution.
#[derive(Debug, Clone)]
pub enum StreamMessage {
    Stdout(String),
    Stderr(String),
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
    async fn execute_stream(&self, cmd: &str) -> Result<Receiver<StreamMessage>>;

    /// Get autocompletion suggestions for a given input.
    async fn suggest(&self, input: &str) -> Result<Vec<String>>;

    /// Validate the syntax of a command.
    async fn validate(&self, cmd: &str) -> Result<bool>;
}
