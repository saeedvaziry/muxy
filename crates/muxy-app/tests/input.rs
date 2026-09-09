#[path = "../src/views/terminal/input.rs"]
mod input;

use gpui::{Keystroke, Modifiers};
use muxy_protocol::Modes;

fn key(name: &str, text: Option<&str>, modifiers: Modifiers) -> Keystroke {
    Keystroke {
        key: name.into(),
        key_char: text.map(str::to_owned),
        modifiers,
    }
}

#[test]
fn named_keys_encode_in_normal_and_application_modes() {
    for application in [false, true] {
        let modes = Modes {
            application_cursor_keys: application,
            bracketed_paste: false,
        };
        for (name, expected) in [
            ("enter", "\r"),
            ("return", "\r"),
            ("backspace", "\x7f"),
            ("tab", "\t"),
            ("escape", "\x1b"),
            ("pageup", "\x1b[5~"),
            ("pagedown", "\x1b[6~"),
            ("insert", "\x1b[2~"),
            ("delete", "\x1b[3~"),
            ("f1", "\x1bOP"),
            ("f2", "\x1bOQ"),
            ("f3", "\x1bOR"),
            ("f4", "\x1bOS"),
            ("f5", "\x1b[15~"),
            ("f6", "\x1b[17~"),
            ("f7", "\x1b[18~"),
            ("f8", "\x1b[19~"),
            ("f9", "\x1b[20~"),
            ("f10", "\x1b[21~"),
            ("f11", "\x1b[23~"),
            ("f12", "\x1b[24~"),
            ("space", " "),
        ] {
            assert_eq!(
                input::encode(&key(name, None, Modifiers::default()), modes).as_deref(),
                Some(expected.as_bytes()),
                "{name}, application={application}"
            );
        }
        for (name, suffix) in [
            ("up", 'A'),
            ("down", 'B'),
            ("right", 'C'),
            ("left", 'D'),
            ("home", 'H'),
            ("end", 'F'),
        ] {
            let expected = format!("\x1b{}{suffix}", if application { 'O' } else { '[' });
            assert_eq!(
                input::encode(&key(name, None, Modifiers::default()), modes).as_deref(),
                Some(expected.as_bytes())
            );
        }
    }
}

#[test]
fn printable_text_preserves_unicode_case_and_multiple_codepoints() {
    for text in ["a", "A", "é", "界", "e\u{301}", "👩‍💻", " "] {
        assert_eq!(
            input::encode(
                &key("a", Some(text), Modifiers::default()),
                Modes::default()
            )
            .as_deref(),
            Some(text.as_bytes())
        );
    }
    for text in [None, Some(""), Some("\n"), Some("\x1b")] {
        assert_eq!(
            input::encode(
                &key("unknown", text, Modifiers::default()),
                Modes::default()
            ),
            None
        );
    }
}

#[test]
fn control_letters_and_symbols_produce_control_bytes() {
    let modifiers = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    for letter in b'a'..=b'z' {
        for name in [
            char::from(letter).to_string(),
            char::from(letter).to_uppercase().to_string(),
        ] {
            assert_eq!(
                input::encode(&key(&name, None, modifiers), Modes::default()),
                Some(vec![letter - b'a' + 1])
            );
        }
    }
    for (name, expected) in [
        ("space", 0),
        ("@", 0),
        ("2", 0),
        ("[", 27),
        ("3", 27),
        ("\\", 28),
        ("4", 28),
        ("]", 29),
        ("5", 29),
        ("^", 30),
        ("6", 30),
        ("_", 31),
        ("-", 31),
        ("7", 31),
        ("/", 31),
        ("?", 127),
        ("8", 127),
    ] {
        assert_eq!(
            input::encode(&key(name, None, modifiers), Modes::default()),
            Some(vec![expected]),
            "{name}"
        );
    }
    assert_eq!(
        input::encode(&key("9", None, modifiers), Modes::default()),
        None
    );
}

#[test]
fn alt_prefixes_the_base_key_and_control_bytes() {
    for (keystroke, expected) in [
        (
            key(
                "f",
                Some("ƒ"),
                Modifiers {
                    alt: true,
                    ..Modifiers::default()
                },
            ),
            b"\x1bf".as_slice(),
        ),
        (
            key(
                "f",
                Some("Ï"),
                Modifiers {
                    alt: true,
                    shift: true,
                    ..Modifiers::default()
                },
            ),
            b"\x1bF".as_slice(),
        ),
        (
            key(
                "c",
                None,
                Modifiers {
                    alt: true,
                    control: true,
                    ..Modifiers::default()
                },
            ),
            b"\x1b\x03".as_slice(),
        ),
        (
            key(
                "left",
                None,
                Modifiers {
                    alt: true,
                    ..Modifiers::default()
                },
            ),
            b"\x1b\x1b[D".as_slice(),
        ),
        (
            key(
                "tab",
                None,
                Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
            ),
            b"\x1b[Z".as_slice(),
        ),
    ] {
        assert_eq!(
            input::encode(&keystroke, Modes::default()).as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn command_combinations_never_reach_the_terminal() {
    for name in ["t", "w", "[", "]", "q", "c", "v", "enter", "up"] {
        for control in [false, true] {
            for alt in [false, true] {
                let modifiers = Modifiers {
                    platform: true,
                    control,
                    alt,
                    ..Modifiers::default()
                };
                assert_eq!(
                    input::encode(&key(name, Some(name), modifiers), Modes::default()),
                    None
                );
            }
        }
    }
}
