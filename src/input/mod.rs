//! Input simulation module - mouse and keyboard control
//!
//! Maps Cardputer input commands to Windows input events

use crate::protocol::{ClickAction, InputMode, KeyEvent, MouseButton, MouseClick, MouseMove};
use enigo::{
    Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings,
    Button as EnigoButton,
};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("Failed to initialize input controller: {0}")]
    InitError(String),

    #[error("Invalid keycode: {0}")]
    InvalidKeycode(u8),

    #[error("Input simulation failed")]
    SimulationFailed,
}

/// Input controller for mouse and keyboard
pub struct InputController {
    enigo: Enigo,
    current_mode: InputMode,
    /// Mouse movement speed (pixels per command)
    mouse_speed: i32,
    /// Keycode mapping from USB HID to enigo keys
    keymap: HashMap<u8, Key>,
}

impl InputController {
    /// Create a new input controller
    pub fn new() -> Result<Self, InputError> {
        let settings = Settings::default();
        let enigo = Enigo::new(&settings)
            .map_err(|e| InputError::InitError(e.to_string()))?;
        let keymap = Self::build_keymap();

        Ok(Self {
            enigo,
            current_mode: InputMode::Mouse,
            mouse_speed: 5,
            keymap,
        })
    }

    /// Build USB HID keycode to enigo Key mapping
    fn build_keymap() -> HashMap<u8, Key> {
        let mut map = HashMap::new();

        // Letters (0x04 - 0x1D = a-z)
        for i in 0..26u8 {
            let c = (b'a' + i) as char;
            map.insert(0x04 + i, Key::Unicode(c));
        }

        // Numbers (0x1E - 0x27 = 1-9, 0)
        for i in 0..9u8 {
            let c = (b'1' + i) as char;
            map.insert(0x1E + i, Key::Unicode(c));
        }
        map.insert(0x27, Key::Unicode('0'));

        // Special keys
        map.insert(0x28, Key::Return);
        map.insert(0x29, Key::Escape);
        map.insert(0x2A, Key::Backspace);
        map.insert(0x2B, Key::Tab);
        map.insert(0x2C, Key::Space);
        map.insert(0x2D, Key::Unicode('-'));
        map.insert(0x2E, Key::Unicode('='));
        map.insert(0x2F, Key::Unicode('['));
        map.insert(0x30, Key::Unicode(']'));
        map.insert(0x31, Key::Unicode('\\'));
        map.insert(0x33, Key::Unicode(';'));
        map.insert(0x34, Key::Unicode('\''));
        map.insert(0x35, Key::Unicode('`'));
        map.insert(0x36, Key::Unicode(','));
        map.insert(0x37, Key::Unicode('.'));
        map.insert(0x38, Key::Unicode('/'));

        // Function keys (0x3A - 0x45 = F1-F12)
        map.insert(0x3A, Key::F1);
        map.insert(0x3B, Key::F2);
        map.insert(0x3C, Key::F3);
        map.insert(0x3D, Key::F4);
        map.insert(0x3E, Key::F5);
        map.insert(0x3F, Key::F6);
        map.insert(0x40, Key::F7);
        map.insert(0x41, Key::F8);
        map.insert(0x42, Key::F9);
        map.insert(0x43, Key::F10);
        map.insert(0x44, Key::F11);
        map.insert(0x45, Key::F12);

        // Navigation keys
        map.insert(0x4A, Key::Home);
        map.insert(0x4B, Key::PageUp);
        map.insert(0x4C, Key::Delete);
        map.insert(0x4D, Key::End);
        map.insert(0x4E, Key::PageDown);

        // Arrow keys
        map.insert(0x4F, Key::RightArrow);
        map.insert(0x50, Key::LeftArrow);
        map.insert(0x51, Key::DownArrow);
        map.insert(0x52, Key::UpArrow);

        map
    }

    /// Get current input mode
    pub fn get_mode(&self) -> InputMode {
        self.current_mode
    }

    /// Switch input mode
    pub fn switch_mode(&mut self, mode: InputMode) {
        self.current_mode = mode;
    }

    /// Toggle between mouse and keyboard mode
    pub fn toggle_mode(&mut self) -> InputMode {
        self.current_mode = match self.current_mode {
            InputMode::Mouse => InputMode::Keyboard,
            InputMode::Keyboard => InputMode::Mouse,
        };
        self.current_mode
    }

    /// Set mouse movement speed (pixels per command)
    pub fn set_mouse_speed(&mut self, speed: i32) {
        self.mouse_speed = speed.clamp(1, 50);
    }

