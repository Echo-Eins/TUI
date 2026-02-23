pub mod executor;
#[allow(dead_code)]
pub mod linux;
pub mod windows;

pub use executor::CommandExecutor;
use std::sync::Arc;

pub fn get_executor() -> Arc<dyn CommandExecutor> {
    #[cfg(target_os = "windows")]
    {
        // Dummy fallback initialization for generic usage.
        // For specific instances like in monitors, they initialize their own.
        Arc::new(windows::executor::PowerShellExecutor::new(
            "powershell.exe".to_string(),
            30,
            0,
            false,
        ))
    }

    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::shell::LinuxCommandExecutor::new())
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    compile_error!("Unsupported target OS");
}
