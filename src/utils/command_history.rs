use std::collections::VecDeque;

/// Command history with circular menu support
pub struct CommandHistory {
    commands: VecDeque<String>,
    max_size: usize,
    selected_index: usize,
}

impl CommandHistory {
    pub fn new(max_size: usize) -> Self {
        Self {
            commands: VecDeque::new(),
            max_size,
            selected_index: 0,
        }
    }

    pub fn add(&mut self, command: String) {
        // Don't add empty commands or duplicates
        if command.is_empty() {
            return;
        }

        // Remove if already exists
        self.commands.retain(|cmd| cmd != &command);

        // Add to front
        self.commands.push_front(command);

        // Trim to max size
        while self.commands.len() > self.max_size {
            self.commands.pop_back();
        }

        self.selected_index = 0;
    }

    pub fn get_selected(&self) -> Option<&String> {
        self.commands.get(self.selected_index)
    }

    pub fn next(&mut self) {
        if !self.commands.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.commands.len();
        }
    }

    pub fn previous(&mut self) {
        if !self.commands.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.commands.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    pub fn get_all(&self) -> &VecDeque<String> {
        &self.commands
    }

    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    pub fn handle_mouse_click(&mut self, _x: u16, _y: u16) {
        // TODO: Implement radial menu mouse selection
    }
}
