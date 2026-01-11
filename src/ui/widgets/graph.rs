#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders},
    Frame,
};

pub struct Graph {
    pub data: Vec<f64>,
    pub max_value: f64,
    pub color: Color,
}

impl Graph {
    pub fn new(color: Color) -> Self {
        Self {
            data: Vec::new(),
            max_value: 100.0,
            color,
        }
    }

    pub fn add_point(&mut self, value: f64) {
        self.data.push(value);
        // Keep only last N points based on graph width (320 pixels / 4 = 80 points)
        if self.data.len() > 80 {
            self.data.remove(0);
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect, title: &str) {
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.color));

        // TODO: Implement actual graph rendering
        // For now, just render the block
        f.render_widget(block, area);
    }
}
