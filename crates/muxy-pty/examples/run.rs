use std::error::Error;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, MutexGuard};
use std::{env, thread};

use muxy_pty::{ExitStatus, Pty, PtyEvent, PtySize, SpawnRequest};

const END_OF_TRANSMISSION: &[u8] = b"\x04";

fn main() -> ExitCode {
    match run() {
        Ok(status) => exit_code(status),
        Err(error) => {
            let _ = writeln!(io::stderr(), "run: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitStatus, Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let Some(program) = args.next() else {
        return Err("usage: run <program> [args...]".into());
    };
    let request = SpawnRequest {
        program: PathBuf::from(program),
        args: args.collect(),
        cwd: env::current_dir()?,
        env: Vec::new(),
        size: PtySize { cols: 80, rows: 24 },
    };

    let pty = Arc::new(Mutex::new(Pty::spawn(request)?));
    let (sender, receiver) = channel();
    let reader = lock(&pty)?.start_reader(sender)?;

    let input_pty = Arc::clone(&pty);
    thread::spawn(move || forward_stdin(&input_pty));

    let mut stdout = io::stdout().lock();
    for event in receiver {
        match event {
            PtyEvent::Output(bytes) => {
                stdout.write_all(&bytes)?;
                stdout.flush()?;
            }
            PtyEvent::Closed => break,
        }
    }

    let status = lock(&pty)?.wait()?;
    reader.join().map_err(|_| "reader thread panicked")?;
    Ok(status)
}

fn forward_stdin(pty: &Mutex<Pty>) {
    let mut stdin = io::stdin().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        match stdin.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if write(pty, &line).is_err() {
                    return;
                }
            }
        }
    }
    let _ = write(pty, END_OF_TRANSMISSION);
}

fn write(pty: &Mutex<Pty>, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    lock(pty)?.write(bytes)?;
    Ok(())
}

fn lock(pty: &Mutex<Pty>) -> Result<MutexGuard<'_, Pty>, Box<dyn Error>> {
    pty.lock().map_err(|_| "pty lock poisoned".into())
}

fn exit_code(status: ExitStatus) -> ExitCode {
    let code = status
        .code
        .or_else(|| status.signal.map(|signal| 128 + signal))
        .unwrap_or(1);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
