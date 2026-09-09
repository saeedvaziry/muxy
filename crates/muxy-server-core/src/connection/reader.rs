use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use muxy_core::worker::WorkerPool;

use muxy_protocol::{
    CONTROL, ChannelId, ErrorCode, HistoryCursor, Message, ReplyBody, RequestBody, RequestId,
    SearchSource, SessionId, Size, Version,
};
use muxy_wire::{Decoder, WireError, message_version};

use crate::archive::SearchCache;
use crate::{AttachmentEvent, AttachmentId, Registry, ServerError, SessionCommand};

use super::{POLL, handshake::fatal, outbox::Outbox};

static NEXT_ATTACHMENT: AtomicU64 = AtomicU64::new(1);

pub(super) enum Exit {
    Disconnected,
    Fatal,
}

pub(super) fn run(
    decoder: &mut Decoder<impl Read>,
    registry: &Arc<Registry>,
    outbox: &Arc<Outbox>,
    version: Version,
) -> Result<Exit, WireError> {
    let requests = Requests {
        registry: Arc::clone(registry),
        outbox: Arc::clone(outbox),
        workers: WorkerPool::new("connection-work", 1, 32)?,
        search_cache: Arc::new(Mutex::new(SearchCache::default())),
        version,
        last_channel: Arc::new(AtomicU32::new(0)),
    };
    while !outbox.is_closed() {
        let (channel, message) = match decoder.next() {
            Ok(frame) => frame,
            Err(WireError::Closed) => return Ok(Exit::Disconnected),
            Err(error @ WireError::Io(_)) => return Err(error),
            Err(error) => {
                outbox.close_with(fatal(error.to_string()));
                return Ok(Exit::Fatal);
            }
        };
        if outbox.is_closed() {
            return Ok(Exit::Disconnected);
        }
        let validation = message.validate();
        match (channel, message) {
            (CONTROL, Message::Request { id, body }) => {
                let result = match validation {
                    Ok(()) => requests.route(body, id),
                    Err(code) => Err(ServerError::new(code, "invalid request")),
                };
                let reply = match result {
                    Ok(Some(body)) => body,
                    Ok(None) => continue,
                    Err(error) => ReplyBody::Error(error.to_reply()),
                };
                outbox.push_control(Message::Reply { id, body: reply });
            }
            (CONTROL, Message::FrameAck { channel, seq })
                if channel.0 > 0 && channel.0 <= requests.last_channel.load(Ordering::Acquire) =>
            {
                outbox.ack(channel, seq);
            }
            (channel, Message::Input(bytes))
                if channel.0 > 0 && channel.0 <= requests.last_channel.load(Ordering::Acquire) =>
            {
                if outbox.handle(channel).is_none() {
                    continue;
                }
                if muxy_protocol::validate_input(&bytes).is_err() {
                    outbox.close_with(fatal("input exceeds the limit"));
                    return Ok(Exit::Fatal);
                }
                if let Some(handle) = outbox.handle(channel) {
                    let _ = handle.send(SessionCommand::Input(bytes));
                }
            }
            (channel, Message::Mouse(event))
                if channel.0 > 0 && channel.0 <= requests.last_channel.load(Ordering::Acquire) =>
            {
                if let Some(handle) = outbox.handle(channel) {
                    if validation.is_err() {
                        outbox.close_with(fatal("invalid mouse event"));
                        return Ok(Exit::Fatal);
                    }
                    let _ = handle.send(SessionCommand::Mouse(event));
                }
            }
            _ => {
                outbox.close_with(fatal("misplaced client message"));
                return Ok(Exit::Fatal);
            }
        }
    }
    Ok(Exit::Disconnected)
}

struct Requests {
    registry: Arc<Registry>,
    outbox: Arc<Outbox>,
    workers: WorkerPool,
    search_cache: Arc<Mutex<SearchCache>>,
    version: Version,
    last_channel: Arc<AtomicU32>,
}

impl Requests {
    fn route(&self, body: RequestBody, id: RequestId) -> Result<Option<ReplyBody>, ServerError> {
        let version = self.version;
        if message_version(&Message::Request {
            id,
            body: body.clone(),
        }) > version
        {
            return Err(ServerError::new(
                ErrorCode::BadRequest,
                "request requires a newer protocol version",
            ));
        }
        let outbox = &self.outbox;
        match body {
            RequestBody::Ping => Ok(Some(ReplyBody::Pong)),
            body => {
                let registry = Arc::clone(&self.registry);
                let output = Arc::clone(outbox);
                let cache = Arc::clone(&self.search_cache);
                let last_channel = Arc::clone(&self.last_channel);
                self.workers
                    .try_spawn(move || {
                        if output.is_closed() {
                            return;
                        }
                        let result =
                            ordered_request(body, id, &registry, &output, &cache, &last_channel);
                        let body = match result {
                            Ok(Some(body)) => body,
                            Ok(None) => return,
                            Err(error) => ReplyBody::Error(error.to_reply()),
                        };
                        output.push_control(Message::Reply { id, body });
                    })
                    .map_err(|error| ServerError::new(ErrorCode::BadRequest, error.to_string()))?;
                Ok(None)
            }
        }
    }
}

