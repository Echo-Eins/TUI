use std::collections::HashSet;
use std::path::Path;

// ── Suggestion Types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestionKind {
    Builtin,
    Command, // PATH binary
    Alias,
    Portage, // Gentoo package
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub text: String,
    pub kind: SuggestionKind,
    pub score: f64,
    pub description: Option<String>,
}

// ── Provider Trait ─────────────────────────────────────────────────────────

pub trait SuggestionProvider {
    fn name(&self) -> &str;
    fn suggest(&self, prefix: &str, cwd: &str) -> Vec<Suggestion>;
}

// ── Builtin Provider ───────────────────────────────────────────────────────

/// Common shell builtins available on Linux (bash/zsh).
const BASH_BUILTINS: &[&str] = &[
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "caller",
    "case",
    "cd",
    "command",
    "compgen",
    "complete",
    "continue",
    "declare",
    "dirs",
    "disown",
    "echo",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "for",
    "function",
    "getopts",
    "hash",
    "help",
    "history",
    "if",
    "jobs",
    "kill",
    "let",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "select",
    "set",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "then",
    "time",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "until",
    "wait",
    "while",
];

pub struct BuiltinProvider {
    builtins: Vec<String>,
}

impl BuiltinProvider {
    pub fn new() -> Self {
        Self {
            builtins: BASH_BUILTINS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl SuggestionProvider for BuiltinProvider {
    fn name(&self) -> &str {
        "builtins"
    }

    fn suggest(&self, prefix: &str, _cwd: &str) -> Vec<Suggestion> {
        if prefix.is_empty() {
            return Vec::new();
        }
        self.builtins
            .iter()
            .filter(|b| b.starts_with(prefix))
            .map(|b| Suggestion {
                text: b.clone(),
                kind: SuggestionKind::Builtin,
                score: 0.7,
                description: Some("shell builtin".to_string()),
            })
            .collect()
    }
}

// ── PATH Provider ──────────────────────────────────────────────────────────

/// Check if a directory entry is an executable file (platform-specific).
#[cfg(unix)]
fn is_executable(entry: &std::fs::DirEntry, _path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    entry
        .metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(_entry: &std::fs::DirEntry, path: &std::path::Path) -> bool {
    path.extension()
        .map(|ext| {
            let ext = ext.to_string_lossy().to_lowercase();
            ext == "exe" || ext == "cmd" || ext == "bat" || ext == "ps1"
        })
        .unwrap_or(false)
}

#[cfg(not(any(unix, windows)))]
fn is_executable(_entry: &std::fs::DirEntry, _path: &std::path::Path) -> bool {
    true // assume executable on unknown platforms
}

pub struct PathProvider {
    commands: Vec<String>,
    command_set: HashSet<String>,
}

impl PathProvider {
    /// Scan $PATH directories and collect all executable names.
    pub fn new() -> Self {
        let mut command_set = HashSet::new();
        let mut commands = Vec::new();

        if let Ok(path_var) = std::env::var("PATH") {
            let sep = if cfg!(windows) { ';' } else { ':' };
            for dir in path_var.split(sep) {
                let dir_path = Path::new(dir);
                if !dir_path.is_dir() {
                    continue;
                }
                if let Ok(entries) = std::fs::read_dir(dir_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let is_exec = is_executable(&entry, &path);

                        if is_exec && path.is_file() {
                            if let Some(name) = path.file_name() {
                                let name_str = name.to_string_lossy().to_string();
                                // On windows, strip the extension for display
                                #[cfg(windows)]
                                let name_str = {
                                    let p = std::path::Path::new(&name_str);
                                    p.file_stem()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or(name_str)
                                };
                                if command_set.insert(name_str.clone()) {
                                    commands.push(name_str);
                                }
                            }
                        }
                    }
                }
            }
        }

        commands.sort();
        Self {
            commands,
            command_set,
        }
    }

    /// Check if a command name exists in PATH.
    pub fn is_known_command(&self, cmd: &str) -> bool {
        self.command_set.contains(cmd)
    }
}

impl SuggestionProvider for PathProvider {
    fn name(&self) -> &str {
        "path"
    }

    fn suggest(&self, prefix: &str, _cwd: &str) -> Vec<Suggestion> {
        if prefix.is_empty() {
            return Vec::new();
        }
        self.commands
            .iter()
            .filter(|c| c.starts_with(prefix))
            .take(20) // limit PATH results
            .map(|c| Suggestion {
                text: c.clone(),
                kind: SuggestionKind::Command,
                score: 0.6,
                description: None,
            })
            .collect()
    }
}

// ── Alias Provider ─────────────────────────────────────────────────────────

pub struct AliasProvider {
    aliases: Vec<(String, String)>, // (name, expansion)
    alias_set: HashSet<String>,
}

impl AliasProvider {
    pub fn new() -> Self {
        Self {
            aliases: Vec::new(),
            alias_set: HashSet::new(),
        }
    }

    /// Parse aliases from `alias` command output (bash format: alias name='value')
    pub fn load_from_output(&mut self, output: &str) {
        self.aliases.clear();
        self.alias_set.clear();
        for line in output.lines() {
            let line = line.trim();
            // Format: alias name='value' or alias name="value"
            if let Some(rest) = line.strip_prefix("alias ") {
                if let Some(eq_pos) = rest.find('=') {
                    let name = rest[..eq_pos].trim().to_string();
                    let value = rest[eq_pos + 1..]
                        .trim()
                        .trim_matches('\'')
                        .trim_matches('"')
                        .to_string();
                    self.alias_set.insert(name.clone());
                    self.aliases.push((name, value));
                }
            }
        }
    }

    /// Check if a name is a known alias.
    pub fn is_alias(&self, name: &str) -> bool {
        self.alias_set.contains(name)
    }
}

impl SuggestionProvider for AliasProvider {
    fn name(&self) -> &str {
        "aliases"
    }

    fn suggest(&self, prefix: &str, _cwd: &str) -> Vec<Suggestion> {
        if prefix.is_empty() {
            return Vec::new();
        }
        self.aliases
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, expansion)| Suggestion {
                text: name.clone(),
                kind: SuggestionKind::Alias,
                score: 0.8,
                description: Some(format!("→ {}", expansion)),
            })
            .collect()
    }
}

// ── Portage Provider ───────────────────────────────────────────────────────

pub struct PortageProvider {
    packages: std::sync::Arc<std::sync::RwLock<Vec<String>>>,
    last_update: std::sync::Arc<std::sync::RwLock<std::time::Instant>>,
}

impl PortageProvider {
    pub fn new() -> Self {
        let provider = Self {
            packages: std::sync::Arc::new(std::sync::RwLock::new(Vec::new())),
            last_update: std::sync::Arc::new(std::sync::RwLock::new(std::time::Instant::now())),
        };
        provider.spawn_background_update();
        provider
    }

