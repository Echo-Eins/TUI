/// Sudo detection, blacklist, and availability checking.

// ── Permission Failure Detection ───────────────────────────────────────────

/// Patterns in stderr that indicate a permission/authorization failure.
const PERMISSION_PATTERNS: &[&str] = &[
    "permission denied",
    "operation not permitted",
    "access denied",
    "eacces",
    "authentication required",
    "must be root",
    "superuser privileges",
    "you must be root",
    "root privileges required",
    "need to be root",
    "requires root",
    "insufficient privileges",
    "not permitted",
    "authorization required",
    "failed to authenticate",
    "polkit",
];

/// Exit codes that indicate a permission-related failure.
const PERMISSION_EXIT_CODES: &[i32] = &[1, 126];

/// Check if a command failure looks like a permission error.
pub fn detect_permission_failure(exit_code: i32, stderr: &str) -> bool {
    if !PERMISSION_EXIT_CODES.contains(&exit_code) {
        return false;
    }

    let stderr_lower = stderr.to_lowercase();
    PERMISSION_PATTERNS.iter().any(|pattern| stderr_lower.contains(pattern))
}

// ── Command Blacklist ──────────────────────────────────────────────────────

/// Patterns of dangerous commands that should NEVER be retried with sudo.
const BLACKLIST_PATTERNS: &[&str] = &[
    // Destructive filesystem operations
    "rm -rf /",
    "rm -rf /*",
    "rm -r /",
    "rm -r /*",
    // Disk formatting
    "mkfs",
    "mkfs.",
    // Raw disk writes
    "dd if=",
    "dd of=/dev/",
    // Dangerous permissions
    "chmod -R 777 /",
    "chmod 777 /",
    "chown -R root",
    // Fork bomb
    ":(){ :|:& };:",
    // Overwrite disk/system files
    "> /dev/sd",
    "> /dev/nvme",
    // Destructive package operations without confirmation
    "pacman -Rns",
    "apt remove --purge",
    // System shutdown/reboot (user should be explicit)
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
    "poweroff",
    "halt",
];

/// Check if a command is blacklisted and should never be retried with sudo.
pub fn is_blacklisted(cmd: &str) -> bool {
    let cmd_trimmed = cmd.trim();
    BLACKLIST_PATTERNS.iter().any(|pattern| cmd_trimmed.contains(pattern))
}

// ── Sudo Availability ──────────────────────────────────────────────────────

/// Check if sudo credentials are cached (no password needed).
/// Returns true if `sudo -n true` succeeds (exit code 0).
/// On Windows, always returns false.
pub fn is_sudo_cached() -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("sudo")
            .args(["-n", "true"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        false
    }
}

/// Build the sudo-prefixed version of a command.
pub fn sudo_command(cmd: &str) -> String {
    let trimmed = cmd.trim();
    if trimmed.starts_with("sudo ") {
        // Already has sudo
        trimmed.to_string()
    } else {
        format!("sudo {}", trimmed)
    }
}

// ── Collect stderr from block ──────────────────────────────────────────────

/// Extract the last N lines of stderr from a block's output lines.
pub fn collect_stderr_tail(
    output_lines: &[crate::app::console_state::OutputLine],
    max_lines: usize,
) -> String {
    output_lines
        .iter()
        .filter(|l| l.stream == crate::app::console_state::OutputStream::Stderr)
        .rev()
        .take(max_lines)
        .map(|l| l.text.clone())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_denied() {
        assert!(detect_permission_failure(1, "Failed to restart nginx.service: Access denied"));
        assert!(detect_permission_failure(126, "bash: /usr/sbin/nginx: Permission denied"));
        assert!(detect_permission_failure(1, "Error: Operation not permitted"));
        assert!(detect_permission_failure(1, "You must be root to perform this action"));
    }

    #[test]
    fn test_not_permission_failure() {
        // Wrong exit code
        assert!(!detect_permission_failure(0, "Permission denied"));
        assert!(!detect_permission_failure(2, "Permission denied"));
        // No permission pattern
        assert!(!detect_permission_failure(1, "File not found"));
        assert!(!detect_permission_failure(1, "Syntax error"));
    }

    #[test]
    fn test_blacklist() {
        assert!(is_blacklisted("rm -rf /"));
        assert!(is_blacklisted("rm -rf /*"));
        assert!(is_blacklisted("mkfs.ext4 /dev/sda1"));
        assert!(is_blacklisted("dd if=/dev/zero of=/dev/sda"));
        assert!(is_blacklisted("chmod -R 777 /etc"));
        assert!(is_blacklisted("shutdown now"));
    }

    #[test]
    fn test_not_blacklisted() {
        assert!(!is_blacklisted("systemctl restart nginx"));
        assert!(!is_blacklisted("cat /etc/fstab"));
        assert!(!is_blacklisted("ls -la /root"));
        assert!(!is_blacklisted("apt install vim"));
    }

    #[test]
    fn test_sudo_command() {
        assert_eq!(sudo_command("systemctl restart nginx"), "sudo systemctl restart nginx");
        assert_eq!(sudo_command("sudo systemctl restart nginx"), "sudo systemctl restart nginx");
        assert_eq!(sudo_command("  cat /etc/shadow  "), "sudo cat /etc/shadow");
    }
}
