use ratatui::style::Color;

// ── Token Types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Command,
    Argument,
    Flag,        // starts with -
    Path,        // contains /
    QuotedString,
    Pipe,        // |
    Redirect,    // > >> < << 2>
    Semicolon,   // ;
    And,         // &&
    Or,          // ||
    Whitespace,
    Variable,    // $VAR or ${VAR}
}

#[derive(Debug, Clone)]
pub struct InputToken {
    pub text: String,
    pub kind: TokenKind,
    pub valid: Option<bool>, // None = unchecked, Some(true/false) = verified
}

impl InputToken {
    /// Get the display color for this token.
    pub fn color(&self) -> Color {
        match self.kind {
            TokenKind::Command => {
                match self.valid {
                    Some(true) => Color::Green,
                    Some(false) => Color::Red,
                    None => Color::White,
                }
            }
            TokenKind::Path => {
                match self.valid {
                    Some(true) => Color::Green,
                    Some(false) => Color::Red,
                    None => Color::White,
                }
            }
            TokenKind::Flag => Color::Cyan,
            TokenKind::QuotedString => Color::Yellow,
            TokenKind::Variable => Color::Magenta,
            TokenKind::Pipe | TokenKind::And | TokenKind::Or => Color::Magenta,
            TokenKind::Redirect => Color::Magenta,
            TokenKind::Semicolon => Color::DarkGray,
            TokenKind::Whitespace => Color::Reset,
            TokenKind::Argument => Color::White,
        }
    }
}

// ── Tokenizer ──────────────────────────────────────────────────────────────

