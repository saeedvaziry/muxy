mod handshake;
mod merge;
mod outbox;
mod reader;
mod writer;

use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use muxy_transport::{ByteStream, StreamCancellation};
use muxy_wire::{Decoder, Encoder, WireError};

use crate::{Registry, ServerEvent};
use outbox::Outbox;

const POLL: Duration = Duration::from_millis(50);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

pub fn serve(
    stream: Box<dyn ByteStream>,
    registry: Arc<Registry>,
    events: Receiver<ServerEvent>,
) -> Result<(), WireError> {
    let cancellation: Arc<dyn StreamCancellation> = Arc::from(stream.cancellation()?);
    let (read, write) = stream.split()?;
    let mut decoder = Decoder::new(read);
    let mut encoder = Encoder::new(write);
    let Some(version) = handshake::accept(&mut decoder, &mut encoder)? else {
        return Ok(());
    };
    let outbox = Arc::new(Outbox::new(version));
    let output = Arc::clone(&outbox);
    let cancel = Arc::clone(&cancellation);
    let (done, finished) = mpsc::channel();
    let writer = thread::Builder::new()
        .name("connection-writer".into())
        .spawn(move || {
            let result = writer::run(encoder, &output, cancel.as_ref());
            let _ = done.send(());
            cancel.cancel();
            result
        })?;
    let output = Arc::clone(&outbox);
    let forward = match thread::Builder::new()
        .name("connection-events".into())
        .spawn(move || {
            while !output.is_closed() {
                match events.recv_timeout(POLL) {
                    Ok(ServerEvent::SessionEnded { id, reason }) => {
                        output.session_ended(id, reason);
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        output.close();
                        break;
                    }
                }
            }
        }) {
        Ok(forward) => forward,
        Err(error) => {
            outbox.close();
            cancellation.cancel();
            let _ = writer.join();
            return Err(error.into());
        }
    };
    let result = reader::run(&mut decoder, &registry, &outbox, version);
    drop(registry);
    outbox.close();
    if !matches!(result, Ok(reader::Exit::Fatal))
        || matches!(
            finished.recv_timeout(CLOSE_TIMEOUT),
            Err(RecvTimeoutError::Timeout)
        )
    {
        cancellation.cancel();
    }
    let written = writer
        .join()
        .map_err(|_| io::Error::other("connection writer panicked"))?;
    forward
        .join()
        .map_err(|_| io::Error::other("connection event forwarder panicked"))?;
    written.and(result.map(|_| ()))
}
