use muxy_protocol::{ChannelId, Message, V1, Version};
use serde::{Serialize, de::DeserializeOwned};

use crate::{HEADER_LEN, Header, MessageKind, WireError};

pub fn encode(
    message: &Message,
    channel: ChannelId,
    output: &mut Vec<u8>,
) -> Result<(), WireError> {
    output.clear();
    if let Message::Input(bytes) = message {
        Header::new(bytes.len(), channel, MessageKind::Input)?;
    }
    output.resize(HEADER_LEN, 0);
    match message {
        Message::Hello { versions } | Message::HelloReply { versions } => {
            serialize(versions, output)?;
        }
        Message::Request { id, body } => serialize(&(id, body), output)?,
        Message::FrameAck { channel, seq } => serialize(&(channel, seq), output)?,
        Message::VersionUnsupported => serialize(&(), output)?,
        Message::Reply { id, body } => serialize(&(id, body), output)?,
        Message::SessionEnded { session, reason } => serialize(&(session, reason), output)?,
        Message::Fatal(error) => serialize(error, output)?,
        Message::Input(bytes) => output.extend_from_slice(bytes),
        Message::Frame(frame) => serialize(frame, output)?,
        Message::Metadata(event) => serialize(event, output)?,
        Message::Mouse(event) => serialize(event, output)?,
    }
    let mut header = Header::new(
        output.len() - HEADER_LEN,
        channel,
        MessageKind::from(message),
    )?;
    header.version = message_version(message).0;
    output[..HEADER_LEN].copy_from_slice(&header.to_bytes());
    Ok(())
}

pub fn decode(header: Header, payload: &[u8]) -> Result<(ChannelId, Message), WireError> {
    header.validate()?;
    if payload.len() != header.payload_len()? {
        return Err(postcard::Error::DeserializeBadEncoding.into());
    }
    let message = match MessageKind::from_u8(header.kind)? {
        MessageKind::Hello => Message::Hello {
            versions: deserialize(payload)?,
        },
        MessageKind::Request => {
            let (id, body) = deserialize(payload)?;
            Message::Request { id, body }
        }
        MessageKind::FrameAck => {
            let (channel, seq) = deserialize(payload)?;
            Message::FrameAck { channel, seq }
        }
        MessageKind::HelloReply => Message::HelloReply {
            versions: deserialize(payload)?,
        },
        MessageKind::VersionUnsupported => {
            deserialize::<()>(payload)?;
            Message::VersionUnsupported
        }
        MessageKind::Reply => {
            let (id, body) = deserialize(payload)?;
            Message::Reply { id, body }
        }
        MessageKind::SessionEnded => {
            let (session, reason) = deserialize(payload)?;
            Message::SessionEnded { session, reason }
        }
        MessageKind::Fatal => Message::Fatal(deserialize(payload)?),
        MessageKind::Input => Message::Input(payload.to_vec()),
        MessageKind::Frame => Message::Frame(deserialize(payload)?),
        MessageKind::Metadata => Message::Metadata(deserialize(payload)?),
        MessageKind::Mouse => Message::Mouse(deserialize(payload)?),
    };
    if message_version(&message).0 > header.version {
        return Err(postcard::Error::DeserializeBadEncoding.into());
    }
    Ok((ChannelId(header.channel), message))
}

fn serialize<T: Serialize>(value: &T, output: &mut Vec<u8>) -> Result<(), WireError> {
    postcard::to_io(value, output)?;
    Ok(())
}

fn deserialize<T: DeserializeOwned>(payload: &[u8]) -> Result<T, WireError> {
    let (value, trailing) = postcard::take_from_bytes(payload)?;
    if !trailing.is_empty() {
        return Err(postcard::Error::DeserializeBadEncoding.into());
    }
    Ok(value)
}

pub fn message_version(_: &Message) -> Version {
    V1
}
