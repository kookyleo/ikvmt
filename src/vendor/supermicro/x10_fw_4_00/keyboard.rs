use anyhow::{Result, bail};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Text { text: String },
    Key { key: String },
    Chord { keys: Vec<String> },
    Secret { env: String },
}

#[derive(Clone, Debug)]
pub struct KeyEvent {
    pub code: u32,
    pub down: bool,
    pub action: usize,
}

pub fn packet(code: u32, down: bool) -> [u8; 18] {
    let mut p = [0; 18];
    p[0] = 4;
    p[2] = u8::from(down);
    p[5..9].copy_from_slice(&code.to_be_bytes());
    p
}

fn character(c: char) -> Result<(u32, bool)> {
    if c.is_ascii_alphabetic() {
        return Ok((
            u32::from(c.to_ascii_lowercase()) - 97 + 4,
            c.is_ascii_uppercase(),
        ));
    }
    let plain = "1234567890-=[]\\;'`,./";
    let shifted = "!@#$%^&*()_+{}|:\"~<>?";
    let codes = [
        30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 45, 46, 47, 48, 49, 51, 52, 53, 54, 55, 56,
    ];
    if c == ' ' {
        return Ok((44, false));
    }
    for (chars, shift) in [(plain, false), (shifted, true)] {
        if let Some(i) = chars.chars().position(|v| v == c) {
            return Ok((codes[i], shift));
        }
    }
    bail!(
        "UNSUPPORTED_CHARACTER: U+{:04X}; text accepts printable US ASCII only",
        u32::from(c)
    )
}

pub fn key(name: &str) -> Result<u32> {
    if name.len() == 1 {
        let (code, shift) = character(name.chars().next().unwrap())?;
        if shift {
            bail!("UNSUPPORTED_ACTION: use text or an explicit Shift chord for {name}");
        }
        return Ok(code);
    }
    Ok(match name {
        "Enter" => 40,
        "Escape" | "Esc" => 41,
        "Backspace" => 42,
        "Tab" => 43,
        "Space" => 44,
        "CapsLock" => 57,
        "PrintScreen" => 70,
        "ScrollLock" => 71,
        "Pause" => 72,
        "Insert" => 73,
        "Home" => 74,
        "PageUp" => 75,
        "Delete" => 76,
        "End" => 77,
        "PageDown" => 78,
        "ArrowRight" => 79,
        "ArrowLeft" => 80,
        "ArrowDown" => 81,
        "ArrowUp" => 82,
        "Control" | "Ctrl" => 224,
        "Shift" => 225,
        "Alt" => 226,
        "Meta" => 227,
        _ => {
            if let Some(n) = name.strip_prefix('F').and_then(|s| s.parse::<u32>().ok())
                && (1..=12).contains(&n)
            {
                return Ok(57 + n);
            }
            bail!("UNSUPPORTED_ACTION: unknown key {name}")
        }
    })
}

/// Build the entire sequence before sending any byte, including resolving secrets.
pub fn compile(actions: &[Action]) -> Result<Vec<KeyEvent>> {
    if actions.is_empty() || actions.len() > 32 {
        bail!("INVALID_ARGUMENT: expected 1..32 actions");
    }
    let mut out = Vec::new();
    let mut text_len = 0;
    for (index, a) in actions.iter().enumerate() {
        let mut push = |code, down| {
            out.push(KeyEvent {
                code,
                down,
                action: index,
            })
        };
        match a {
            Action::Text { text } | Action::Secret { env: text } => {
                let secret;
                let value = if matches!(a, Action::Secret { .. }) {
                    secret = std::env::var(text).map_err(|_| {
                        anyhow::anyhow!(
                            "CREDENTIAL_UNAVAILABLE: secret environment variable is not set"
                        )
                    })?;
                    &secret
                } else {
                    text
                };
                text_len += value.chars().count();
                if text_len > 4096 {
                    bail!("INVALID_ARGUMENT: at most 4096 characters per call");
                }
                for c in value.chars() {
                    let (code, shift) = character(c)?;
                    if shift {
                        push(225, true);
                    }
                    push(code, true);
                    push(code, false);
                    if shift {
                        push(225, false);
                    }
                }
            }
            Action::Key { key: name } => {
                let code = key(name)?;
                push(code, true);
                push(code, false);
            }
            Action::Chord { keys } => {
                if keys.is_empty() || keys.len() > 6 {
                    bail!("INVALID_ARGUMENT: chord requires 1..6 keys");
                }
                let codes = keys.iter().map(|k| key(k)).collect::<Result<Vec<_>>>()?;
                let mut unique = codes.clone();
                unique.sort();
                unique.dedup();
                if unique.len() != codes.len() {
                    bail!("INVALID_ARGUMENT: duplicate chord key");
                }
                for &c in &codes {
                    push(c, true);
                }
                for &c in codes.iter().rev() {
                    push(c, false);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insyde_wire_format_is_not_standard_rfb() {
        assert_eq!(
            packet(40, true),
            [4, 0, 1, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }
    #[test]
    fn text_has_explicit_shift_and_no_enter() {
        let e = compile(&[Action::Text { text: "A@|".into() }]).unwrap();
        assert_eq!(e.len(), 12);
        assert_eq!(
            e.iter().map(|e| e.code).collect::<Vec<_>>(),
            vec![225, 4, 4, 225, 225, 31, 31, 225, 225, 49, 49, 225]
        );
        assert!(!e.iter().any(|e| e.code == 40));
    }
    #[test]
    fn validates_entire_input_before_dispatch() {
        assert!(
            compile(&[Action::Text {
                text: "valid\ninvalid".into()
            }])
            .is_err()
        );
        assert!(
            compile(&[Action::Text {
                text: "中文".into()
            }])
            .is_err()
        );
    }
}
