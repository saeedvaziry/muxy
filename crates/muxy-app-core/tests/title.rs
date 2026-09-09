use muxy_app_core::title::derive;
use muxy_protocol::{ForegroundProcess, ServerPath};

#[test]
fn program_title_then_foreground_process_then_directory() {
    for (title, name, is_shell, directory, expected) in [
        ("hello", "vim", false, b"/tmp".as_slice(), "hello"),
        ("", "vim", false, b"/tmp".as_slice(), "vim"),
        ("", "zsh", true, b"/tmp".as_slice(), "tmp"),
        ("hello", "zsh", true, b"/tmp".as_slice(), "hello"),
        ("  ", "zsh", true, b"/tmp/".as_slice(), "tmp"),
        ("", "", false, b"/Users/me/project".as_slice(), "project"),
        ("", "  ", false, b"/".as_slice(), "/"),
        ("", "zsh", true, b"/tmp/\xff".as_slice(), "�"),
        ("", "zsh", true, b"".as_slice(), "Terminal"),
    ] {
        let process = ForegroundProcess {
            name: name.into(),
            is_shell,
        };
        assert_eq!(
            derive(title, Some(&process), &ServerPath(directory.to_vec())),
            expected
        );
    }
    assert_eq!(derive("", None, &ServerPath(b"/tmp".to_vec())), "tmp");
}
