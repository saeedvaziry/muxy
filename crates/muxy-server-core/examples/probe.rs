use std::collections::HashMap;
use std::error::Error;
use std::io::{self, BufRead, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::{env, thread};

use muxy_protocol::{
    CONTROL, ChannelId, Message, ReplyBody, RequestBody, RequestId, Row, SUPPORTED, ServerPath,
    SessionId, Size,
};
use muxy_server_core::{Registry, ServerEvent, ServerSettings, connection};
use muxy_transport::{Listener, UnixSocketListener, connect};
use muxy_wire::{Decoder, Encoder};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
type Subscribers = Arc<Mutex<HashMap<u64, Sender<ServerEvent>>>>;
type Output = Arc<Mutex<Encoder<Box<dyn Write + Send>>>>;
const SIZE: Size = Size { cols: 80, rows: 24 };

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "probe: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [mode, socket] if mode == "serve" => serve(Path::new(socket)),
        [mode, socket] if mode == "probe" => probe(Path::new(socket)),
        _ => Err("usage: probe <serve|probe> <socket>".into()),
    }
}

fn serve(path: &Path) -> Result {
    let listener = UnixSocketListener::bind(path)?;
    let (sender, events) = mpsc::channel();
    let registry = Arc::new(Registry::new(ServerSettings::default(), sender));
    let subscribers = Subscribers::default();
    let sinks = Arc::clone(&subscribers);
    thread::spawn(move || {
        for event in events {
            if let Ok(mut sinks) = sinks.lock() {
                sinks.retain(|_, sink| sink.send(event.clone()).is_ok());
            }
        }
    });
    writeln!(io::stdout(), "serving {}", path.display())?;
    let mut next_client = 0_u64;
    loop {
        let stream = listener.accept()?;
        let id = next_client;
        next_client = next_client.checked_add(1).ok_or("client IDs exhausted")?;
        let (sender, events) = mpsc::channel();
        subscribers
            .lock()
            .map_err(|_| "subscriber lock poisoned")?
            .insert(id, sender);
        let registry = Arc::clone(&registry);
        let subscribers = Arc::clone(&subscribers);
        thread::spawn(move || {
            if let Err(error) = connection::serve(stream, registry, events) {
                let _ = writeln!(io::stderr(), "client {id}: {error}");
            }
            if let Ok(mut subscribers) = subscribers.lock() {
                subscribers.remove(&id);
            }
        });
    }
}

fn probe(path: &Path) -> Result {
    let (read, write) = connect(path)?.split()?;
    let mut decoder = Decoder::new(read);
    let mut encoder = Encoder::new(write);
    encoder.send(
        CONTROL,
        &Message::Hello {
            versions: SUPPORTED.to_vec(),
        },
    )?;
    match decoder.next()? {
        (CONTROL, Message::HelloReply { versions }) => {
            writeln!(io::stdout(), "hello: {versions:?}")?;
        }
        other => return Err(format!("handshake rejected: {other:?}").into()),
    }
    match request(&mut decoder, &mut encoder, 1, RequestBody::ListSessions)? {
        ReplyBody::Sessions(sessions) => {
            let ids: Vec<_> = sessions.iter().map(|session| session.id.get()).collect();
            writeln!(io::stdout(), "sessions: {ids:?}")?;
        }
        other => return Err(format!("expected sessions, got {other:?}").into()),
    }
    let info = match request(
        &mut decoder,
        &mut encoder,
        2,
        RequestBody::CreateSession {
            directory: ServerPath(env::current_dir()?.as_os_str().as_bytes().to_vec()),
            size: SIZE,
        },
    )? {
        ReplyBody::SessionCreated(info) => info,
        other => return Err(format!("expected created session, got {other:?}").into()),
    };
    writeln!(io::stdout(), "created: {}", info.id.get())?;
    let snapshot = match request(
        &mut decoder,
        &mut encoder,
        3,
        RequestBody::Attach {
            session: info.id,
            size: SIZE,
        },
    )? {
        ReplyBody::Attached { snapshot, process } => {
            writeln!(
                io::stdout(),
                "process: {process:?}; history rows: {}",
                snapshot.history.len()
            )?;
            *snapshot
        }
        other => return Err(format!("expected snapshot, got {other:?}").into()),
    };
    let channel = snapshot.channel;
    writeln!(
        io::stdout(),
        "snapshot: channel={} size={}x{} cursor={:?} modes={:?}",
        channel.0,
        snapshot.size.cols,
        snapshot.size.rows,
        snapshot.cursor,
        snapshot.modes
    )?;
    print_rows(&snapshot.rows)?;
    writeln!(
        io::stdout(),
        "Type commands; Ctrl-C disconnects without ending the session."
    )?;
    let output = Arc::new(Mutex::new(encoder));
    let input = Arc::clone(&output);
    thread::spawn(move || {
        if let Err(error) = forward_stdin(&input, channel) {
            let _ = writeln!(io::stderr(), "stdin: {error}");
        }
    });
    frames(&mut decoder, &output, channel, info.id)
}

fn frames(
    decoder: &mut Decoder<impl Read>,
    output: &Output,
    channel: ChannelId,
    session_id: SessionId,
) -> Result {
    loop {
        match decoder.next()? {
            (received, Message::Frame(frame)) if received == channel => {
                writeln!(
                    io::stdout(),
                    "frame: channel={} seq={} reset={} cursor={:?} modes={:?}",
                    channel.0,
                    frame.seq,
                    frame.reset,
                    frame.cursor,
                    frame.modes
                )?;
                print_rows(&frame.rows)?;
                output.lock().map_err(|_| "writer lock poisoned")?.send(
                    CONTROL,
                    &Message::FrameAck {
                        channel,
                        seq: frame.seq,
                    },
                )?;
            }
            (received, Message::Metadata(event)) if received == channel => {
                writeln!(io::stdout(), "metadata: {event:?}")?;
            }
            (CONTROL, Message::SessionEnded { session, reason }) => {
                writeln!(
                    io::stdout(),
                    "session ended: {} ({reason:?})",
                    session.get()
                )?;
                if session == session_id {
                    return Ok(());
                }
            }
            (CONTROL, Message::Fatal(error)) => return Err(error.message.into()),
            other => return Err(format!("unexpected message: {other:?}").into()),
        }
    }
}

fn request(
    decoder: &mut Decoder<impl Read>,
    encoder: &mut Encoder<impl Write>,
    id: u32,
    body: RequestBody,
) -> Result<ReplyBody> {
    let id = RequestId(id);
    encoder.send(CONTROL, &Message::Request { id, body })?;
    loop {
        match decoder.next()? {
            (CONTROL, Message::Reply { id: received, body }) if received == id => return Ok(body),
            (CONTROL, Message::SessionEnded { session, reason }) => writeln!(
                io::stdout(),
                "session ended: {} ({reason:?})",
                session.get()
            )?,
            other => return Err(format!("unexpected reply: {other:?}").into()),
        }
    }
}

fn forward_stdin(output: &Output, channel: ChannelId) -> Result {
    let mut stdin = io::stdin().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        if stdin.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        output
            .lock()
            .map_err(|_| "writer lock poisoned")?
            .send(channel, &Message::Input(line.clone()))?;
    }
}

fn print_rows(rows: &[Row]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    for row in rows {
        let text: String = row.runs.iter().map(|run| run.text.as_str()).collect();
        writeln!(stdout, "  row {}: {:?}", row.index, text.trim_end())?;
    }
    stdout.flush()
}
