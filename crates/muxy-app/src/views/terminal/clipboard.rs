use muxy_protocol::Modes;

pub(crate) fn paste(text: &str, modes: Modes) -> Vec<u8> {
    let text = text.replace("\r\n", "\n").replace('\n', "\r");
    if modes.bracketed_paste && !text.is_empty() {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}
