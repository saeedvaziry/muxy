use gpui::Keystroke;
use muxy_protocol::Modes;

pub(crate) fn encode(key: &Keystroke, modes: Modes) -> Option<Vec<u8>> {
    let modifiers = key.modifiers;
    if modifiers.platform {
        return None;
    }
    let bytes = match key.key.as_str() {
        "enter" | "return" => b"\r".to_vec(),
        "backspace" => vec![0x7f],
        "escape" => vec![0x1b],
        "tab" if modifiers.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "up" => cursor(b'A', modes),
        "down" => cursor(b'B', modes),
        "right" => cursor(b'C', modes),
        "left" => cursor(b'D', modes),
        "home" => cursor(b'H', modes),
        "end" => cursor(b'F', modes),
        "pageup" => b"\x1b[5~".to_vec(),
        "pagedown" => b"\x1b[6~".to_vec(),
        "insert" => b"\x1b[2~".to_vec(),
        "delete" => b"\x1b[3~".to_vec(),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => b"\x1b[15~".to_vec(),
        "f6" => b"\x1b[17~".to_vec(),
        "f7" => b"\x1b[18~".to_vec(),
        "f8" => b"\x1b[19~".to_vec(),
        "f9" => b"\x1b[20~".to_vec(),
        "f10" => b"\x1b[21~".to_vec(),
        "f11" => b"\x1b[23~".to_vec(),
        "f12" => b"\x1b[24~".to_vec(),
        _ if modifiers.control => vec![control(&key.key)?],
        _ if modifiers.alt && key.key.chars().count() == 1 => {
            if modifiers.shift {
                key.key.to_uppercase().into_bytes()
            } else {
                key.key.as_bytes().to_vec()
            }
        }
        "space" => vec![b' '],
        _ => {
            let text = key.key_char.as_deref().filter(|text| !text.is_empty())?;
            if text.chars().any(char::is_control) {
                return None;
            }
            text.as_bytes().to_vec()
        }
    };
    if modifiers.alt {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0x1b);
        prefixed.extend(bytes);
        Some(prefixed)
    } else {
        Some(bytes)
    }
}

fn cursor(suffix: u8, modes: Modes) -> Vec<u8> {
    vec![
        0x1b,
        if modes.application_cursor_keys {
            b'O'
        } else {
            b'['
        },
        suffix,
    ]
}

fn control(key: &str) -> Option<u8> {
    match key {
        "space" | " " | "@" | "2" => Some(0),
        "[" | "3" => Some(0x1b),
        "\\" | "4" => Some(0x1c),
        "]" | "5" => Some(0x1d),
        "^" | "6" => Some(0x1e),
        "_" | "-" | "7" | "/" => Some(0x1f),
        "?" | "8" => Some(0x7f),
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() => {
            Some(key.as_bytes()[0].to_ascii_uppercase() - b'@')
        }
        _ => None,
    }
}
