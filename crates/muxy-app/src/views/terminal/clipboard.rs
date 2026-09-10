use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use muxy_protocol::Modes;

pub(crate) fn paste(text: &str, modes: Modes) -> Vec<u8> {
    let text = text.replace("\r\n", "\n").replace('\n', "\r");
    bracket(text.into_bytes(), modes)
}

pub(crate) fn paths(paths: &[PathBuf], modes: Modes) -> Option<Vec<u8>> {
    let mut text = Vec::new();
    for path in paths {
        let bytes = path.as_os_str().as_bytes();
        if !path.is_absolute() || bytes.contains(&0) {
            return None;
        }
        if !text.is_empty() {
            text.push(b' ');
        }
        let ansi = bytes.iter().any(u8::is_ascii_control);
        if ansi {
            text.push(b'$');
        }
        text.push(b'\'');
        for &byte in bytes {
            match (ansi, byte) {
                (true, byte) if byte.is_ascii_control() => {
                    let hex = b"0123456789abcdef";
                    text.extend_from_slice(&[
                        b'\\',
                        b'x',
                        hex[usize::from(byte >> 4)],
                        hex[usize::from(byte & 15)],
                    ]);
                }
                (true, b'\'' | b'\\') => text.extend_from_slice(&[b'\\', byte]),
                (false, b'\'') => text.extend_from_slice(b"'\\''"),
                _ => text.push(byte),
            }
        }
        text.push(b'\'');
    }
    Some(bracket(text, modes))
}

fn bracket(text: Vec<u8>, modes: Modes) -> Vec<u8> {
    if modes.bracketed_paste && !text.is_empty() {
        [b"\x1b[200~".as_slice(), &text, b"\x1b[201~"].concat()
    } else {
        text
    }
}
