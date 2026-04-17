use std::collections::BTreeSet;
use std::time::Duration;

use crossterm::event::KeyEvent;
use ratatui::{layout::Rect, Frame};

use crate::app::console_state::{split_shell_words, CommandOutput, ConsolePlotBlock, OutputLine};

mod math;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionKind {
    Builtin,
    Script,
    ExternalProcess,
    Wasm,
}

impl ExtensionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Script => "script",
            Self::ExternalProcess => "external-process",
            Self::Wasm => "wasm",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleCommandSpec {
    pub name: &'static str,
    pub summary: &'static str,
    pub usage: &'static str,
    pub tags: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionMetadata {
    pub id: &'static str,
    pub title: &'static str,
    pub version: &'static str,
    pub kind: ExtensionKind,
    pub description: &'static str,
    pub commands: Vec<ConsoleCommandSpec>,
    pub tags: &'static [&'static str],
    pub permissions: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleContext {
    pub cwd: String,
    pub shell_name: String,
    pub terminal_size: (u16, u16),
    pub env_vars: Vec<(String, String)>,
    pub config: ConsoleConfigContext,
    pub theme: ConsoleThemeContext,
    pub permissions: PermissionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleConfigContext {
    pub history_limit: usize,
    pub max_output_lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleThemeContext {
    pub name: String,
    pub compact_mode: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPolicy {
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub network: bool,
    pub shell: bool,
    pub env: bool,
}

impl PermissionPolicy {
    pub fn default_deny() -> Self {
        Self {
            filesystem_read: false,
            filesystem_write: false,
            network: false,
            shell: false,
            env: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Paused,
    Finished,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub title: String,
    pub status: SessionStatus,
    pub lines: Vec<String>,
}

pub trait ConsoleSession: Send {
    fn title(&self) -> &str;
    fn tick(&mut self, dt: Duration) -> SessionStatus;
    fn handle_key(&mut self, key: KeyEvent) -> SessionStatus;
    fn render(&self, frame: &mut Frame, area: Rect);
    fn summary(&self) -> SessionSummary;
}

pub enum ConsoleResult {
    Text(Vec<OutputLine>),
    Table(Vec<Vec<String>>),
    Formula(Vec<String>),
    Plot(ConsolePlotBlock),
    Canvas(Vec<String>),
    StartSession(Box<dyn ConsoleSession>),
    Error(String),
}

pub struct ConsoleCommandResponse {
    pub result: ConsoleResult,
    pub exit_code: i32,
}

impl ConsoleCommandResponse {
    pub fn ok(lines: Vec<OutputLine>) -> Self {
        Self {
            result: ConsoleResult::Text(lines),
            exit_code: 0,
        }
    }

    pub fn error(message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            result: ConsoleResult::Error(message.into()),
            exit_code,
        }
    }

    pub fn plot(plot: ConsolePlotBlock) -> Self {
        Self {
            result: ConsoleResult::Plot(plot),
            exit_code: 0,
        }
    }

    pub fn into_outputs(self) -> (Vec<CommandOutput>, i32) {
        let mut exit_code = self.exit_code;
        let outputs = match self.result {
            ConsoleResult::Text(lines) => lines.into_iter().map(CommandOutput::Line).collect(),
            ConsoleResult::Table(rows) => rows
                .into_iter()
                .map(|row| CommandOutput::Line(OutputLine::stdout(row.join("  "))))
                .collect(),
            ConsoleResult::Formula(lines) | ConsoleResult::Canvas(lines) => lines
                .into_iter()
                .map(|line| CommandOutput::Line(OutputLine::stdout(line)))
                .collect(),
            ConsoleResult::Plot(plot) => vec![CommandOutput::Plot(plot)],
            ConsoleResult::StartSession(_) => {
                exit_code = 1;
                vec![CommandOutput::Line(OutputLine::stderr(
                    "interactive console sessions are not wired yet",
                ))]
            }
            ConsoleResult::Error(message) => vec![CommandOutput::Line(OutputLine::stderr(message))],
        };
        (outputs, exit_code)
    }

    pub fn into_lines(self) -> (Vec<OutputLine>, i32) {
        let mut exit_code = self.exit_code;
        let lines = match self.result {
            ConsoleResult::Text(lines) => lines,
            ConsoleResult::Table(rows) => rows
                .into_iter()
                .map(|row| OutputLine::stdout(row.join("  ")))
                .collect(),
            ConsoleResult::Formula(lines) | ConsoleResult::Canvas(lines) => {
                lines.into_iter().map(OutputLine::stdout).collect()
            }
            ConsoleResult::Plot(plot) => plot
                .fallback_lines
                .into_iter()
                .map(OutputLine::stdout)
                .collect(),
            ConsoleResult::StartSession(_) => {
                exit_code = 1;
                vec![OutputLine::stderr(
                    "interactive console sessions are not wired yet",
                )]
            }
            ConsoleResult::Error(message) => vec![OutputLine::stderr(message)],
        };
        (lines, exit_code)
    }
}

pub trait ConsoleExtension: Send + Sync {
    fn metadata(&self) -> &ExtensionMetadata;

    fn execute(
        &self,
        command: &str,
        args: &[String],
        ctx: &ConsoleContext,
        registry: &ConsoleExtensionRegistry,
    ) -> ConsoleCommandResponse;
}

pub enum ConsoleRoute {
    Shell,
    Handled(ConsoleCommandResponse),
}

pub struct ConsoleCommandRouter {
    registry: ConsoleExtensionRegistry,
}

impl ConsoleCommandRouter {
    pub fn builtin() -> Self {
        let mut registry = ConsoleExtensionRegistry::default();
        registry.register(Box::new(CoreExtension::new()));
        registry.register(Box::new(math::MathExtension::new()));
        Self { registry }
    }

    pub fn route(&self, line: &str, ctx: &ConsoleContext) -> ConsoleRoute {
        let trimmed = line.trim();
        let Some(internal) = trimmed.strip_prefix(':') else {
            return ConsoleRoute::Shell;
        };

        let words = match split_shell_words(internal.trim()) {
            Ok(words) => words,
            Err(error) => {
                return ConsoleRoute::Handled(ConsoleCommandResponse::error(
                    format!("console command parse error: {error}"),
                    2,
                ));
            }
        };

        let Some(command) = words.first() else {
            return ConsoleRoute::Handled(ConsoleCommandResponse::error(
                "empty console extension command; try :help",
                2,
            ));
        };

        let command = command.to_ascii_lowercase();
        let args = words.into_iter().skip(1).collect::<Vec<_>>();
        ConsoleRoute::Handled(self.registry.execute(&command, &args, ctx))
    }

    pub fn is_prefixed_command(&self, command: &str) -> bool {
        command
            .strip_prefix(':')
            .is_some_and(|name| self.registry.has_command(name))
    }

    pub fn suggest_prefixed_command(&self, input: &str) -> Option<String> {
        let prefix = input.strip_prefix(':')?;
        if prefix.contains(char::is_whitespace) {
            return None;
        }

        self.registry
            .command_names()
            .find(|name| name.starts_with(prefix) && *name != prefix)
            .map(|name| format!(":{name}"))
    }
}

#[derive(Default)]
pub struct ConsoleExtensionRegistry {
    extensions: Vec<Box<dyn ConsoleExtension>>,
}

impl ConsoleExtensionRegistry {
    pub fn register(&mut self, extension: Box<dyn ConsoleExtension>) {
        self.extensions.push(extension);
    }

    pub fn execute(
        &self,
        command: &str,
        args: &[String],
        ctx: &ConsoleContext,
    ) -> ConsoleCommandResponse {
        if let Some(extension) = self.extensions.iter().find(|extension| {
            extension
                .metadata()
                .commands
                .iter()
                .any(|spec| spec.name == command)
        }) {
            return extension.execute(command, args, ctx, self);
        }

        ConsoleCommandResponse::error(
            format!("unknown console extension command :{command}; try :help"),
            127,
        )
    }

    pub fn extensions(&self) -> impl Iterator<Item = &ExtensionMetadata> {
        self.extensions.iter().map(|extension| extension.metadata())
    }

    pub fn command_specs(&self) -> impl Iterator<Item = &ConsoleCommandSpec> {
        self.extensions()
            .flat_map(|metadata| metadata.commands.iter())
    }

    pub fn command_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.command_specs().map(|spec| spec.name)
    }

    pub fn has_command(&self, command: &str) -> bool {
        self.command_names().any(|name| name == command)
    }
}

struct CoreExtension {
    metadata: ExtensionMetadata,
}

impl CoreExtension {
    fn new() -> Self {
        Self {
            metadata: ExtensionMetadata {
                id: "core",
                title: "Console Core",
                version: "0.1.0",
                kind: ExtensionKind::Builtin,
                description: "Console-native command routing and extension discovery.",
                commands: vec![
                    ConsoleCommandSpec {
                        name: "help",
                        summary: "Show Console extension commands.",
                        usage: ":help [command]",
                        tags: &["core", "help"],
                    },
                    ConsoleCommandSpec {
                        name: "mods",
                        summary: "List loaded Console extensions.",
                        usage: ":mods",
                        tags: &["core", "modules"],
                    },
                ],
                tags: &["core"],
                permissions: &[],
            },
        }
    }

    fn help_lines(
        &self,
        args: &[String],
        ctx: &ConsoleContext,
        registry: &ConsoleExtensionRegistry,
    ) -> ConsoleCommandResponse {
        if let Some(command) = args.first() {
            let normalized = command.trim_start_matches(':');
            if let Some(spec) = registry
                .command_specs()
                .find(|spec| spec.name == normalized)
            {
                return ConsoleCommandResponse::ok(vec![
                    OutputLine::system(format!(":{} - {}", spec.name, spec.summary)),
                    OutputLine::stdout(format!("usage: {}", spec.usage)),
                    OutputLine::stdout(format!("tags: {}", spec.tags.join(", "))),
                ]);
            }

            return ConsoleCommandResponse::error(
                format!("unknown console extension command :{normalized}"),
                127,
            );
        }

        let mut lines = vec![
            OutputLine::system(
                "Console extensions use ':' and never replace normal shell commands.",
            ),
            OutputLine::stdout(format!(
                "Context: shell={} cwd={} size={}x{} theme={} compact={}",
                ctx.shell_name,
                ctx.cwd,
                ctx.terminal_size.0,
                ctx.terminal_size.1,
                ctx.theme.name,
                ctx.theme.compact_mode
            )),
            OutputLine::stdout(""),
            OutputLine::stdout("Available commands:"),
        ];

        for spec in registry.command_specs() {
            lines.push(OutputLine::stdout(format!(
                "  :{:<8} {}",
                spec.name, spec.summary
            )));
        }

        lines.push(OutputLine::stdout(""));
        lines.push(OutputLine::stdout(
            "Next milestones: :base, :calc, :formula, :plot, :games, :toy.",
        ));

        ConsoleCommandResponse::ok(lines)
    }

    fn mods_lines(&self, registry: &ConsoleExtensionRegistry) -> ConsoleCommandResponse {
        let mut lines = vec![OutputLine::system("Loaded Console extensions:")];
        let mut seen_kinds = BTreeSet::new();

        for metadata in registry.extensions() {
            seen_kinds.insert(metadata.kind.label());
            let permissions = if metadata.permissions.is_empty() {
                "none".to_string()
            } else {
                metadata.permissions.join(", ")
            };
            let commands = metadata
                .commands
                .iter()
                .map(|command| format!(":{}", command.name))
                .collect::<Vec<_>>()
                .join(", ");

            lines.push(OutputLine::stdout(format!(
                "  {:<12} {:<8} v{}  commands: {}  permissions: {}",
                metadata.id,
                metadata.kind.label(),
                metadata.version,
                commands,
                permissions
            )));
            lines.push(OutputLine::stdout(format!("    {}", metadata.description)));
        }

        lines.push(OutputLine::stdout(""));
        lines.push(OutputLine::stdout(format!(
            "Kinds online: {}",
            seen_kinds.into_iter().collect::<Vec<_>>().join(", ")
        )));

        ConsoleCommandResponse::ok(lines)
    }
}

impl ConsoleExtension for CoreExtension {
    fn metadata(&self) -> &ExtensionMetadata {
        &self.metadata
    }

    fn execute(
        &self,
        command: &str,
        args: &[String],
        ctx: &ConsoleContext,
        registry: &ConsoleExtensionRegistry,
    ) -> ConsoleCommandResponse {
        match command {
            "help" => self.help_lines(args, ctx, registry),
            "mods" => self.mods_lines(registry),
            _ => ConsoleCommandResponse::error(
                format!("core extension does not handle :{command}"),
                127,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ConsoleContext {
        ConsoleContext {
            cwd: "/tmp".to_string(),
            shell_name: "bash".to_string(),
            terminal_size: (100, 30),
            env_vars: Vec::new(),
            config: ConsoleConfigContext {
                history_limit: 1000,
                max_output_lines: 500,
            },
            theme: ConsoleThemeContext {
                name: "dark".to_string(),
                compact_mode: false,
            },
            permissions: PermissionPolicy::default_deny(),
        }
    }

    #[test]
    fn router_passes_non_prefixed_input_to_shell() {
        let router = ConsoleCommandRouter::builtin();
        assert!(matches!(router.route("help", &ctx()), ConsoleRoute::Shell));
        assert!(matches!(
            router.route("echo :help", &ctx()),
            ConsoleRoute::Shell
        ));
    }

    #[test]
    fn router_handles_help_and_mods() {
        let router = ConsoleCommandRouter::builtin();

        let ConsoleRoute::Handled(help) = router.route(":help", &ctx()) else {
            panic!(":help should be handled");
        };
        let (help_lines, help_code) = help.into_lines();
        assert_eq!(help_code, 0);
        assert!(help_lines.iter().any(|line| line.text.contains(":mods")));

        let ConsoleRoute::Handled(mods) = router.route(":mods", &ctx()) else {
            panic!(":mods should be handled");
        };
        let (mods_lines, mods_code) = mods.into_lines();
        assert_eq!(mods_code, 0);
        assert!(mods_lines.iter().any(|line| line.text.contains("core")));
    }

    #[test]
    fn router_rejects_unknown_prefixed_command_without_shell_fallback() {
        let router = ConsoleCommandRouter::builtin();
        let ConsoleRoute::Handled(response) = router.route(":definitely-not-a-command", &ctx())
        else {
            panic!("prefixed commands must not fall through to shell");
        };
        let (lines, code) = response.into_lines();
        assert_eq!(code, 127);
        assert!(lines[0].text.contains("unknown console extension command"));
    }

    #[test]
    fn router_supports_command_suggestions() {
        let router = ConsoleCommandRouter::builtin();
        assert!(router.is_prefixed_command(":help"));
        assert_eq!(
            router.suggest_prefixed_command(":he"),
            Some(":help".to_string())
        );
        assert_eq!(router.suggest_prefixed_command("he"), None);
    }
}