    /// Handle mouse movement
    pub fn mouse_move(&mut self, movement: MouseMove) {
        let dx = movement.dx as i32 * self.mouse_speed;
        let dy = movement.dy as i32 * self.mouse_speed;
        let _ = self.enigo.move_mouse(dx, dy, Coordinate::Rel);
    }

    /// Handle mouse click
    pub fn mouse_click(&mut self, click: MouseClick) {
        let button = match click.button {
            MouseButton::Left => EnigoButton::Left,
            MouseButton::Right => EnigoButton::Right,
            MouseButton::Middle => EnigoButton::Middle,
        };

        match click.action {
            ClickAction::Press => {
                let _ = self.enigo.button(button, Direction::Press);
            }
            ClickAction::Release => {
                let _ = self.enigo.button(button, Direction::Release);
            }
            ClickAction::Click => {
                let _ = self.enigo.button(button, Direction::Click);
            }
            ClickAction::DoubleClick => {
                let _ = self.enigo.button(button, Direction::Click);
                std::thread::sleep(std::time::Duration::from_millis(50));
                let _ = self.enigo.button(button, Direction::Click);
            }
        }
    }

    /// Handle key press
    pub fn key_press(&mut self, event: KeyEvent) {
        // Apply modifiers
        if event.modifiers & 0x01 != 0 {
            let _ = self.enigo.key(Key::Control, Direction::Press);
        }
        if event.modifiers & 0x02 != 0 {
            let _ = self.enigo.key(Key::Shift, Direction::Press);
        }
        if event.modifiers & 0x04 != 0 {
            let _ = self.enigo.key(Key::Alt, Direction::Press);
        }
        if event.modifiers & 0x08 != 0 {
            let _ = self.enigo.key(Key::Meta, Direction::Press);
        }

        // Press the key
        if let Some(&key) = self.keymap.get(&event.keycode) {
            let _ = self.enigo.key(key, Direction::Press);
        }
    }

    /// Handle key release
    pub fn key_release(&mut self, event: KeyEvent) {
        // Release the key
        if let Some(&key) = self.keymap.get(&event.keycode) {
            let _ = self.enigo.key(key, Direction::Release);
        }

        // Release modifiers
        if event.modifiers & 0x01 != 0 {
            let _ = self.enigo.key(Key::Control, Direction::Release);
        }
        if event.modifiers & 0x02 != 0 {
            let _ = self.enigo.key(Key::Shift, Direction::Release);
        }
        if event.modifiers & 0x04 != 0 {
            let _ = self.enigo.key(Key::Alt, Direction::Release);
        }
        if event.modifiers & 0x08 != 0 {
            let _ = self.enigo.key(Key::Meta, Direction::Release);
        }
    }

    /// Type a string (for keyboard mode)
    pub fn type_string(&mut self, text: &str) {
        let _ = self.enigo.text(text);
    }

    /// Handle arrow key input in mouse mode (convert to mouse movement)
    pub fn arrow_to_mouse(&mut self, keycode: u8) {
        let movement = match keycode {
            0x4F => MouseMove { dx: 1, dy: 0 },  // Right
            0x50 => MouseMove { dx: -1, dy: 0 }, // Left
            0x51 => MouseMove { dx: 0, dy: 1 },  // Down
            0x52 => MouseMove { dx: 0, dy: -1 }, // Up
            _ => return,
        };
        self.mouse_move(movement);
    }
}

/// Modifier key flags
pub mod modifiers {
    pub const CTRL: u8 = 0x01;
    pub const SHIFT: u8 = 0x02;
    pub const ALT: u8 = 0x04;
    pub const GUI: u8 = 0x08;
}

/// USB HID keycodes for common keys
pub mod keycodes {
    pub const KEY_A: u8 = 0x04;
    pub const KEY_Z: u8 = 0x1D;
    pub const KEY_1: u8 = 0x1E;
    pub const KEY_0: u8 = 0x27;
    pub const KEY_ENTER: u8 = 0x28;
    pub const KEY_ESCAPE: u8 = 0x29;
    pub const KEY_BACKSPACE: u8 = 0x2A;
    pub const KEY_TAB: u8 = 0x2B;
    pub const KEY_SPACE: u8 = 0x2C;
    pub const KEY_RIGHT: u8 = 0x4F;
    pub const KEY_LEFT: u8 = 0x50;
    pub const KEY_DOWN: u8 = 0x51;
    pub const KEY_UP: u8 = 0x52;
    pub const KEY_F1: u8 = 0x3A;
    pub const KEY_F12: u8 = 0x45;
}
