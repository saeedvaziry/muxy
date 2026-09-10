#[path = "../src/views/terminal/clipboard.rs"]
mod clipboard;

use muxy_protocol::Modes;

#[test]
fn paste_preserves_unicode_and_normalizes_lf_crlf_and_cr() {
    assert_eq!(
        clipboard::paste("one\ntwo\r\n三\rfour", Modes::default()),
        "one\rtwo\r三\rfour".as_bytes()
    );
}

#[test]
fn bracketed_paste_wraps_the_whole_payload_once() {
    let modes = Modes {
        bracketed_paste: true,
        ..Modes::default()
    };
    assert_eq!(
        clipboard::paste("  one\n    two\n", modes),
        b"\x1b[200~  one\r    two\r\x1b[201~"
    );
    assert!(clipboard::paste("", modes).is_empty());
    assert!(clipboard::paste("", Modes::default()).is_empty());
}

#[test]
fn file_paths_are_shell_escaped_losslessly_and_bracketed_once() {
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;
    let paths = [PathBuf::from("/tmp/a b"), PathBuf::from("/tmp/it's.txt")];
    assert_eq!(
        clipboard::paths(&paths, Modes::default()),
        Some(b"'/tmp/a b' '/tmp/it'\\''s.txt'".to_vec())
    );
    let modes = Modes {
        bracketed_paste: true,
        ..Modes::default()
    };
    assert_eq!(
        clipboard::paths(&paths, modes),
        Some(b"\x1b[200~'/tmp/a b' '/tmp/it'\\''s.txt'\x1b[201~".to_vec())
    );
    let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/\xff".to_vec()));
    assert_eq!(
        clipboard::paths(&[path], Modes::default()),
        Some(b"'/tmp/\xff'".to_vec())
    );
    assert_eq!(clipboard::paths(&[], modes), Some(Vec::new()));
    for path in ["relative", "/tmp/a\0b"] {
        assert!(clipboard::paths(&[PathBuf::from(path)], modes).is_none());
    }
}

#[test]
fn control_character_paths_round_trip_through_bash_and_zsh_without_terminal_controls()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;
    let paths = [
        PathBuf::from("/tmp/normal file"),
        PathBuf::from("/tmp/a\nb\t'\\a$()\r\x1b[201~\x7f\n"),
        PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/\xff\n".to_vec())),
    ];
    let escaped = clipboard::paths(&paths, Modes::default()).ok_or("paths")?;
    assert!(!escaped.iter().any(u8::is_ascii_control));
    assert!(escaped.windows(4).any(|window| window == b"\\x0a"));
    let expected: Vec<u8> = paths
        .iter()
        .flat_map(|path| path.as_os_str().as_bytes().iter().copied().chain([0]))
        .collect();
    for shell in ["/bin/bash", "/bin/zsh"] {
        let command = [b"printf '%s\\0' ".as_slice(), &escaped].concat();
        let output = std::process::Command::new(shell)
            .args(["-f", "-c"])
            .arg(std::ffi::OsString::from_vec(command))
            .output()?;
        assert!(output.status.success(), "{shell}: {:?}", output.stderr);
        assert_eq!(output.stdout, expected, "{shell}");
    }
    assert_eq!(
        clipboard::paths(
            &paths,
            Modes {
                bracketed_paste: true,
                ..Modes::default()
            }
        ),
        Some([b"\x1b[200~".as_slice(), &escaped, b"\x1b[201~"].concat())
    );
    Ok(())
}
