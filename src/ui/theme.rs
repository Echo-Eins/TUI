use ratatui::style::Color;
use crate::app::Config;

pub fn parse_color(hex: &str) -> Color {
    if hex.starts_with('#') && hex.len() == 7 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&hex[1..3], 16),
            u8::from_str_radix(&hex[3..5], 16),
            u8::from_str_radix(&hex[5..7], 16),
        ) {
            return Color::Rgb(r, g, b);
        }
    }
    Color::White
}

/// Theme helper that provides colors from the config
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub cpu_color: Color,
    pub gpu_color: Color,
    pub ram_color: Color,
    pub disk_color: Color,
    pub network_color: Color,
    pub warning_color: Color,
    pub error_color: Color,
    pub success_color: Color,
}

impl Theme {
    pub fn from_config(config: &Config) -> Self {
        let dark_theme = &config.theme.dark;

        Self {
            background: parse_color(&dark_theme.background),
            foreground: parse_color(&dark_theme.foreground),
            cpu_color: parse_color(&dark_theme.cpu_color),
            gpu_color: parse_color(&dark_theme.gpu_color),
            ram_color: parse_color(&dark_theme.ram_color),
            disk_color: parse_color(&dark_theme.disk_color),
            network_color: parse_color(&dark_theme.network_color),
            warning_color: parse_color(&dark_theme.warning_color),
            error_color: parse_color(&dark_theme.error_color),
            success_color: parse_color(&dark_theme.success_color),
        }
    }

    pub fn get_temp_color(&self, temp: f32) -> Color {
        if temp < 50.0 {
            self.success_color
        } else if temp < 70.0 {
            Color::Yellow
        } else if temp < 85.0 {
            self.warning_color
        } else {
            self.error_color
        }
    }

    pub fn get_usage_color(&self, usage: f32) -> Color {
        if usage < 50.0 {
            self.success_color
        } else if usage < 75.0 {
            Color::Yellow
        } else if usage < 90.0 {
            self.warning_color
        } else {
            self.error_color
        }
    }
}
