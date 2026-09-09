use serde::{Deserialize, Serialize};

use crate::{
    ChannelId, ErrorReply, ExitReason, MetadataEvent, MouseEvent, ReplyBody, RequestBody,
    RequestId, ScreenFrame, SessionId, Version,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ChannelKind {
    Control,
    Session,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Hello {
        versions: Vec<Version>,
    },
    Request {
        id: RequestId,
        body: RequestBody,
    },
    FrameAck {
        channel: ChannelId,
        seq: u64,
    },
    HelloReply {
        versions: Vec<Version>,
    },
    VersionUnsupported,
    Reply {
        id: RequestId,
        body: ReplyBody,
    },
    SessionEnded {
        session: SessionId,
        reason: ExitReason,
    },
    Fatal(ErrorReply),
    Input(Vec<u8>),
    Frame(ScreenFrame),
    Metadata(MetadataEvent),
    Mouse(MouseEvent),
}

impl Message {
    pub fn channel_kind(&self) -> ChannelKind {
        match self {
            Self::Hello { .. }
            | Self::Request { .. }
            | Self::FrameAck { .. }
            | Self::HelloReply { .. }
            | Self::VersionUnsupported
            | Self::Reply { .. }
            | Self::SessionEnded { .. }
            | Self::Fatal(_) => ChannelKind::Control,
            Self::Input(_) | Self::Frame(_) | Self::Metadata(_) | Self::Mouse(_) => {
                ChannelKind::Session
            }
        }
    }
}
