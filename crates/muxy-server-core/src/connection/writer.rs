use std::io::Write;

use muxy_transport::StreamCancellation;
use muxy_wire::{Encoder, WireError};

use super::outbox::Outbox;

pub(super) fn run(
    mut encoder: Encoder<impl Write>,
    outbox: &Outbox,
    cancellation: &dyn StreamCancellation,
) -> Result<(), WireError> {
    while let Some((channel, message)) = outbox.next() {
        if let Err(error) = encoder.send(channel, &message) {
            let closed = outbox.is_closed();
            outbox.close();
            cancellation.cancel();
            return if closed { Ok(()) } else { Err(error) };
        }
    }
    Ok(())
}
