use muxy_protocol::Message;

use crate::WireError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MessageKind {
    Hello = 1,
    Request = 2,
    FrameAck = 3,
    HelloReply = 4,
    VersionUnsupported = 5,
    Reply = 6,
    SessionEnded = 7,
    Fatal = 8,
    Input = 9,
    Frame = 10,
    Metadata = 11,
    Mouse = 12,
}

impl MessageKind {
    pub fn from_u8(value: u8) -> Result<Self, WireError> {
        if value & 0xc0 != 0 {
            return Err(WireError::FlagsSet(value));
        }
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::Request),
            3 => Ok(Self::FrameAck),
            4 => Ok(Self::HelloReply),
            5 => Ok(Self::VersionUnsupported),
            6 => Ok(Self::Reply),
            7 => Ok(Self::SessionEnded),
            8 => Ok(Self::Fatal),
            9 => Ok(Self::Input),
            10 => Ok(Self::Frame),
            11 => Ok(Self::Metadata),
            12 => Ok(Self::Mouse),
            _ => Err(WireError::UnknownKind(value)),
        }
    }
}

impl From<&Message> for MessageKind {
    fn from(message: &Message) -> Self {
        match message {
            Message::Hello { .. } => Self::Hello,
            Message::Request { .. } => Self::Request,
            Message::FrameAck { .. } => Self::FrameAck,
            Message::HelloReply { .. } => Self::HelloReply,
            Message::VersionUnsupported => Self::VersionUnsupported,
            Message::Reply { .. } => Self::Reply,
            Message::SessionEnded { .. } => Self::SessionEnded,
            Message::Fatal(_) => Self::Fatal,
            Message::Input(_) => Self::Input,
            Message::Frame(_) => Self::Frame,
            Message::Metadata(_) => Self::Metadata,
            Message::Mouse(_) => Self::Mouse,
        }
    }
}
