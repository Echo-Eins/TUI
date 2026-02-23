use std::collections::VecDeque;
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleMode {
    Normal,
    Insert,
}

#[derive(Debug, Clone)]
pub struct ConsoleMessage {
    pub text: String,
    pub color: Color,
}

impl ConsoleMessage {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: Color::White,
        }
    }

    pub fn with_color(text: impl Into<String>, color: Color) -> Self {
        Self {
            text: text.into(),
            color,
        }
    }
}

pub struct ConsoleState {
    pub mode: ConsoleMode,
    pub input_buffer: String,
    pub cursor_position: usize,
    pub output_history: VecDeque<ConsoleMessage>,
    pub scroll_offset: u16,
    pub max_history: usize,
    pub is_running: bool,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self::new(1000)
    }
}

impl ConsoleState {
    pub fn new(max_history: usize) -> Self {
        Self {
            mode: ConsoleMode::Normal,
            input_buffer: String::new(),
            cursor_position: 0,
            output_history: VecDeque::with_capacity(max_history),
            scroll_offset: 0,
            max_history,
            is_running: false,
        }
    }

    pub fn enter_insert_mode(&mut self) {
        self.mode = ConsoleMode::Insert;
        self.scroll_offset = 0; // Snap to bottom when typing
    }

    pub fn enter_normal_mode(&mut self) {
        self.mode = ConsoleMode::Normal;
    }

    pub fn toggle_mode(&mut self) {
        if self.mode == ConsoleMode::Normal {
            self.enter_insert_mode();
        } else {
            self.enter_normal_mode();
        }
    }

    pub fn append_output(&mut self, text: &str) {
        // Very basic newline splitting for incoming stream chunks
        for line in text.split('\n') {
            if !line.is_empty() || text.ends_with('\n') {
                self.output_history.push_back(ConsoleMessage::new(line));
            }
        }
        
        while self.output_history.len() > self.max_history {
            self.output_history.pop_front();
        }
        
        // Auto-scroll to bottom if not in history view
        if self.scroll_offset == 0 {
            // we are already at the bottom
        }
    }

    pub fn append_error(&mut self, text: &str) {
        for line in text.split('\n') {
            if !line.is_empty() || text.ends_with('\n') {
                self.output_history.push_back(ConsoleMessage::with_color(line, Color::Red));
            }
        }
        
        while self.output_history.len() > self.max_history {
            self.output_history.pop_front();
        }
    }

    pub fn scroll_up(&mut self, amount: u16) {
        let max_scroll = self.output_history.len().saturating_sub(1) as u16;
        self.scroll_offset = self.scroll_offset.saturating_add(amount).min(max_scroll);
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }
    
    // Simple cursor handling for now
    pub fn insert_char(&mut self, c: char) {
        if self.cursor_position >= self.input_buffer.len() {
            self.input_buffer.push(c);
            self.cursor_position = self.input_buffer.len();
        } else {
            self.input_buffer.insert(self.cursor_position, c);
            self.cursor_position += 1;
        }
    }
    
    pub fn backspace(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
            self.input_buffer.remove(self.cursor_position);
        }
    }
    
    pub fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }
    
    pub fn move_cursor_right(&mut self) {
        if self.cursor_position < self.input_buffer.len() {
            self.cursor_position += 1;
        }
    }
    
    pub fn clear_input(&mut self) {
        self.input_buffer.clear();
        self.cursor_position = 0;
    }
}
