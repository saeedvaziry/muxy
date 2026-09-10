use std::error::Error;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use muxy_client::{Client, ClientEvent};
use muxy_protocol::{HistoryCursor, MetadataEvent, Size};
use muxy_server_core::{Registry, ServerSettings, connection};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn live_prompt_marks_follow_frames_attach_and_every_history_page() -> TestResult {
    let (send, events) = mpsc::channel();
    let registry = Arc::new(Registry::new(
        ServerSettings {
            default_shell: Some("/bin/sh".into()),
            ..ServerSettings::default()
        },
        send,
    ));
    let (local, remote) = UnixStream::pair()?;
    let sessions = Arc::clone(&registry);
    let serving = thread::spawn(move || connection::serve(Box::new(remote), sessions, events));
    let client = Client::from_stream(Box::new(local))?;
    let result = (|| -> TestResult {
        let size = Size { cols: 80, rows: 24 };
        let session = client.create_session(&std::env::temp_dir(), size)?;
        let mut attachment = client.attach(session.id, size)?;
        let events = client.events().ok_or("missing events")?;
        client.send_input(attachment.channel, b"stty -echo; PS1=''; printf '\\033[2J\\033[H'; i=0; while [ $i -lt 3000 ]; do printf '\\033]133;A\\007$ echo hi\\033]133;B\\007\\r\\n\\033]133;C\\007hi\\r\\n\\033]133;D;0\\007'; i=$((i+1)); done; printf '\\033]133;A\\007final-done> \\033]133;B\\007'\n")?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|error| format!("{error}: grid={:?}", attachment.grid))?
            {
                ClientEvent::Frame { channel, frame } => {
                    client.ack(channel, frame.seq)?;
                    attachment.grid.apply(&frame);
                }
                ClientEvent::Metadata {
                    event: MetadataEvent::ScreenPrompts { seq, rows },
                    ..
                } => attachment.grid.screen_prompts(seq, rows),
                _ => {}
            }
            if attachment.grid.rows.iter().enumerate().any(|(row, _)| {
                attachment.grid.row_text(row).starts_with("final-done>")
                    && attachment
                        .grid
                        .prompts
                        .contains(&(attachment.grid.history.len() + row))
            }) {
                break;
            }
        }
        let second = client.attach(session.id, size)?;
        assert!(!second.grid.prompts.is_empty());
        let mut before = HistoryCursor(0);
        let mut marks = 0;
        let mut pages = 0;
        loop {
            let page = client.history_page(second.channel, before, 500)?;
            for index in &page.prompts {
                let index = usize::from(*index);
                let row = if index < page.rows.len() {
                    &page.rows[index]
                } else {
                    &page.screen.as_ref().ok_or("missing prompt screen")?.rows
                        [index - page.rows.len()]
                };
                let text: String = row.runs.iter().map(|run| run.text.as_str()).collect();
                assert!(
                    text.starts_with("$ echo hi") || text.starts_with("final-done>"),
                    "{text:?}"
                );
            }
            marks += page.prompts.len();
            pages += 1;
            let Some(next) = page.next else {
                break;
            };
            before = next;
        }
        assert_eq!(marks, 3001);
        assert!(pages > 1);
        client.end_session(session.id)?;
        assert!(
            client
                .saved_history_page(session.id, HistoryCursor(0), 200)?
                .prompts
                .is_empty()
        );
        client.discard_session(session.id)?;
        Ok(())
    })();
    client.disconnect();
    registry.shutdown();
    serving.join().map_err(|_| "connection panicked")??;
    result
}
