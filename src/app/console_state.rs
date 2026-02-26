use ratatui::style::Color;
use std::collections::HashMap;
use std::time::Instant;

// ── Console Modes ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleMode {
    Normal,
    Insert,
    HistorySearch,
    Confirm,
    // Future: SuggestionSelect
}

// ── Output Stream Discrimination ───────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
    System, // Internal messages (e.g. "Command interrupted")
}

impl OutputStream {
    pub fn color(&self) -> Color {
        match self {
            OutputStream::Stdout => Color::White,
            OutputStream::Stderr => Color::Red,
            OutputStream::System => Color::DarkGray,
        }
    }
}

// ── Output Line ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OutputLine {
    pub text: String,
    pub stream: OutputStream,
}

impl OutputLine {
    pub fn stdout(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            stream: OutputStream::Stdout,
        }
    }

    pub fn stderr(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            stream: OutputStream::Stderr,
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            stream: OutputStream::System,
        }
    }
}

// ── Task State Machine ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TaskState {
    Completed {
        exit_code: i32,
        elapsed_ms: u64,
    },
    Failed {
        error: String,
        exit_code: Option<i32>,
        elapsed_ms: u64,
    },
    Interrupted {
        elapsed_ms: u64,
    },
    TimedOut {
        timeout_ms: u64,
    },
}

impl TaskState {
    /// Badge text for the status indicator.
    pub fn badge(&self) -> String {
        match self {
            TaskState::Completed {
                exit_code,
                elapsed_ms,
            } => {
                format!("[✓ {}] [{:.1}s]", exit_code, *elapsed_ms as f64 / 1000.0)
            }
            TaskState::Failed {
                exit_code,
                elapsed_ms,
                ..
            } => {
                let code = exit_code.map_or("?".to_string(), |c| c.to_string());
                format!("[✗ {}] [{:.1}s]", code, *elapsed_ms as f64 / 1000.0)
            }
            TaskState::Interrupted { elapsed_ms } => {
                format!("[⊘] [{:.1}s]", *elapsed_ms as f64 / 1000.0)
            }
            TaskState::TimedOut { timeout_ms } => {
                format!("[⏱ timeout {:.1}s]", *timeout_ms as f64 / 1000.0)
            }
        }
    }

    pub fn badge_color(&self) -> Color {
        match self {
            TaskState::Completed { exit_code, .. } if *exit_code == 0 => Color::Green,
            TaskState::Completed { .. } => Color::Yellow,
            TaskState::Failed { .. } => Color::Red,
            TaskState::Interrupted { .. } => Color::Yellow,
            TaskState::TimedOut { .. } => Color::Magenta,
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, TaskState::Completed { exit_code, .. } if *exit_code == 0)
    }
}

// ── Command Block ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CommandBlock {
    pub id: u64,
    pub input: String,
    pub cwd: String,
    pub output_lines: Vec<OutputLine>,
    pub state: Option<TaskState>, // None = still running
    pub started_at: Instant,
    pub finished_at: Option<Instant>,
    /// True if this block failed with a permission error and sudo retry is available.
    pub sudo_hint: bool,
    /// True if the block failed with stderr output, meaning it can be explained by AI.
    pub explain_hint: bool,
    /// True if currently waiting for Ollama to explain the error.
    pub is_explaining: bool,
    /// The AI explanation text, if available.
    pub explanation: Option<String>,
}

impl CommandBlock {
    pub fn new(id: u64, input: String, cwd: String) -> Self {
        Self {
            id,
            input,
            cwd,
            output_lines: Vec::new(),
            state: None,
            started_at: Instant::now(),
            finished_at: None,
            sudo_hint: false,
            explain_hint: false,
            is_explaining: false,
            explanation: None,
        }
    }

    /// Duration in milliseconds since the command started.
    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Check if still running.
    pub fn is_running(&self) -> bool {
        self.state.is_none()
    }

    /// Push an output line, keeping max_lines at most.
    pub fn push_line(&mut self, line: OutputLine, max_lines: usize) {
        self.output_lines.push(line);
        if self.output_lines.len() > max_lines {
            let excess = self.output_lines.len() - max_lines;
            self.output_lines.drain(0..excess);
        }
    }

