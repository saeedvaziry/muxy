use std::env;
use std::error::Error;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;

use muxy_client::{Client, ClientEvent, RunGrid};
use muxy_protocol::{SessionId, Size};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
const SIZE: Size = Size { cols: 80, rows: 24 };

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "attach: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [mode, socket] if mode == "list" => list(&Client::connect(Path::new(socket))?),
        [mode, socket] if mode == "new" => new(&Client::connect(Path::new(socket))?, None),
        [mode, socket, directory] if mode == "new" => {
            new(&Client::connect(Path::new(socket))?, Some(PathBuf::from(directory)))
        }
        [mode, socket, id] if mode == "attach" => {
            attach(&Client::connect(Path::new(socket))?, session_id(id)?)
        }
        [mode, socket, id] if mode == "end" => {
            end(&Client::connect(Path::new(socket))?, session_id(id)?)
        }
        _ => Err("usage: attach list <socket> | new <socket> [dir] | attach <socket> <id> | end <socket> <id>".into()),
    }
}

fn session_id(value: &std::ffi::OsStr) -> Result<SessionId> {
    let value = value.to_str().ok_or("session ID is not valid UTF-8")?;
    SessionId::new(value.parse()?).ok_or_else(|| "session ID must be non-zero".into())
}

fn list(client: &Client) -> Result {
    let mut stdout = io::stdout().lock();
    for session in client.list_sessions()? {
        writeln!(
            stdout,
            "{}\t{}",
            session.id.get(),
            String::from_utf8_lossy(&session.directory.0)
        )?;
    }
    Ok(())
}

fn new(client: &Client, directory: Option<PathBuf>) -> Result {
    let directory = match directory {
        Some(directory) => directory,
        None => env::current_dir()?,
    };
    let info = client.create_session(&directory, SIZE)?;
    writeln!(io::stdout(), "{}", info.id.get())?;
    Ok(())
}

fn attach(client: &Client, session: SessionId) -> Result {
    let events = client.events().ok_or("event stream already taken")?;
    let mut attachment = client.attach(session, SIZE)?;
    let channel = attachment.channel;
    writeln!(
        io::stdout(),
        "metadata: title={:?} directory={:?} process={:?}",
        attachment.title,
        String::from_utf8_lossy(&attachment.directory.0),
        attachment.process
    )?;
    redraw(&attachment.grid)?;
    let input = client.clone();
    thread::spawn(move || {
        if let Err(error) = forward_stdin(&input, channel) {
            let _ = writeln!(io::stderr(), "stdin: {error}");
        }
    });
    for event in events {
        match event {
            ClientEvent::Frame {
                channel: received,
                frame,
            } if received == channel => {
                attachment.grid.apply(&frame);
                redraw(&attachment.grid)?;
                client.ack(channel, frame.seq)?;
            }
            ClientEvent::Metadata {
                channel: received,
                event,
            } if received == channel => {
                writeln!(io::stdout(), "metadata: {event:?}")?;
            }
            ClientEvent::Frame { .. } | ClientEvent::Metadata { .. } => {}
            ClientEvent::SessionEnded {
                session: ended,
                reason,
            } => {
                if ended == session {
                    writeln!(io::stdout(), "\nsession ended: {reason:?}")?;
                    return Ok(());
                }
            }
            ClientEvent::Disconnected => return Err("disconnected from server".into()),
        }
    }
    Ok(())
}

fn end(client: &Client, session: SessionId) -> Result {
    let events = client.events().ok_or("event stream already taken")?;
    client.end_session(session)?;
    for event in events {
        match event {
            ClientEvent::SessionEnded {
                session: ended,
                reason,
            } if ended == session => {
                writeln!(io::stdout(), "session ended: {reason:?}")?;
                return Ok(());
            }
            ClientEvent::Disconnected => return Err("disconnected from server".into()),
            ClientEvent::Frame { .. }
            | ClientEvent::Metadata { .. }
            | ClientEvent::SessionEnded { .. } => {}
        }
    }
    Err("event stream closed before the session ended".into())
}

fn forward_stdin(client: &Client, channel: muxy_protocol::ChannelId) -> Result {
    let mut stdin = io::stdin().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        if stdin.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        client.send_input(channel, &line)?;
    }
}

fn redraw(grid: &RunGrid) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(b"\x1b[2J\x1b[H")?;
    for index in 0..grid.rows.len() {
        writeln!(stdout, "{}", grid.row_text(index))?;
    }
    stdout.flush()
}