/// Tokenize input into classified tokens for syntax highlighting.
///
/// `is_known_command` is a callback that checks if a command name exists
/// in the shell builtins, PATH, or aliases.
pub fn tokenize<F>(input: &str, is_known_command: F) -> Vec<InputToken>
where
    F: Fn(&str) -> bool,
{
    if input.is_empty() {
        return Vec::new();
    }

    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    let mut is_first_token = true; // track if we're at the command position
    let mut after_pipe_or_semi = false; // next non-whitespace token is a command

    while chars.peek().is_some() {
        // Consume whitespace
        let mut ws = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                ws.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if !ws.is_empty() {
            tokens.push(InputToken {
                text: ws,
                kind: TokenKind::Whitespace,
                valid: None,
            });
        }

        if chars.peek().is_none() {
            break;
        }

        let c = *chars.peek().unwrap();

        // Check for operators
        if c == '|' {
            chars.next();
            if chars.peek() == Some(&'|') {
                chars.next();
                tokens.push(InputToken { text: "||".to_string(), kind: TokenKind::Or, valid: None });
            } else {
                tokens.push(InputToken { text: "|".to_string(), kind: TokenKind::Pipe, valid: None });
            }
            is_first_token = false;
            after_pipe_or_semi = true;
            continue;
        }

        if c == '&' {
            chars.next();
            if chars.peek() == Some(&'&') {
                chars.next();
                tokens.push(InputToken { text: "&&".to_string(), kind: TokenKind::And, valid: None });
                after_pipe_or_semi = true;
            } else {
                tokens.push(InputToken { text: "&".to_string(), kind: TokenKind::Argument, valid: None });
            }
            is_first_token = false;
            continue;
        }

        if c == ';' {
            chars.next();
            tokens.push(InputToken { text: ";".to_string(), kind: TokenKind::Semicolon, valid: None });
            is_first_token = false;
            after_pipe_or_semi = true;
            continue;
        }

        // Redirections: > >> < << 2> 2>>
        if c == '>' || c == '<' {
            let mut redir = String::new();
            redir.push(c);
            chars.next();
            if chars.peek() == Some(&c) {
                redir.push(c);
                chars.next();
            }
            tokens.push(InputToken { text: redir, kind: TokenKind::Redirect, valid: None });
            is_first_token = false;
            continue;
        }

        // Quoted strings
        if c == '\'' || c == '"' {
            let quote = c;
            let mut quoted = String::new();
            quoted.push(c);
            chars.next();
            while let Some(&ch) = chars.peek() {
                quoted.push(ch);
                chars.next();
                if ch == quote {
                    break;
                }
            }
            tokens.push(InputToken {
                text: quoted,
                kind: TokenKind::QuotedString,
                valid: None,
            });
            is_first_token = false;
            continue;
        }

        // Variables: $VAR or ${VAR}
        if c == '$' {
            let mut var = String::new();
            var.push(c);
            chars.next();
            if chars.peek() == Some(&'{') {
                var.push('{');
                chars.next();
                while let Some(&ch) = chars.peek() {
                    var.push(ch);
                    chars.next();
                    if ch == '}' {
                        break;
                    }
                }
            } else {
                while let Some(&ch) = chars.peek() {
                    if ch.is_alphanumeric() || ch == '_' {
                        var.push(ch);
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            tokens.push(InputToken {
                text: var,
                kind: TokenKind::Variable,
                valid: None,
            });
            is_first_token = false;
            continue;
        }

        // Regular word
        let mut word = String::new();
        while let Some(&ch) = chars.peek() {
            if ch.is_whitespace() || ch == '|' || ch == '&' || ch == ';'
                || ch == '>' || ch == '<' || ch == '\'' || ch == '"' || ch == '$'
            {
                break;
            }
            word.push(ch);
            chars.next();
        }

        if word.is_empty() {
            continue;
        }

        // Classify the word
        let is_command_position = is_first_token || after_pipe_or_semi;

        let kind;
        let valid;

        if is_command_position {
            kind = TokenKind::Command;
            valid = Some(is_known_command(&word));
            after_pipe_or_semi = false;
        } else if word.starts_with('-') {
            kind = TokenKind::Flag;
            valid = None;
        } else if word.contains('/') || word.starts_with('~') || word.starts_with('.') {
            // Looks like a path — validate existence
            kind = TokenKind::Path;
            let expanded = if word.starts_with('~') {
                dirs::home_dir()
                    .map(|h| word.replacen('~', &h.display().to_string(), 1))
                    .unwrap_or_else(|| word.clone())
            } else {
                word.clone()
            };
            valid = Some(std::path::Path::new(&expanded).exists());
        } else {
            kind = TokenKind::Argument;
            valid = None;
        }

        tokens.push(InputToken {
            text: word,
            kind,
            valid,
        });

        is_first_token = false;
    }

    tokens
}

/// Convert tokenized input to colored (text, color) pairs for rendering.
pub fn highlight(input: &str, is_known_command: impl Fn(&str) -> bool) -> Vec<(String, Color)> {
    let tokens = tokenize(input, is_known_command);
    tokens.into_iter()
        .map(|t| {
            let color = t.color();
            (t.text, color)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always_known(cmd: &str) -> bool {
        matches!(cmd, "ls" | "echo" | "cat" | "grep" | "systemctl")
    }

    #[test]
    fn test_simple_command() {
        let tokens = tokenize("ls -la /tmp", always_known);
        assert_eq!(tokens.len(), 5); // "ls" " " "-la" " " "/tmp"
        assert_eq!(tokens[0].kind, TokenKind::Command);
        assert_eq!(tokens[0].valid, Some(true));
        assert_eq!(tokens[2].kind, TokenKind::Flag);
        assert_eq!(tokens[4].kind, TokenKind::Path);
    }

    #[test]
    fn test_unknown_command() {
        let tokens = tokenize("sysemctl restart", always_known);
        assert_eq!(tokens[0].kind, TokenKind::Command);
        assert_eq!(tokens[0].valid, Some(false)); // typo
    }

    #[test]
    fn test_pipe() {
        let tokens = tokenize("cat file | grep foo", always_known);
        let commands: Vec<&InputToken> = tokens.iter()
            .filter(|t| t.kind == TokenKind::Command)
            .collect();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].text, "cat");
        assert_eq!(commands[1].text, "grep");
    }

    #[test]
    fn test_quoted_string() {
        let tokens = tokenize("echo \"hello world\"", always_known);
        let quoted: Vec<&InputToken> = tokens.iter()
            .filter(|t| t.kind == TokenKind::QuotedString)
            .collect();
        assert_eq!(quoted.len(), 1);
        assert_eq!(quoted[0].text, "\"hello world\"");
    }

    #[test]
    fn test_variable() {
        let tokens = tokenize("echo $HOME", always_known);
        let vars: Vec<&InputToken> = tokens.iter()
            .filter(|t| t.kind == TokenKind::Variable)
            .collect();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].text, "$HOME");
    }

    #[test]
    fn test_redirect() {
        let tokens = tokenize("echo hello > file.txt", always_known);
        let redirects: Vec<&InputToken> = tokens.iter()
            .filter(|t| t.kind == TokenKind::Redirect)
            .collect();
        assert_eq!(redirects.len(), 1);
    }
}