    fn spawn_background_update(&self) {
        let packages_lock = self.packages.clone();
        let time_lock = self.last_update.clone();

        std::thread::spawn(move || {
            // Check /var/db/pkg for installed packages (Gentoo standard)
            // On non-Gentoo or Windows, this will quickly fail and leave the list empty or mock it.
            let mut found = Vec::new();

            #[cfg(unix)]
            {
                if let Ok(categories) = std::fs::read_dir("/var/db/pkg") {
                    for cat_entry in categories.flatten() {
                        if !cat_entry.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                            continue;
                        }
                        let cat_name = cat_entry.file_name().to_string_lossy().to_string();
                        if cat_name.starts_with('-') || cat_name.contains('.') {
                            continue;
                        }

                        if let Ok(pkgs) = std::fs::read_dir(cat_entry.path()) {
                            for pkg_entry in pkgs.flatten() {
                                if !pkg_entry.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                                    continue;
                                }
                                let pkg_name = pkg_entry.file_name().to_string_lossy().to_string();
                                // strip version suffix if desired, or keep exact ebuild name.
                                // For suggestions, category/package-name is ideal. We'll add the full path.
                                found.push(format!("{}/{}", cat_name, pkg_name));
                            }
                        }
                    }
                }
            }

            // Fallback sample data for non-Unix targets where Portage paths don't exist.
            #[cfg(not(unix))]
            if found.is_empty() {
                // Mock data just for demonstration of the feature
                found = vec![
                    "sys-apps/systemd".to_string(),
                    "sys-fs/sysfsutils".to_string(),
                    "app-editors/neovim".to_string(),
                    "app-admin/sudo".to_string(),
                    "x11-base/xorg-server".to_string(),
                ];
            }

            found.sort();

            if let Ok(mut pkgs) = packages_lock.write() {
                *pkgs = found;
            }
            if let Ok(mut time) = time_lock.write() {
                *time = std::time::Instant::now();
            }
        });
    }
}