    /// Complete the block.
    pub fn complete(&mut self, exit_code: i32) {
        let elapsed_ms = self.elapsed_ms();
        self.finished_at = Some(Instant::now());
        if exit_code == 0 {
            self.state = Some(TaskState::Completed {
                exit_code,
                elapsed_ms,
            });
        } else {
            // Collect last stderr lines for potential error analysis
            let stderr_tail: String = self
                .output_lines
                .iter()
                .filter(|l| l.stream == OutputStream::Stderr)
                .rev()
                .take(5)
                .map(|l| l.text.clone())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");

            self.state = Some(TaskState::Failed {
                error: stderr_tail,
                exit_code: Some(exit_code),
                elapsed_ms,
            });
        }
    }

    /// Mark as failed with an error message (no exit code available).
    pub fn fail(&mut self, error: String) {
        let elapsed_ms = self.elapsed_ms();
        self.finished_at = Some(Instant::now());
        self.state = Some(TaskState::Failed {
            error,
            exit_code: None,
            elapsed_ms,
        });
    }

    /// Mark as interrupted.
    pub fn interrupt(&mut self) {
        let elapsed_ms = self.elapsed_ms();
        self.finished_at = Some(Instant::now());
        self.state = Some(TaskState::Interrupted { elapsed_ms });
    }
}

// ── Console State ──────────────────────────────────────────────────────────

/// Status threshold: commands faster than this don't show a badge.
const DEFAULT_STATUS_THRESHOLD_MS: u64 = 400;
/// How long to persist the badge after completion.
const DEFAULT_STATUS_PERSIST_MS: u64 = 1800;

pub struct ConsoleState {
    // Mode
    pub mode: ConsoleMode,

    // Input
    pub input_buffer: String,
    pub cursor_position: usize,

    // Ghost text (Fish-style inline preview)
    pub ghost_text: Option<String>,

    // Syntax highlighting (pre-computed colored spans)
    pub highlighted_input: Vec<(String, ratatui::style::Color)>,

    // Command macros (!! and !$)
    pub last_command: Option<String>,
    pub last_args: Option<String>,

    // History search (Ctrl+R)
    pub history_search_query: String,
    pub history_search_results: Vec<String>,
    pub history_search_index: usize,

    // History navigation (Up/Down in Insert mode)
    pub history_nav_index: Option<usize>,
    pub history_nav_cache: Vec<String>,
    /// Saved input before starting history navigation
    pub history_nav_saved_input: String,

    // Confirm mode (sudo retry)
    pub confirm_command: Option<String>,
    pub confirm_action: Option<String>,

    // Blocks
    pub blocks: Vec<CommandBlock>,
    pub max_blocks: usize,
    next_block_id: u64,

    // Active task tracking
    pub active_block_id: Option<u64>,

    // Scrolling
    pub scroll_offset: u16,
    pub selected_block: Option<usize>,

    // Config
    pub status_threshold_ms: u64,
    pub status_persist_ms: u64,

    // Feature flags
    pub enable_ai_explain: bool,

    // Environment info (for status bar / session dashboard)
    pub shell_name: String,
    pub username: String,
    pub hostname: String,
    pub cwd: String,

    // Session environment (accumulated via `export VAR=val`)
    pub env_vars: HashMap<String, String>,
    /// Previous CWD for `cd -` support
    pub prev_cwd: Option<String>,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self::new(500)
    }
}

