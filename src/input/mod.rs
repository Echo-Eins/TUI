//! Input simulation module - mouse and keyboard control.
//!
//! Maps Cardputer input commands to host input events.

#[cfg(any(windows, target_os = "linux"))]
use crate::protocol::{ClickAction, MouseButton};
use crate::protocol::{InputMode, KeyEvent, MouseClick, MouseMove};
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

#[cfg(windows)]
mod imp {
    use super::*;
    use enigo::{
        Button as EnigoButton, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings,
    };
    use std::collections::HashMap;

    /// Input controller for mouse and keyboard.
    pub struct InputController {
        enigo: Enigo,
        current_mode: InputMode,
        /// Mouse movement speed in pixels per command.
        mouse_speed: i32,
        /// Keycode mapping from USB HID to enigo keys.
        keymap: HashMap<u8, Key>,
    }

    impl InputController {
        /// Create a new input controller.
        pub fn new() -> Result<Self, InputError> {
            let enigo = Enigo::new(&Settings::default())
                .map_err(|error| InputError::InitError(error.to_string()))?;
            let keymap = Self::build_keymap();

            Ok(Self {
                enigo,
                current_mode: InputMode::Mouse,
                mouse_speed: 5,
                keymap,
            })
        }

        /// Build USB HID keycode to enigo Key mapping.
        fn build_keymap() -> HashMap<u8, Key> {
            let mut map = HashMap::new();

            for i in 0..26u8 {
                let c = (b'a' + i) as char;
                map.insert(0x04 + i, Key::Unicode(c));
            }

            for i in 0..9u8 {
                let c = (b'1' + i) as char;
                map.insert(0x1E + i, Key::Unicode(c));
            }
            map.insert(0x27, Key::Unicode('0'));

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

            map.insert(0x4A, Key::Home);
            map.insert(0x4B, Key::PageUp);
            map.insert(0x4C, Key::Delete);
            map.insert(0x4D, Key::End);
            map.insert(0x4E, Key::PageDown);
            map.insert(0x4F, Key::RightArrow);
            map.insert(0x50, Key::LeftArrow);
            map.insert(0x51, Key::DownArrow);
            map.insert(0x52, Key::UpArrow);

            map
        }

        pub fn get_mode(&self) -> InputMode {
            self.current_mode
        }

        pub fn switch_mode(&mut self, mode: InputMode) {
            self.current_mode = mode;
        }

        pub fn toggle_mode(&mut self) -> InputMode {
            self.current_mode = match self.current_mode {
                InputMode::Mouse => InputMode::Keyboard,
                InputMode::Keyboard => InputMode::Mouse,
            };
            self.current_mode
        }

        pub fn set_mouse_speed(&mut self, speed: i32) {
            self.mouse_speed = speed.clamp(1, 50);
        }

        pub fn mouse_move(&mut self, movement: MouseMove) {
            let dx = movement.dx as i32 * self.mouse_speed;
            let dy = movement.dy as i32 * self.mouse_speed;
            let _ = self.enigo.move_mouse(dx, dy, Coordinate::Rel);
        }

        pub fn mouse_click(&mut self, click: MouseClick) {
            let button = match click.button {
                MouseButton::Left => EnigoButton::Left,
                MouseButton::Right => EnigoButton::Right,
                MouseButton::Middle => EnigoButton::Middle,
            };

            match click.action {
                ClickAction::Press => self.button(button, Direction::Press),
                ClickAction::Release => self.button(button, Direction::Release),
                ClickAction::Click => self.button(button, Direction::Click),
                ClickAction::DoubleClick => {
                    self.button(button, Direction::Click);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    self.button(button, Direction::Click);
                }
            }
        }

        fn button(&mut self, button: EnigoButton, direction: Direction) {
            let _ = self.enigo.button(button, direction);
        }

        pub fn key_press(&mut self, event: KeyEvent) {
            self.apply_modifiers(event.modifiers, Direction::Press);
            self.key(event.keycode, Direction::Press);
        }

        pub fn key_release(&mut self, event: KeyEvent) {
            self.key(event.keycode, Direction::Release);
            self.apply_modifiers(event.modifiers, Direction::Release);
        }

        pub fn type_string(&mut self, text: &str) {
            let _ = self.enigo.text(text);
        }

        fn apply_modifiers(&mut self, modifiers: u8, direction: Direction) {
            if modifiers & 0x01 != 0 {
                self.key_named(Key::Control, direction);
            }
            if modifiers & 0x02 != 0 {
                self.key_named(Key::Shift, direction);
            }
            if modifiers & 0x04 != 0 {
                self.key_named(Key::Alt, direction);
            }
            if modifiers & 0x08 != 0 {
                self.key_named(Key::Meta, direction);
            }
        }

        fn key(&mut self, keycode: u8, direction: Direction) {
            if let Some(&key) = self.keymap.get(&keycode) {
                self.key_named(key, direction);
            }
        }

