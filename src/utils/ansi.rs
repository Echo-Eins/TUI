/// ANSI escape code stripping and visible-width calculation.
///
/// Handles:
/// - CSI sequences: `\x1b[...` (colors, cursor, erase, etc.)
/// - OSC sequences: `\x1b]...ST` (terminal title, hyperlinks, etc.)
/// - Single-byte C1 controls: `\x1b` followed by `@`..`_`
/// - SGR reset shorthand: `\x1b[m`

/// Strip all ANSI escape sequences from the input string.
pub fn strip_ansi(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ESC character — start of an escape sequence
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: ESC [ ... final_byte
                    chars.next(); // consume '['
                    // Read parameter bytes (0x30–0x3F) and intermediate bytes (0x20–0x2F)
                    // then final byte (0x40–0x7E)
                    loop {
                        match chars.peek() {
                            Some(&ch) if ('\x40'..='\x7E').contains(&ch) => {
                                chars.next(); // consume final byte
                                break;
                            }
                            Some(&ch) if ('\x20'..='\x3F').contains(&ch) => {
                                chars.next(); // consume parameter/intermediate byte
                            }
                            None => break, // unterminated sequence
                            _ => {
                                // Unexpected byte — consume and break
                                chars.next();
                                break;
                            }
                        }
                    }
                }
                Some(']') => {
                    // OSC sequence: ESC ] ... (ST or BEL)
                    // ST = ESC \ or 0x9C; BEL = 0x07
                    chars.next(); // consume ']'
                    loop {
                        match chars.next() {
                            Some('\x07') => break,             // BEL terminator
                            Some('\x1b') => {
                                // Check for ST (ESC \)
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                            Some('\u{9c}') => break,             // 8-bit ST
                            None => break,                     // unterminated
                            _ => {}                            // skip payload
                        }
                    }
                }
                Some(&ch) if ('\x40'..='\x5F').contains(&ch) => {
                    // Single-byte C1 control (Fe escape): ESC + '@'..'_'
                    chars.next();
                }
                _ => {
                    // Unknown or malformed — skip the ESC only
                }
            }
        } else if c == '\u{9b}' {
            // 8-bit CSI (rare but possible)
            loop {
                match chars.peek() {
                    Some(&ch) if ('\x40'..='\x7E').contains(&ch) => {
                        chars.next();
                        break;
                    }
                    Some(&ch) if ('\x20'..='\x3F').contains(&ch) => {
                        chars.next();
                    }
                    None => break,
                    _ => {
                        chars.next();
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Return the visible (display column) width of a string, ignoring ANSI sequences.
///
/// Uses Unicode width rules: ASCII = 1 column, CJK fullwidth = 2 columns,
/// zero-width characters = 0 columns.
pub fn visible_width(input: &str) -> usize {
    let stripped = strip_ansi(input);
    unicode_width::UnicodeWidthStr::width(stripped.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_plain_text_unchanged() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn strip_sgr_color_codes() {
        assert_eq!(
            strip_ansi("\x1b[31mred\x1b[0m normal"),
            "red normal"
        );
        assert_eq!(
            strip_ansi("\x1b[1;32;40mbold green\x1b[m"),
            "bold green"
        );
    }

    #[test]
    fn strip_cursor_movement() {
        assert_eq!(strip_ansi("\x1b[2J\x1b[Hhello"), "hello");
        assert_eq!(strip_ansi("\x1b[10;20Htext"), "text");
    }

    #[test]
    fn strip_osc_title_sequence() {
        assert_eq!(
            strip_ansi("\x1b]0;Window Title\x07visible"),
            "visible"
        );
        assert_eq!(
            strip_ansi("\x1b]0;Title\x1b\\visible"),
            "visible"
        );
    }

    #[test]
    fn strip_mixed_sequences() {
        let input = "\x1b[32m➜\x1b[0m \x1b[36m~/code\x1b[0m \x1b[33mgit:\x1b[0m\x1b[31m(main)\x1b[0m";
        assert_eq!(strip_ansi(input), "➜ ~/code git:(main)");
    }

    #[test]
    fn visible_width_ascii() {
        assert_eq!(visible_width("hello"), 5);
    }

    #[test]
    fn visible_width_with_ansi() {
        assert_eq!(visible_width("\x1b[31mhello\x1b[0m"), 5);
    }

    #[test]
    fn visible_width_cjk() {
        // Each CJK character is 2 columns wide
        assert_eq!(visible_width("日本"), 4);
    }

    #[test]
    fn strip_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn strip_only_escape_codes() {
        assert_eq!(strip_ansi("\x1b[0m\x1b[31m\x1b[0m"), "");
    }

    #[test]
    fn visible_width_cyrillic() {
        // Cyrillic chars are 1 column wide each
        assert_eq!(visible_width("привет"), 6);
    }
}