impl ConsoleState {
    pub fn new(max_blocks: usize) -> Self {
        let username = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string());

        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "localhost".to_string());

        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~".to_string());

        Self {
            mode: ConsoleMode::Normal,
            input_buffer: String::new(),
            cursor_position: 0,
            ghost_text: None,
            highlighted_input: Vec::new(),
            last_command: None,
            last_args: None,
            history_search_query: String::new(),
            history_search_results: Vec::new(),
            history_search_index: 0,
            history_nav_index: None,
            history_nav_cache: Vec::new(),
            history_nav_saved_input: String::new(),
            confirm_command: None,
            confirm_action: None,
            blocks: Vec::new(),
            max_blocks,
            next_block_id: 1,
            active_block_id: None,
            scroll_offset: 0,
            selected_block: None,
            status_threshold_ms: DEFAULT_STATUS_THRESHOLD_MS,
            status_persist_ms: DEFAULT_STATUS_PERSIST_MS,
            enable_ai_explain: false,
            shell_name: "bash".to_string(),
            username,
            hostname,
            cwd,
            env_vars: HashMap::new(),
            prev_cwd: None,
        }
    }

    // ── Mode transitions ───────────────────────────────────────────────

    pub fn enter_insert_mode(&mut self) {
        self.mode = ConsoleMode::Insert;
        self.scroll_offset = 0;
    }

    pub fn enter_normal_mode(&mut self) {
        self.mode = ConsoleMode::Normal;
    }

    // ── Block management ───────────────────────────────────────────────

    /// Start a new command block. Returns the block ID.
    pub fn start_command(&mut self, input: String) -> u64 {
        let id = self.next_block_id;
        self.next_block_id += 1;

        let block = CommandBlock::new(id, input, self.cwd.clone());
        self.blocks.push(block);
        self.active_block_id = Some(id);
        self.scroll_offset = 0;

        // Enforce max blocks limit
        while self.blocks.len() > self.max_blocks {
            self.blocks.remove(0);
        }

        id
    }

    /// Find a mutable reference to a block by ID.
    pub fn get_block_mut(&mut self, block_id: u64) -> Option<&mut CommandBlock> {
        self.blocks.iter_mut().find(|b| b.id == block_id)
    }

    /// Find a reference to a block by ID.
    pub fn get_block(&self, block_id: u64) -> Option<&CommandBlock> {
        self.blocks.iter().find(|b| b.id == block_id)
    }

    /// Check if a command is currently running.
    pub fn is_running(&self) -> bool {
        self.active_block_id
            .and_then(|id| self.get_block(id))
            .map_or(false, |b| b.is_running())
    }

    /// Complete the active block with an exit code.
    pub fn complete_active(&mut self, exit_code: i32) {
        if let Some(id) = self.active_block_id {
            if let Some(block) = self.get_block_mut(id) {
                block.complete(exit_code);
            }
            self.active_block_id = None;
        }
    }

    /// Fail the active block with an error.
    pub fn fail_active(&mut self, error: String) {
        if let Some(id) = self.active_block_id {
            if let Some(block) = self.get_block_mut(id) {
                block.fail(error);
            }
            self.active_block_id = None;
        }
    }

    /// Interrupt the active block.
    pub fn interrupt_active(&mut self) {
        if let Some(id) = self.active_block_id {
            if let Some(block) = self.get_block_mut(id) {
                block.interrupt();
            }
            self.active_block_id = None;
        }
    }

    /// Should the status badge be visible for a given task state?
    pub fn should_show_badge(&self, block: &CommandBlock) -> bool {
        match &block.state {
            None => {
                // Running: show if elapsed > threshold
                block.elapsed_ms() >= self.status_threshold_ms
            }
            Some(TaskState::Completed { elapsed_ms, .. })
            | Some(TaskState::Failed { elapsed_ms, .. })
            | Some(TaskState::Interrupted { elapsed_ms }) => {
                if *elapsed_ms < self.status_threshold_ms {
                    return false;
                }
                if let Some(finished_at) = block.finished_at {
                    finished_at.elapsed().as_millis() as u64 <= self.status_persist_ms
                } else {
                    true
                }
            }
            Some(TaskState::TimedOut { .. }) => {
                if let Some(finished_at) = block.finished_at {
                    finished_at.elapsed().as_millis() as u64 <= self.status_persist_ms
                } else {
                    true
                }
            }
        }
    }

    // ── Input helpers (char-based cursor) ─────────────────────────────
    //
    // `cursor_position` is a CHARACTER offset (not byte offset).
    // All methods convert between char index and byte index using
    // `char_indices()` to ensure correct behavior with multi-byte UTF-8.

    /// Convert a char-based cursor position to a byte index in the input buffer.
    fn cursor_byte_pos(&self) -> usize {
        self.input_buffer
            .char_indices()
            .nth(self.cursor_position)
            .map(|(i, _)| i)
            .unwrap_or(self.input_buffer.len())
    }

    /// Number of characters in the input buffer.
    fn input_char_count(&self) -> usize {
        self.input_buffer.chars().count()
    }

    pub fn insert_char(&mut self, c: char) {
        let byte_pos = self.cursor_byte_pos();
        if byte_pos >= self.input_buffer.len() {
            self.input_buffer.push(c);
        } else {
            self.input_buffer.insert(byte_pos, c);
        }
        self.cursor_position += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
            let byte_pos = self.cursor_byte_pos();
            // Find the byte range of the char at this position
            if let Some((_, ch)) = self.input_buffer.char_indices().nth(self.cursor_position) {
                self.input_buffer
                    .drain(byte_pos..byte_pos + ch.len_utf8());
            }
        }
    }

    pub fn delete_char(&mut self) {
        let char_count = self.input_char_count();
        if self.cursor_position < char_count {
            let byte_pos = self.cursor_byte_pos();
            if let Some((_, ch)) = self.input_buffer.char_indices().nth(self.cursor_position) {
                self.input_buffer
                    .drain(byte_pos..byte_pos + ch.len_utf8());
            }
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_position < self.input_char_count() {
            self.cursor_position += 1;
        }
    }

    pub fn move_cursor_home(&mut self) {
        self.cursor_position = 0;
    }

    pub fn move_cursor_end(&mut self) {
        self.cursor_position = self.input_char_count();
    }

    /// Kill (delete) from cursor to end of line. Returns the killed text.
    pub fn kill_to_end(&mut self) -> String {
        let byte_pos = self.cursor_byte_pos();
        let killed = self.input_buffer[byte_pos..].to_string();
        self.input_buffer.truncate(byte_pos);
        killed
    }

    /// Kill (delete) from start of line to cursor. Returns the killed text.
    pub fn kill_to_start(&mut self) -> String {
        let byte_pos = self.cursor_byte_pos();
        let killed = self.input_buffer[..byte_pos].to_string();
        self.input_buffer = self.input_buffer[byte_pos..].to_string();
        self.cursor_position = 0;
        killed
    }

    /// Kill (delete) the word before the cursor. Returns the killed text.
    pub fn kill_word_back(&mut self) -> String {
        if self.cursor_position == 0 {
            return String::new();
        }

        let chars: Vec<char> = self.input_buffer.chars().collect();
        let mut new_pos = self.cursor_position;

        // Skip trailing whitespace
        while new_pos > 0 && chars[new_pos - 1].is_whitespace() {
            new_pos -= 1;
        }
        // Skip word characters
        while new_pos > 0 && !chars[new_pos - 1].is_whitespace() {
            new_pos -= 1;
        }

        // Calculate byte positions
        let start_byte = self
            .input_buffer
            .char_indices()
            .nth(new_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.input_buffer.len());
        let end_byte = self.cursor_byte_pos();

        let killed = self.input_buffer[start_byte..end_byte].to_string();
        self.input_buffer.drain(start_byte..end_byte);
        self.cursor_position = new_pos;
        killed
    }

    pub fn clear_input(&mut self) {
        self.input_buffer.clear();
        self.cursor_position = 0;
    }

    // ── Scroll helpers ─────────────────────────────────────────────────

    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    // ── Ghost text ──────────────────────────────────────────────────────

    /// Update the ghost text suggestion.
    pub fn update_ghost_text(&mut self, suggestion: Option<String>) {
        self.ghost_text = suggestion;
    }

    /// Accept the entire ghost text, appending it to the input buffer.
    pub fn accept_ghost_text(&mut self) {
        if let Some(ghost) = self.ghost_text.take() {
            // Ghost text represents the full command; replace input with it
            self.input_buffer = ghost;
            self.cursor_position = self.input_char_count();
        }
    }

    /// Accept the next word from the ghost text.
    pub fn accept_ghost_word(&mut self) {
        if let Some(ghost) = &self.ghost_text {
            let input_chars = self.input_buffer.chars().count();
            let ghost_chars = ghost.chars().count();
            if ghost_chars > input_chars {
                // Get the remaining part of ghost text after current input (char-safe)
                let remaining: String = ghost.chars().skip(input_chars).collect();
                // Skip leading spaces, then take until next space
                let trimmed = remaining.trim_start();
                let leading_spaces = remaining.chars().count() - trimmed.chars().count();
                let word_end = trimmed
                    .char_indices()
                    .find(|(_, c)| c.is_whitespace())
                    .map(|(i, _)| trimmed[..i].chars().count())
                    .unwrap_or_else(|| trimmed.chars().count());
                let accept_char_count = leading_spaces + word_end;

                let to_append: String = remaining.chars().take(accept_char_count).collect();
                self.input_buffer.push_str(&to_append);
                self.cursor_position = self.input_char_count();

                // If we've consumed all ghost text, clear it
                if self.input_buffer.chars().count() >= ghost_chars {
                    self.ghost_text = None;
                }
            }
        }
    }

    /// Clear ghost text (called when input changes).
    pub fn clear_ghost_text(&mut self) {
        self.ghost_text = None;
    }

    // ── History search (Ctrl+R) ─────────────────────────────────────────

    /// Enter history search mode.
    pub fn enter_history_search(&mut self) {
        self.mode = ConsoleMode::HistorySearch;
        self.history_search_query.clear();
        self.history_search_results.clear();
        self.history_search_index = 0;
    }

    /// Exit history search mode, returning to Insert mode.
    pub fn exit_history_search(&mut self, accept: bool) {
        if accept {
            if let Some(cmd) = self.history_search_results.get(self.history_search_index) {
                self.input_buffer = cmd.clone();
                self.cursor_position = self.input_buffer.len();
            }
        }
        self.mode = ConsoleMode::Insert;
        self.history_search_query.clear();
        self.history_search_results.clear();
        self.history_search_index = 0;
    }

    /// Navigate history search results up.
    pub fn history_search_up(&mut self) {
        if self.history_search_index > 0 {
            self.history_search_index -= 1;
        }
    }

    /// Navigate history search results down.
    pub fn history_search_down(&mut self) {
        if !self.history_search_results.is_empty()
            && self.history_search_index < self.history_search_results.len() - 1
        {
            self.history_search_index += 1;
        }
    }

    // ── History navigation (Up/Down in Insert mode) ─────────────────────

    /// Start navigating history (called on first Up press in Insert mode).
    pub fn start_history_nav(&mut self, recent_commands: Vec<String>) {
        self.history_nav_saved_input = self.input_buffer.clone();
        self.history_nav_cache = recent_commands;
        self.history_nav_index = None;
    }

    /// Navigate to the previous history entry (Up).
    pub fn history_nav_up(&mut self) {
        if self.history_nav_cache.is_empty() {
            return;
        }
        match self.history_nav_index {
            None => {
                self.history_nav_index = Some(0);
            }
            Some(idx) if idx < self.history_nav_cache.len() - 1 => {
                self.history_nav_index = Some(idx + 1);
            }
            _ => return,
        }
        if let Some(idx) = self.history_nav_index {
            if let Some(cmd) = self.history_nav_cache.get(idx) {
                self.input_buffer = cmd.clone();
                self.cursor_position = self.input_buffer.len();
            }
        }
    }

    /// Navigate to the next history entry (Down).
    pub fn history_nav_down(&mut self) {
        match self.history_nav_index {
            Some(0) => {
                // Return to saved input
                self.history_nav_index = None;
                self.input_buffer = self.history_nav_saved_input.clone();
                self.cursor_position = self.input_buffer.len();
            }
            Some(idx) => {
                self.history_nav_index = Some(idx - 1);
                if let Some(cmd) = self.history_nav_cache.get(idx - 1) {
                    self.input_buffer = cmd.clone();
                    self.cursor_position = self.input_buffer.len();
                }
            }
            None => {}
        }
    }

    /// Reset history navigation (called when user types).
    pub fn reset_history_nav(&mut self) {
        self.history_nav_index = None;
        self.history_nav_cache.clear();
        self.history_nav_saved_input.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_respects_threshold_for_completed_and_failed_states() {
        let state = ConsoleState::new(16);

        let mut fast_ok = CommandBlock::new(1, "echo hi".to_string(), ".".to_string());
        fast_ok.state = Some(TaskState::Completed {
            exit_code: 0,
            elapsed_ms: state.status_threshold_ms.saturating_sub(1),
        });
        assert!(!state.should_show_badge(&fast_ok));

        let mut slow_fail = CommandBlock::new(2, "false".to_string(), ".".to_string());
        slow_fail.state = Some(TaskState::Failed {
            error: "failed".to_string(),
            exit_code: Some(1),
            elapsed_ms: state.status_threshold_ms + 1,
        });
        assert!(state.should_show_badge(&slow_fail));
    }
}