        fn key_named(&mut self, key: Key, direction: Direction) {
            let _ = self.enigo.key(key, direction);
        }

        pub fn arrow_to_mouse(&mut self, keycode: u8) {
            let movement = match keycode {
                0x4F => MouseMove { dx: 1, dy: 0 },
                0x50 => MouseMove { dx: -1, dy: 0 },
                0x51 => MouseMove { dx: 0, dy: 1 },
                0x52 => MouseMove { dx: 0, dy: -1 },
                _ => return,
            };
            self.mouse_move(movement);
        }
    }

    pub use InputController as ImplInputController;
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::process::Command;

    /// Linux input controller backed by xdotool. Keeping this backend free of
    /// link-time X11 dependencies lets native Linux builds work without libxdo-dev.
    pub struct InputController {
        current_mode: InputMode,
        /// Mouse movement speed in pixels per command.
        mouse_speed: i32,
    }

    impl InputController {
        /// Create a new input controller.
        pub fn new() -> Result<Self, InputError> {
            if !command_exists("xdotool") {
                return Err(InputError::InitError(
                    "Linux input injection requires xdotool at runtime".to_string(),
                ));
            }

            Ok(Self {
                current_mode: InputMode::Mouse,
                mouse_speed: 5,
            })
        }

        pub fn get_mode(&self) -> InputMode {
            self.current_mode
        }

        pub fn switch_mode(&mut self, mode: InputMode) {
            self.current_mode = mode;
        }

        pub fn toggle_mode(&mut self) -> InputMode {
            self.current_mode = match self.current_mode {
                InputMode::Mouse => InputMode::Keyboard,
                InputMode::Keyboard => InputMode::Mouse,
            };
            self.current_mode
        }

        pub fn set_mouse_speed(&mut self, speed: i32) {
            self.mouse_speed = speed.clamp(1, 50);
        }

        pub fn mouse_move(&mut self, movement: MouseMove) {
            let dx = movement.dx as i32 * self.mouse_speed;
            let dy = movement.dy as i32 * self.mouse_speed;
            run_xdotool(&[
                "mousemove_relative".to_string(),
                "--".to_string(),
                dx.to_string(),
                dy.to_string(),
            ]);
        }

        pub fn mouse_click(&mut self, click: MouseClick) {
            let button = match click.button {
                MouseButton::Left => 1,
                MouseButton::Middle => 2,
                MouseButton::Right => 3,
            };

            match click.action {
                ClickAction::Press => run_xdotool(&["mousedown".to_string(), button.to_string()]),
                ClickAction::Release => run_xdotool(&["mouseup".to_string(), button.to_string()]),
                ClickAction::Click => run_xdotool(&["click".to_string(), button.to_string()]),
                ClickAction::DoubleClick => {
                    run_xdotool(&["click".to_string(), button.to_string()]);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    run_xdotool(&["click".to_string(), button.to_string()]);
                }
            }
        }

        pub fn key_press(&mut self, event: KeyEvent) {
            self.apply_modifiers(event.modifiers, "keydown");
            self.key(event.keycode, "keydown");
        }

        pub fn key_release(&mut self, event: KeyEvent) {
            self.key(event.keycode, "keyup");
            self.apply_modifiers(event.modifiers, "keyup");
        }

        pub fn type_string(&mut self, text: &str) {
            run_xdotool(&[
                "type".to_string(),
                "--delay".to_string(),
                "0".to_string(),
                text.to_string(),
            ]);
        }

        fn apply_modifiers(&mut self, modifiers: u8, action: &str) {
            if modifiers & 0x01 != 0 {
                run_xdotool(&[action.to_string(), "ctrl".to_string()]);
            }
            if modifiers & 0x02 != 0 {
                run_xdotool(&[action.to_string(), "shift".to_string()]);
            }
            if modifiers & 0x04 != 0 {
                run_xdotool(&[action.to_string(), "alt".to_string()]);
            }
            if modifiers & 0x08 != 0 {
                run_xdotool(&[action.to_string(), "super".to_string()]);
            }
        }

        fn key(&mut self, keycode: u8, action: &str) {
            if let Some(name) = xdotool_key_name(keycode) {
                run_xdotool(&[action.to_string(), name.to_string()]);
            }
        }

        pub fn arrow_to_mouse(&mut self, keycode: u8) {
            let movement = match keycode {
                0x4F => MouseMove { dx: 1, dy: 0 },
                0x50 => MouseMove { dx: -1, dy: 0 },
                0x51 => MouseMove { dx: 0, dy: 1 },
                0x52 => MouseMove { dx: 0, dy: -1 },
                _ => return,
            };
            self.mouse_move(movement);
        }
    }

    fn command_exists(program: &str) -> bool {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
            .unwrap_or(false)
    }

