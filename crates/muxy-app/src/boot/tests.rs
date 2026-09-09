use super::*;
use std::error::Error;
use std::fs;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use muxy_protocol::{CONTROL, Message, ReplyBody, RequestBody, SUPPORTED};
use muxy_wire::{Decoder, Encoder};

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

#[test]
fn blocked_client_request_does_not_block_input_or_acks_and_flush_waits() -> TestResult {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = PathBuf::from(format!(
        "/tmp/muxy-bridge-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory)?;
    let socket = directory.join("server.sock");
    let listener = UnixListener::bind(&socket)?;
    let (progress, received) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let server = thread::spawn(move || fake_server(&listener, &progress, &gate));
    let (work, updates) = bridge(socket)?;
    work.send((1, Work::Connect))?;
    assert!(matches!(updates.recv_blocking()?.1, Update::Connected(_)));
    let session = SessionId::from(std::num::NonZeroU64::MIN);
    work.send((
        1,
        Work::ReadSaved {
            pane: PaneId::new(),
            session,
        },
    ))?;
    received.recv_timeout(Duration::from_secs(2))?;
    work.send((1, Work::Input(ChannelId(1), b"input".to_vec())))?;
    work.send((1, Work::Ack(ChannelId(1), 17)))?;
    work.send((1, Work::Flush))?;
    let fast_path_completed = received.recv_timeout(Duration::from_secs(2));
    let early_update = updates.try_recv();
    release.send(())?;
    fast_path_completed?;
    assert!(
        early_update.is_err(),
        "flush must not overtake the blocked request"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saved = false;
    let mut flushed = false;
    while !flushed && Instant::now() < deadline {
        match updates.try_recv() {
            Ok((1, Update::Saved { .. })) => saved = true,
            Ok((1, Update::Flushed)) => {
                assert!(saved);
                flushed = true;
            }
            Ok(other) => return Err(format!("unexpected update: {other:?}").into()),
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
    work.send((1, Work::Stop))?;
    server.join().map_err(|_| "fake server panicked")??;
    fs::remove_dir_all(directory)?;
    assert!(flushed, "flush must finish after pending work completes");
    Ok(())
}

fn fake_server(
    listener: &UnixListener,
    progress: &Sender<()>,
    gate: &mpsc::Receiver<()>,
) -> TestResult {
    let (socket, _) = listener.accept()?;
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut decoder = Decoder::new(socket.try_clone()?);
    let mut encoder = Encoder::new(socket);
    assert!(matches!(decoder.next()?, (CONTROL, Message::Hello { .. })));
    encoder.send(
        CONTROL,
        &Message::HelloReply {
            versions: SUPPORTED.to_vec(),
        },
    )?;
    let (
        CONTROL,
        Message::Request {
            id,
            body: RequestBody::ListSessions,
        },
    ) = decoder.next()?
    else {
        return Err("expected listing".into());
    };
    encoder.send(
        CONTROL,
        &Message::Reply {
            id,
            body: ReplyBody::Sessions(Vec::new()),
        },
    )?;
    let (
        CONTROL,
        Message::Request {
            id,
            body: RequestBody::ReadSavedScreen(_),
        },
    ) = decoder.next()?
    else {
        return Err("expected saved request".into());
    };
    progress.send(())?;
    assert_eq!(
        decoder.next()?,
        (ChannelId(1), Message::Input(b"input".to_vec()))
    );
    assert_eq!(
        decoder.next()?,
        (
            CONTROL,
            Message::FrameAck {
                channel: ChannelId(1),
                seq: 17
            }
        )
    );
    progress.send(())?;
    gate.recv_timeout(Duration::from_secs(3))?;
    encoder.send(
        CONTROL,
        &Message::Reply {
            id,
            body: ReplyBody::Error(muxy_protocol::ErrorReply {
                code: ErrorCode::SavedContentUnavailable,
                message: "test record".into(),
            }),
        },
    )?;
    let _ = decoder.next();
    Ok(())
}

#[test]
fn attachment_completion_precedes_early_frames_and_disconnect_without_blocking_other_channels()
-> TestResult {
    let mut delivery = delivery::Delivery::default();
    delivery.pending = 1;
    let first = attachment_update(ChannelId(1))?;
    assert!(matches!(
        delivery.complete(Some(first)).as_slice(),
        [Update::Attached { .. }]
    ));
    delivery.pending = 2;
    let early = frame_event(ChannelId(2));
    assert!(delivery.event(early.clone())?.is_empty());
    assert!(delivery.event(ClientEvent::Disconnected)?.is_empty());
    assert!(matches!(
        delivery.event(frame_event(ChannelId(1)))?.as_slice(),
        [Update::Event(ClientEvent::Frame {
            channel: ChannelId(1),
            ..
        })]
    ));
    assert!(delivery.flush().is_empty());
    let ready = delivery.complete(Some(attachment_update(ChannelId(2))?));
    assert!(
        matches!(ready.as_slice(), [Update::Attached { .. }, Update::Event(event)] if *event == early)
    );
    assert!(matches!(
        delivery.complete(None).as_slice(),
        [Update::Event(ClientEvent::Disconnected), Update::Flushed]
    ));
    Ok(())
}

#[test]
fn deferred_lifecycle_events_are_bounded() {
    let mut delivery = delivery::Delivery::default();
    delivery.pending = 1;
    let mut accepted = 0;
    while delivery
        .event(ClientEvent::SessionEnded {
            session: SessionId::from(std::num::NonZeroU64::MIN),
            reason: muxy_protocol::ExitReason::Ended,
        })
        .is_ok()
    {
        accepted += 1;
        assert!(accepted <= 1024);
    }
    assert_eq!(accepted, 1024);
    assert_eq!(delivery.complete(None).len(), accepted);
}

fn attachment_update(channel: ChannelId) -> Result<Update, Box<dyn Error + Send + Sync>> {
    let snapshot = Message::samples()
        .into_iter()
        .find_map(|message| match message {
            Message::Reply {
                body: ReplyBody::Attached { snapshot, .. },
                ..
            } => Some(snapshot),
            _ => None,
        })
        .ok_or("missing attachment sample")?;
    Ok(Update::Attached {
        pane: PaneId::new(),
        session: SessionId::from(std::num::NonZeroU64::MIN),
        attachment: Attachment {
            channel,
            grid: muxy_client::RunGrid::from_snapshot(&snapshot),
            title: snapshot.title,
            directory: snapshot.directory,
            process: None,
        },
        created: true,
    })
}

fn frame_event(channel: ChannelId) -> ClientEvent {
    ClientEvent::Frame {
        channel,
        frame: muxy_protocol::ScreenFrame {
            seq: 1,
            reset: false,
            rows: Vec::new(),
            cursor: muxy_protocol::Cursor {
                row: 0,
                col: 0,
                visible: true,
            },
            modes: muxy_protocol::Modes::default(),
        },
    }
}

#[test]
fn a_full_event_buffer_preserves_the_only_disconnect_after_history_completion() -> TestResult {
    let mut delivery = delivery::Delivery::default();
    delivery.pending = 1;
    for id in 1..=1024 {
        assert!(
            delivery
                .event(ClientEvent::SessionEnded {
                    session: SessionId::from(std::num::NonZeroU64::new(id).ok_or("zero session")?),
                    reason: muxy_protocol::ExitReason::Ended,
                })?
                .is_empty()
        );
    }
    assert!(delivery.event(ClientEvent::Disconnected)?.is_empty());
    assert!(delivery.flush().is_empty());
    let ready = delivery.complete(Some(Update::History {
        pane: PaneId::new(),
        request: HistoryRequest::recent(),
        result: Err(ClientError::Disconnected),
    }));
    assert!(matches!(ready.first(), Some(Update::History { .. })));
    assert!(matches!(
        &ready[ready.len() - 2..],
        [Update::Event(ClientEvent::Disconnected), Update::Flushed]
    ));
    assert_eq!(ready.len(), 1027);
    assert!(delivery.event(ClientEvent::Disconnected)?.is_empty());
    Ok(())
}

mod resize;
