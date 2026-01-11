// Radial menu for command history circular selection
#![allow(dead_code)]

pub struct RadialMenu {
    pub items: Vec<String>,
    pub selected_index: usize,
}

impl RadialMenu {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected_index: 0,
        }
    }

    pub fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
    }

    pub fn next(&mut self) {
        if !self.items.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.items.len();
        }
    }

    pub fn previous(&mut self) {
        if !self.items.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.items.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }
}