fn ordered_request(
    body: RequestBody,
    id: RequestId,
    registry: &Registry,
    outbox: &Arc<Outbox>,
    search_cache: &Mutex<SearchCache>,
    last_channel: &AtomicU32,
) -> Result<Option<ReplyBody>, ServerError> {
    Ok(Some(match body {
        RequestBody::SetTerminalColors(colors) => {
            outbox.set_colors(colors);
            ReplyBody::TerminalColorsSet
        }
        RequestBody::Attach { session, size } => {
            attach(session, size, id, registry, outbox, last_channel)?;
            return Ok(None);
        }
        RequestBody::Detach(channel) => {
            outbox.detach(channel)?;
            ReplyBody::Detached
        }
        RequestBody::Resize { channel, size } => {
            outbox.resize(channel, id, size)?;
            return Ok(None);
        }
        RequestBody::ListSessions => ReplyBody::Sessions(registry.list()),
        RequestBody::CreateSession { directory, size } => {
            ReplyBody::SessionCreated(registry.create_with_colors(
                Path::new(OsStr::from_bytes(&directory.0)),
                size,
                outbox.colors(),
            )?)
        }
        RequestBody::EndSession(session) => {
            registry.end(session)?;
            ReplyBody::SessionEnded
        }
        RequestBody::ReadSavedScreen(session) => {
            ReplyBody::SavedScreen(registry.read_saved_screen(session)?)
        }
        RequestBody::HistoryPage {
            channel,
            before,
            max_rows,
        } => {
            let handle = outbox
                .handle(channel)
                .ok_or_else(|| ServerError::new(ErrorCode::UnknownChannel, "unknown channel"))?;
            let (reply, page) = mpsc::channel();
            handle.send(SessionCommand::HistoryPage {
                before,
                max_rows,
                reply,
            })?;
            let page = page.recv_timeout(Duration::from_secs(5)).map_err(|_| {
                ServerError::new(
                    ErrorCode::HistoryUnavailable,
                    "history request did not complete",
                )
            })??;
            ReplyBody::HistoryPage(page)
        }
        RequestBody::SavedHistoryPage {
            session,
            before,
            max_rows,
        } => ReplyBody::HistoryPage(registry.saved_history_page(session, before, max_rows)?),
        RequestBody::DiscardSession(session) => {
            registry.discard(session)?;
            ReplyBody::SessionDiscarded
        }

        RequestBody::Search {
            source,
            query,
            ignore_case,
            before,
            max_results,
        } => match source {
            SearchSource::Live(channel) => {
                live_search(outbox, channel, query, ignore_case, before, max_results)?
            }
            SearchSource::Saved(session) => ReplyBody::SearchPage(
                registry.archive().search(
                    session,
                    &query,
                    ignore_case,
                    before,
                    max_results,
                    &mut search_cache
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                )?,
            ),
        },
        RequestBody::Ping => ReplyBody::Pong,
    }))
}

fn live_search(
    outbox: &Arc<Outbox>,
    channel: ChannelId,
    query: String,
    ignore_case: bool,
    before: HistoryCursor,
    max_results: u16,
) -> Result<ReplyBody, ServerError> {
    let handle = outbox
        .handle(channel)
        .ok_or_else(|| ServerError::new(ErrorCode::UnknownChannel, "unknown channel"))?;
    let (reply, response) = mpsc::channel();
    handle.send(SessionCommand::Search {
        query,
        ignore_case,
        before,
        max_results,
        reply,
    })?;
    Ok(ReplyBody::SearchPage(
        response
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| {
                ServerError::new(ErrorCode::HistoryUnavailable, "search did not complete")
            })??,
    ))
}

fn attach(
    session: SessionId,
    size: Size,
    request: RequestId,
    registry: &Registry,
    outbox: &Arc<Outbox>,
    last_channel: &AtomicU32,
) -> Result<(), ServerError> {
    let handle = registry
        .handle(session)
        .ok_or_else(|| ServerError::unknown_session(session))?;
    let channel = ChannelId(
        last_channel
            .load(Ordering::Relaxed)
            .checked_add(1)
            .ok_or_else(|| ServerError::new(ErrorCode::BadRequest, "channel IDs exhausted"))?,
    );
    let id = AttachmentId(
        NEXT_ATTACHMENT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| ServerError::new(ErrorCode::BadRequest, "attachment IDs exhausted"))?,
    );
    let (sink, events) = mpsc::channel();
    outbox.attach(
        channel,
        id,
        request,
        handle,
        SessionCommand::Attach {
            id,
            channel,
            size,
            sink,
        },
    )?;
    last_channel.store(channel.0, Ordering::Release);
    let output = Arc::clone(outbox);
    let spawned = thread::Builder::new()
        .name(format!("attachment-{}", id.0))
        .spawn(move || {
            while output.handle(channel).is_some() {
                match events.recv_timeout(POLL) {
                    Ok(AttachmentEvent::Snapshot { snapshot, process }) => {
                        output.snapshot(channel, snapshot, process);
                    }
                    Ok(AttachmentEvent::Metadata(event)) => output.push_metadata(channel, event),
                    Ok(AttachmentEvent::Frame(frame)) => output.push_frame(channel, frame),
                    Ok(AttachmentEvent::Resized(frame)) => output.resized(channel, frame),
                    Ok(AttachmentEvent::Ended(_)) | Err(RecvTimeoutError::Disconnected) => {
                        output.attach_failed(channel, &ServerError::unknown_session(session));
                        break;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        });
    if let Err(error) = spawned {
        outbox.attach_failed(channel, &ServerError::spawn_failed(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