    fn run_xdotool(args: &[String]) {
        let _ = Command::new("xdotool").args(args).status();
    }

    fn xdotool_key_name(keycode: u8) -> Option<&'static str> {
        match keycode {
            0x04 => Some("a"),
            0x05 => Some("b"),
            0x06 => Some("c"),
            0x07 => Some("d"),
            0x08 => Some("e"),
            0x09 => Some("f"),
            0x0A => Some("g"),
            0x0B => Some("h"),
            0x0C => Some("i"),
            0x0D => Some("j"),
            0x0E => Some("k"),
            0x0F => Some("l"),
            0x10 => Some("m"),
            0x11 => Some("n"),
            0x12 => Some("o"),
            0x13 => Some("p"),
            0x14 => Some("q"),
            0x15 => Some("r"),
            0x16 => Some("s"),
            0x17 => Some("t"),
            0x18 => Some("u"),
            0x19 => Some("v"),
            0x1A => Some("w"),
            0x1B => Some("x"),
            0x1C => Some("y"),
            0x1D => Some("z"),
            0x1E => Some("1"),
            0x1F => Some("2"),
            0x20 => Some("3"),
            0x21 => Some("4"),
            0x22 => Some("5"),
            0x23 => Some("6"),
            0x24 => Some("7"),
            0x25 => Some("8"),
            0x26 => Some("9"),
            0x27 => Some("0"),
            0x28 => Some("Return"),
            0x29 => Some("Escape"),
            0x2A => Some("BackSpace"),
            0x2B => Some("Tab"),
            0x2C => Some("space"),
            0x2D => Some("minus"),
            0x2E => Some("equal"),
            0x2F => Some("bracketleft"),
            0x30 => Some("bracketright"),
            0x31 => Some("backslash"),
            0x33 => Some("semicolon"),
            0x34 => Some("apostrophe"),
            0x35 => Some("grave"),
            0x36 => Some("comma"),
            0x37 => Some("period"),
            0x38 => Some("slash"),
            0x3A => Some("F1"),
            0x3B => Some("F2"),
            0x3C => Some("F3"),
            0x3D => Some("F4"),
            0x3E => Some("F5"),
            0x3F => Some("F6"),
            0x40 => Some("F7"),
            0x41 => Some("F8"),
            0x42 => Some("F9"),
            0x43 => Some("F10"),
            0x44 => Some("F11"),
            0x45 => Some("F12"),
            0x4A => Some("Home"),
            0x4B => Some("Page_Up"),
            0x4C => Some("Delete"),
            0x4D => Some("End"),
            0x4E => Some("Page_Down"),
            0x4F => Some("Right"),
            0x50 => Some("Left"),
            0x51 => Some("Down"),
            0x52 => Some("Up"),
            _ => None,
        }
    }

    pub use InputController as ImplInputController;
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use super::*;

    /// Input controller stub for unsupported hosts.
    pub struct InputController {
        current_mode: InputMode,
        mouse_speed: i32,
    }

    impl InputController {
        pub fn new() -> Result<Self, InputError> {
            Ok(Self {
                current_mode: InputMode::Mouse,
                mouse_speed: 5,
            })
        }

        pub fn get_mode(&self) -> InputMode {
            self.current_mode
        }

        pub fn switch_mode(&mut self, mode: InputMode) {
            self.current_mode = mode;
        }

        pub fn toggle_mode(&mut self) -> InputMode {
            self.current_mode = match self.current_mode {
                InputMode::Mouse => InputMode::Keyboard,
                InputMode::Keyboard => InputMode::Mouse,
            };
            self.current_mode
        }

        pub fn set_mouse_speed(&mut self, speed: i32) {
            self.mouse_speed = speed.clamp(1, 50);
        }

        pub fn mouse_move(&mut self, _movement: MouseMove) {}

        pub fn mouse_click(&mut self, _click: MouseClick) {}

        pub fn key_press(&mut self, _event: KeyEvent) {}

        pub fn key_release(&mut self, _event: KeyEvent) {}

        pub fn type_string(&mut self, _text: &str) {}

        pub fn arrow_to_mouse(&mut self, keycode: u8) {
            let movement = match keycode {
                0x4F => MouseMove { dx: 1, dy: 0 },
                0x50 => MouseMove { dx: -1, dy: 0 },
                0x51 => MouseMove { dx: 0, dy: 1 },
                0x52 => MouseMove { dx: 0, dy: -1 },
                _ => return,
            };
            self.mouse_move(movement);
        }
    }

    pub use InputController as ImplInputController;
}

pub use imp::ImplInputController as InputController;

/// Modifier key flags.
pub mod modifiers {
    pub const CTRL: u8 = 0x01;
    pub const SHIFT: u8 = 0x02;
    pub const ALT: u8 = 0x04;
    pub const GUI: u8 = 0x08;
}

/// USB HID keycodes for common keys.
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
