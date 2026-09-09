use std::error::Error;
use std::fs;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use muxy_client::{Client, ClientEvent};
use muxy_protocol::{HistoryCursor, SearchSource, SessionId, Size};
use muxy_server_core::{Registry, ServerSettings, connection};

type TestResult = Result<(), Box<dyn Error>>;

struct Fixture {
    root: PathBuf,
    registry: Arc<Registry>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.registry.shutdown();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.registry.is_stopped() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn stalled_saved_read_does_not_delay_input_or_ping_on_the_same_connection() -> TestResult {
    let root = std::env::temp_dir().join(format!("muxy-search-stall-{}", std::process::id()));
    fs::create_dir_all(&root)?;
    let (send, events) = mpsc::channel();
    let registry = Arc::new(Registry::persistent(
        ServerSettings {
            default_shell: Some("/bin/sh".into()),
            ..ServerSettings::default()
        },
        send,
        &root.join("sessions"),
    )?);
    let fixture = Fixture {
        root,
        registry: Arc::clone(&registry),
    };
    let saved = SessionId::new(1).ok_or("zero ID")?;
    let pipe = fixture.root.join("sessions/0000000000000001.postcard");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()?
            .success()
    );
    let (client, server) = UnixStream::pair()?;
    let serving = thread::spawn(move || connection::serve(Box::new(server), registry, events));
    let client = Client::from_stream(Box::new(client))?;
    let size = Size { cols: 80, rows: 24 };
    let session = client.create_session(&fixture.root, size)?;
    let attachment = client.attach(session.id, size)?;
    let events = client.events().ok_or("events already taken")?;
    let (opened, ready) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let writer = thread::spawn(move || -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new().write(true).open(pipe)?;
        let _ = opened.send(());
        let _ = resume.recv_timeout(Duration::from_secs(5));
        file.write_all(&[255])
    });
    let searching = client.clone();
    let search = thread::spawn(move || {
        searching.search(
            SearchSource::Saved(saved),
            "a",
            false,
            HistoryCursor(0),
            500,
        )
    });
    let result = (|| -> TestResult {
        ready.recv_timeout(Duration::from_secs(3))?;
        client
            .clone()
            .with_timeout(Duration::from_millis(500))
            .ping()?;
        client.send_input(
            attachment.channel,
            b"stty -echo; printf '\\033[2J\\033[HINPUT_REACHED_SHELL'\n",
        )?;
        loop {
            if let ClientEvent::Frame { channel, frame } =
                events.recv_timeout(Duration::from_secs(2))?
            {
                client.ack(channel, frame.seq)?;
                if frame
                    .rows
                    .iter()
                    .flat_map(|row| &row.runs)
                    .any(|run| run.text.starts_with("INPUT_REACHED_SHELL"))
                {
                    break;
                }
            }
        }
        Ok(())
    })();
    let _ = release.send(());
    writer.join().map_err(|_| "pipe writer panicked")??;
    assert!(search.join().map_err(|_| "search panicked")?.is_err());
    client.discard_session(session.id)?;
    drop(client);
    serving.join().map_err(|_| "connection panicked")??;
    result
}
