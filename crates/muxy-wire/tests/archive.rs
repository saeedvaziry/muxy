use std::error::Error;

use muxy_protocol::{
    CONTROL, Cursor, ExitReason, Message, ReplyBody, RequestBody, RequestId, Row, SavedScreen,
    SessionId, Size,
};
use muxy_wire::{Decoder, Encoder};

#[test]
fn saved_screen_and_discard_messages_round_trip_without_a_session_channel()
-> Result<(), Box<dyn Error>> {
    let session = SessionId::new(u64::MAX).ok_or("zero ID")?;
    let messages = [
        Message::Request {
            id: RequestId(1),
            body: RequestBody::ReadSavedScreen(session),
        },
        Message::Request {
            id: RequestId(2),
            body: RequestBody::DiscardSession(session),
        },
        Message::Reply {
            id: RequestId(1),
            body: ReplyBody::SavedScreen(SavedScreen {
                size: Size { cols: 1, rows: 1 },
                rows: vec![Row {
                    index: 0,
                    runs: Vec::new(),
                }],
                cursor: Cursor {
                    row: 0,
                    col: 0,
                    visible: false,
                },
                reason: Some(ExitReason::ServerStopped),
            }),
        },
        Message::Reply {
            id: RequestId(2),
            body: ReplyBody::SessionDiscarded,
        },
    ];
    let mut bytes = Vec::new();
    for message in &messages {
        assert_eq!(message.validate(), Ok(()));
        Encoder::new(&mut bytes).send(CONTROL, message)?;
    }
    let mut decoder = Decoder::new(bytes.as_slice());
    for message in messages {
        assert_eq!(decoder.next()?, (CONTROL, message));
    }
    Ok(())
}

#[test]
fn saved_content_uses_the_same_development_schema_as_live_content() -> Result<(), Box<dyn Error>> {
    use muxy_protocol::V1;
    use muxy_wire::{HEADER_LEN, Header, decode, encode};

    let session = SessionId::new(42).ok_or("zero ID")?;
    let mut bytes = Vec::new();
    for body in [
        RequestBody::ReadSavedScreen(session),
        RequestBody::DiscardSession(session),
    ] {
        let message = Message::Request {
            id: RequestId(1),
            body,
        };
        encode(&message, CONTROL, &mut bytes)?;
        let header = Header::from_bytes(bytes[..HEADER_LEN].try_into()?)?;
        assert_eq!(header.version, V1.0);
        assert_eq!(decode(header, &bytes[HEADER_LEN..])?, (CONTROL, message));
        assert!(
            decode(
                Header {
                    version: 0,
                    ..header
                },
                &bytes[HEADER_LEN..]
            )
            .is_err()
        );
    }
    encode(
        &Message::Request {
            id: RequestId(2),
            body: RequestBody::EndSession(session),
        },
        CONTROL,
        &mut bytes,
    )?;
    assert_eq!(
        Header::from_bytes(bytes[..HEADER_LEN].try_into()?)?.version,
        V1.0
    );
    Ok(())
}
