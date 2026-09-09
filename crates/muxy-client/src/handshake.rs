use muxy_protocol::{CONTROL, ChannelId, Message, SUPPORTED, Version, validate_versions};
use muxy_wire::WireError;

use crate::ClientError;

pub(crate) fn hello() -> Message {
    Message::Hello {
        versions: SUPPORTED.to_vec(),
    }
}

pub(crate) fn accept(
    received: Result<(ChannelId, Message), WireError>,
) -> Result<Version, ClientError> {
    match received {
        Ok((CONTROL, Message::HelloReply { versions })) => {
            validate_versions(&versions).map_err(ClientError::Invalid)?;
            versions
                .into_iter()
                .filter(|version| SUPPORTED.contains(version))
                .max()
                .ok_or(ClientError::VersionUnsupported)
        }
        Ok((CONTROL, Message::VersionUnsupported)) => Err(ClientError::VersionUnsupported),
        Ok((CONTROL, Message::Fatal(error))) => Err(ClientError::Protocol(error.message)),
        Ok((channel, message)) => Err(ClientError::Protocol(format!(
            "expected HelloReply on control, got {message:?} on channel {}",
            channel.0
        ))),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use muxy_protocol::{ErrorCode, ErrorReply};

    use super::*;

    #[test]
    fn accepts_a_common_version_and_rejects_everything_else() {
        assert!(
            matches!(accept(Ok((CONTROL, Message::HelloReply { versions: vec![Version(99), muxy_protocol::V1] }))), Ok(version) if version == muxy_protocol::V1)
        );
        assert!(
            accept(Ok((
                CONTROL,
                Message::HelloReply {
                    versions: vec![Version(u16::MAX), muxy_protocol::V1]
                }
            )))
            .is_ok()
        );
        assert!(matches!(
            accept(Ok((
                CONTROL,
                Message::HelloReply {
                    versions: vec![Version(u16::MAX)]
                }
            ))),
            Err(ClientError::VersionUnsupported)
        ));
        assert!(matches!(
            accept(Ok((CONTROL, Message::HelloReply { versions: vec![] }))),
            Err(ClientError::Invalid(ErrorCode::BadRequest))
        ));
        assert!(matches!(
            accept(Ok((CONTROL, Message::VersionUnsupported))),
            Err(ClientError::VersionUnsupported)
        ));
        assert!(matches!(
            accept(Ok((CONTROL, Message::Fatal(ErrorReply { code: ErrorCode::BadRequest, message: "bad".into() })))),
            Err(ClientError::Protocol(message)) if message == "bad"
        ));
        assert!(matches!(
            accept(Ok((ChannelId(1), Message::VersionUnsupported))),
            Err(ClientError::Protocol(_))
        ));
        assert!(matches!(
            accept(Err(WireError::Closed)),
            Err(ClientError::Disconnected)
        ));
    }
}
