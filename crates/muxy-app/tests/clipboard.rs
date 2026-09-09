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
