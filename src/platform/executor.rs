use anyhow::Result;
use tokio::sync::mpsc::Receiver;
use async_trait::async_trait;

/// Generic interface for executing platform-specific commands.
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Execute a command and return its entire stdout output.
    async fn execute(&self, cmd: &str) -> Result<String>;

    /// Execute a command and return its output as an asynchronous stream of lines.
    async fn execute_stream(&self, cmd: &str) -> Result<Receiver<String>>;

    /// Get autocompletion suggestions for a given input.
    async fn suggest(&self, input: &str) -> Result<Vec<String>>;

    /// Validate the syntax of a command.
    async fn validate(&self, cmd: &str) -> Result<bool>;
}
