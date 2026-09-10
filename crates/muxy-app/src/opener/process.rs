use std::io;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub(super) fn run(command: &mut Command, timeout: Duration) -> io::Result<Child> {
    let mut child = command.stdin(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        let result = match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(child),
            Ok(Some(status)) => {
                return Err(io::Error::other(format!("Opener exited with {status}")));
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(
                    Duration::from_millis(10)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
                continue;
            }
            Ok(None) => io::Error::new(io::ErrorKind::TimedOut, "Opener timed out"),
            Err(error) => error,
        };
        let _ = child.kill();
        let _ = child.wait();
        return Err(result);
    }
}
