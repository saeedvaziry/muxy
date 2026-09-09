use std::io::{Read, Write};

use muxy_protocol::{CONTROL, ErrorCode, ErrorReply, Message, SUPPORTED, Version};
use muxy_wire::{Decoder, Encoder, WireError};

pub(super) fn accept(
    decoder: &mut Decoder<impl Read>,
    encoder: &mut Encoder<impl Write>,
) -> Result<Option<Version>, WireError> {
    let message = match decoder.next() {
        Ok((CONTROL, Message::Hello { versions })) if !versions.is_empty() => {
            if let Some(version) = versions
                .into_iter()
                .filter(|version| SUPPORTED.contains(version))
                .max()
            {
                encoder.send(
                    CONTROL,
                    &Message::HelloReply {
                        versions: SUPPORTED.to_vec(),
                    },
                )?;
                return Ok(Some(version));
            }
            Message::VersionUnsupported
        }
        Ok(_) => fatal("expected Hello on control"),
        Err(WireError::Closed) => return Ok(None),
        Err(error @ WireError::Io(_)) => return Err(error),
        Err(error) => fatal(error.to_string()),
    };
    encoder.send(CONTROL, &message)?;
    Ok(None)
}

pub(super) fn fatal(message: impl Into<String>) -> Message {
    let message = message.into();
    log::error!("fatal protocol error: {message}");
    Message::Fatal(ErrorReply {
        code: ErrorCode::BadRequest,
        message,
    })
}
