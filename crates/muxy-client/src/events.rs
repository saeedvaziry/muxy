use std::io::Read;
use std::sync::mpsc::Sender;

use muxy_protocol::{
    CONTROL, ChannelId, ExitReason, Message, MetadataEvent, ScreenFrame, SessionId, Version,
};
use muxy_transport::StreamCancellation;
use muxy_wire::{Decoder, message_version};

use crate::ClientError;
use crate::handshake;
use crate::requests::Pending;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientEvent {
    Frame {
        channel: ChannelId,
        frame: ScreenFrame,
    },
    Metadata {
        channel: ChannelId,
        event: MetadataEvent,
    },
    SessionEnded {
        session: SessionId,
        reason: ExitReason,
    },
    Disconnected,
}

pub(crate) fn route(
    decoder: &mut Decoder<impl Read>,
    pending: &Pending,
    events: &Sender<ClientEvent>,
    connected: &Sender<Result<Version, ClientError>>,
    cancellation: &dyn StreamCancellation,
) {
    let handshake = handshake::accept(decoder.next());
    let accepted = handshake.as_ref().ok().copied();
    let _ = connected.send(handshake);
    if let Some(version) = accepted {
        while let Some(event) = next_event(decoder, pending, version) {
            if events.send(event).is_err() {
                break;
            }
        }
    }
    pending.close();
    cancellation.cancel();
    let _ = events.send(ClientEvent::Disconnected);
}

fn next_event(
    decoder: &mut Decoder<impl Read>,
    pending: &Pending,
    version: Version,
) -> Option<ClientEvent> {
    loop {
        let (channel, message) = decoder.next().ok()?;
        if message.validate().is_err() || message_version(&message) > version {
            return None;
        }
        match (channel, message) {
            (CONTROL, Message::Reply { id, body }) => pending.resolve(id, body),
            (CONTROL, Message::SessionEnded { session, reason }) => {
                return Some(ClientEvent::SessionEnded { session, reason });
            }
            (channel, Message::Frame(frame)) if channel != CONTROL => {
                return Some(ClientEvent::Frame { channel, frame });
            }
            (channel, Message::Metadata(event)) if channel != CONTROL => {
                return Some(ClientEvent::Metadata { channel, event });
            }
            _ => return None,
        }
    }
}
