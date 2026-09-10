use std::error::Error;
use std::fs;
use std::path::PathBuf;

use muxy_protocol::{
    CONTROL, ChannelId, ChannelKind, Message, MetadataEvent, ReplyBody, RequestBody, SearchSource,
};
use muxy_wire::{Decoder, MessageKind, WireError, encode, message_version};

#[test]
fn golden_frames_match_byte_for_byte() -> Result<(), Box<dyn Error>> {
    for message in Message::samples() {
        let fixture = fixture_path(&message);
        let expected = fs::read(&fixture)?;
        let mut actual = Vec::new();
        encode(&message, channel(&message), &mut actual)?;
        assert_eq!(actual, expected, "{}", fixture.display());
        let mut decoder = Decoder::new(expected.as_slice());
        assert_eq!(decoder.next()?, (channel(&message), message));
    }
    Ok(())
}

#[test]
#[ignore = "regenerates the mutable development schema fixtures"]
fn generate_fixtures() -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"))?;
    for message in Message::samples() {
        let mut bytes = Vec::new();
        encode(&message, channel(&message), &mut bytes)?;
        let path = fixture_path(&message);
        fs::write(path, bytes)?;
    }
    Ok(())
}

fn fixture_path(message: &Message) -> PathBuf {
    let name = match message {
        Message::Request {
            body: RequestBody::SetTerminalColors(_),
            ..
        } => "terminal_colors_request",
        Message::Reply {
            body: ReplyBody::TerminalColorsSet,
            ..
        } => "terminal_colors_reply",
        Message::Request {
            body:
                RequestBody::Search {
                    source: SearchSource::Live(_),
                    ..
                },
            ..
        } => "search_request",
        Message::Request {
            body:
                RequestBody::Search {
                    source: SearchSource::Saved(_),
                    ..
                },
            ..
        } => "saved_search_request",
        Message::Reply {
            body: ReplyBody::SearchPage(_),
            ..
        } => "search_reply",
        Message::Metadata(MetadataEvent::ScreenPrompts { .. }) => "screen_prompts",
        Message::Metadata(MetadataEvent::Links { .. }) => "links_metadata",
        Message::Metadata(MetadataEvent::InputModes(_)) => "input_modes",
        Message::Metadata(MetadataEvent::CursorBlinking(_)) => "cursor_blinking",
        Message::Metadata(MetadataEvent::History { .. }) => "history_metadata",
        Message::Request {
            body: RequestBody::HistoryPage { .. },
            ..
        } => "history_request",
        Message::Request {
            body: RequestBody::SavedHistoryPage { .. },
            ..
        } => "saved_history_request",
        Message::Reply {
            body: ReplyBody::HistoryPage(_),
            ..
        } => "history_reply",
        Message::Reply {
            body: ReplyBody::Attached { snapshot, .. },
            ..
        } if !snapshot.history.is_empty() => "attached_history_reply",
        _ => match MessageKind::from(message) {
            MessageKind::Hello => "hello",
            MessageKind::Request => "request",
            MessageKind::FrameAck => "frame_ack",
            MessageKind::HelloReply => "hello_reply",
            MessageKind::VersionUnsupported => "version_unsupported",
            MessageKind::Reply => "reply",
            MessageKind::SessionEnded => "session_ended",
            MessageKind::Fatal => "fatal",
            MessageKind::Input => "input",
            MessageKind::Frame => "frame",
            MessageKind::Metadata => "metadata",
            MessageKind::Mouse => "mouse",
        },
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.bin"))
}

fn channel(message: &Message) -> ChannelId {
    match message.channel_kind() {
        ChannelKind::Control => CONTROL,
        ChannelKind::Session => ChannelId(1),
    }
}

#[test]
fn development_messages_share_one_version_and_reject_unknown_schemas() -> Result<(), Box<dyn Error>>
{
    for message in Message::samples() {
        let mut bytes = Vec::new();
        encode(&message, channel(&message), &mut bytes)?;
        assert_eq!(message_version(&message), muxy_protocol::V1);
        assert_eq!(bytes[4..6], 1_u16.to_le_bytes());
        for version in [0_u16, 2, 9, u16::MAX] {
            bytes[4..6].copy_from_slice(&version.to_le_bytes());
            assert!(matches!(
                Decoder::new(bytes.as_slice()).next(),
                Err(WireError::UnsupportedVersion(_))
            ));
        }
    }
    Ok(())
}