impl SuggestionProvider for PortageProvider {
    fn name(&self) -> &str {
        "portage"
    }

    fn suggest(&self, prefix: &str, _cwd: &str) -> Vec<Suggestion> {
        let pkgs = match self.packages.read() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };

        if pkgs.is_empty() {
            return Vec::new();
        }

        pkgs.iter()
            .filter(|p| p.starts_with(prefix) || p.contains(prefix))
            .take(20)
            .map(|p| Suggestion {
                text: p.clone(),
                kind: SuggestionKind::Portage,
                score: if p.starts_with(prefix) { 0.9 } else { 0.5 },
                description: Some("installed package".to_string()),
            })
            .collect()
    }
}

// ── Suggestion Engine ──────────────────────────────────────────────────────

pub struct SuggestionEngine {
    pub builtin_provider: BuiltinProvider,
    pub path_provider: PathProvider,
    pub alias_provider: AliasProvider,
    pub portage_provider: PortageProvider,
    builtin_set: HashSet<String>,
}

impl SuggestionEngine {
    pub fn new() -> Self {
        let builtin_provider = BuiltinProvider::new();
        let builtin_set: HashSet<String> = BASH_BUILTINS.iter().map(|s| s.to_string()).collect();
        let path_provider = PathProvider::new();
        let alias_provider = AliasProvider::new();
        let portage_provider = PortageProvider::new();

        Self {
            builtin_provider,
            path_provider,
            alias_provider,
            portage_provider,
            builtin_set,
        }
    }

    /// Get merged, deduplicated suggestions for the current input.
    /// Returns ranked suggestions sorted by score (highest first).
    pub fn suggest(&self, input: &str, cwd: &str) -> Vec<Suggestion> {
        let full_input = input.trim_start();
        if full_input.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        let mut seen = HashSet::new();

        // If the command is multi-word, check if the first word is a Gentoo package command
        if full_input.contains(' ') {
            let mut parts = full_input.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("");
            let rest = parts.next().unwrap_or("").trim_start();

            // Allow trailing spaces to trigger showing all packages
            let prefix_to_match = rest;

            if ["emerge", "equery", "eix", "pkg"].contains(&cmd) {
                // Return Portage suggestions, but format them so ghost text appends properly
                for mut suggestion in self.portage_provider.suggest(prefix_to_match, cwd) {
                    suggestion.text = format!("{} {}", cmd, suggestion.text);
                    if seen.insert(suggestion.text.clone()) {
                        results.push(suggestion);
                    }
                }
            }
            // For other multi-word commands, we don't have providers yet (e.g. file paths)
            results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            return results;
        }

        // Single word - complete commands
        let prefix = full_input;

        let mut results = Vec::new();
        let mut seen = HashSet::new();

        // Collect from all providers
        for suggestion in self.builtin_provider.suggest(prefix, cwd) {
            if seen.insert(suggestion.text.clone()) {
                results.push(suggestion);
            }
        }
        for suggestion in self.alias_provider.suggest(prefix, cwd) {
            if seen.insert(suggestion.text.clone()) {
                results.push(suggestion);
            }
        }
        for suggestion in self.path_provider.suggest(prefix, cwd) {
            if seen.insert(suggestion.text.clone()) {
                results.push(suggestion);
            }
        }

        // Sort by score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(20);
        results
    }

    /// Check if a command is known (builtin, PATH, or alias).
    pub fn is_known_command(&self, cmd: &str) -> bool {
        self.builtin_set.contains(cmd)
            || self.path_provider.is_known_command(cmd)
            || self.alias_provider.is_alias(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portage_provider_new_does_not_panic() {
        let result = std::panic::catch_unwind(PortageProvider::new);
        assert!(result.is_ok());
    }
}
