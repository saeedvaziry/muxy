use std::env;
use std::error::Error;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::ExitCode;
use std::thread;

use muxy_transport::{ByteStream, Listener, UnixSocketListener, connect};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "echo: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [mode, path] if mode == "listen" => {
            let listener = UnixSocketListener::bind(Path::new(path))?;
            writeln!(io::stdout(), "listening on {}", Path::new(path).display())?;
            loop {
                let stream = listener.accept()?;
                thread::Builder::new().name("echo".into()).spawn(move || {
                    if let Err(error) = echo(stream) {
                        let _ = writeln!(io::stderr(), "echo connection: {error}");
                    }
                })?;
            }
        }
        [mode, path] if mode == "connect" => client(Path::new(path))?,
        _ => return Err("usage: echo listen|connect <path>".into()),
    }
    Ok(())
}

fn echo(stream: Box<dyn ByteStream>) -> io::Result<()> {
    let (reader, mut writer) = stream.split()?;
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        writer.write_all(b"> ")?;
        writer.write_all(&line)?;
        writer.flush()?;
    }
}

fn client(path: &Path) -> io::Result<()> {
    let (reader, mut writer) = connect(path)?.split()?;
    let mut reader = BufReader::new(reader);
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut line = Vec::new();
    let mut reply = Vec::new();
    loop {
        line.clear();
        if stdin.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if !line.ends_with(b"\n") {
            line.push(b'\n');
        }
        writer.write_all(&line)?;
        writer.flush()?;
        reply.clear();
        if reader.read_until(b'\n', &mut reply)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "server closed before replying",
            ));
        }
        stdout.write_all(&reply)?;
        stdout.flush()?;
    }
}
