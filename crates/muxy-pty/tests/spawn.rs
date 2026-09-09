use std::error::Error;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use std::{env, fs, process};

use muxy_pty::{ExitStatus, Pty, PtyEvent, PtySize, SpawnRequest};

type TestResult = Result<(), Box<dyn Error>>;

const TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn printf_delivers_output_then_closed_with_exit_code_zero() -> TestResult {
    let (output, status) = run(shell("printf hello"))?;

    assert_eq!(output, b"hello");
    assert_eq!(status.code, Some(0));
    Ok(())
}

#[test]
fn cat_echoes_written_input_and_closes_after_kill() -> TestResult {
    let mut pty = Pty::spawn(request("/bin/cat", &[]))?;
    let (sender, receiver) = channel();
    let reader = pty.start_reader(sender)?;

    pty.write(b"abc\n")?;
    let echoed = wait_for(&receiver, b"abc")?;
    assert!(text(&echoed).contains("abc"), "{echoed:?}");

    pty.kill()?;
    collect_until_closed(&receiver)?;
    let status = pty.wait()?;
    reader.join().map_err(|_| "reader thread panicked")?;

    assert_eq!(status.signal, Some(1));
    Ok(())
}

#[test]
fn pwd_prints_the_requested_directory() -> TestResult {
    let dir = env::temp_dir().join(format!("muxy-pty-{}", process::id()));
    fs::create_dir_all(&dir)?;
    let dir = fs::canonicalize(&dir)?;
    let mut request = shell("pwd");
    request.cwd.clone_from(&dir);

    let (output, status) = run(request)?;
    fs::remove_dir(&dir)?;

    assert_eq!(text(&output).trim(), dir.to_string_lossy());
    assert_eq!(status.code, Some(0));
    Ok(())
}

#[test]
fn exit_code_is_reported() -> TestResult {
    let (_, status) = run(shell("exit 3"))?;

    assert_eq!(
        status,
        ExitStatus {
            code: Some(3),
            signal: None
        }
    );
    Ok(())
}

#[test]
fn resize_is_visible_to_stty() -> TestResult {
    let mut pty = Pty::spawn(shell("read line; stty size"))?;
    let (sender, receiver) = channel();
    let reader = pty.start_reader(sender)?;

    pty.resize(PtySize {
        cols: 120,
        rows: 40,
    })?;
    pty.write(b"go\n")?;
    let output = collect_until_closed(&receiver)?;
    let status = pty.wait()?;
    reader.join().map_err(|_| "reader thread panicked")?;

    assert!(text(&output).contains("40 120"), "{output:?}");
    assert_eq!(status.code, Some(0));
    Ok(())
}

#[test]
fn env_entries_are_visible_to_the_child() -> TestResult {
    let mut request = shell("echo $MUXY_TEST");
    request
        .env
        .push((OsString::from("MUXY_TEST"), OsString::from("1")));

    let (output, _) = run(request)?;

    assert_eq!(text(&output).trim(), "1");
    Ok(())
}

fn shell(script: &str) -> SpawnRequest {
    request("/bin/sh", &["-c", script])
}

fn request(program: &str, args: &[&str]) -> SpawnRequest {
    SpawnRequest {
        program: PathBuf::from(program),
        args: args.iter().map(OsString::from).collect(),
        cwd: env::temp_dir(),
        env: Vec::new(),
        size: PtySize { cols: 80, rows: 24 },
    }
}

fn run(request: SpawnRequest) -> Result<(Vec<u8>, ExitStatus), Box<dyn Error>> {
    let mut pty = Pty::spawn(request)?;
    let (sender, receiver) = channel();
    let reader = pty.start_reader(sender)?;
    let output = collect_until_closed(&receiver)?;
    let status = pty.wait()?;
    reader.join().map_err(|_| "reader thread panicked")?;
    Ok((output, status))
}

fn collect_until_closed(receiver: &Receiver<PtyEvent>) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut output = Vec::new();
    loop {
        match receiver.recv_timeout(TIMEOUT)? {
            PtyEvent::Output(bytes) => output.extend(bytes),
            PtyEvent::Closed => return Ok(output),
        }
    }
}

fn wait_for(receiver: &Receiver<PtyEvent>, needle: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let deadline = Instant::now() + TIMEOUT;
    let mut output = Vec::new();
    while !contains(&output, needle) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining)? {
            PtyEvent::Output(bytes) => output.extend(bytes),
            PtyEvent::Closed => return Err("pty closed before expected output".into()),
        }
    }
    Ok(output)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
