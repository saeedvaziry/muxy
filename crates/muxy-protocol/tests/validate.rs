use std::collections::BTreeSet;
use std::num::NonZeroU64;

use muxy_protocol::{
    AttachSnapshot, CONTROL, ChannelId, ChannelKind, Cursor, ErrorCode, ErrorReply, ExitReason,
    MAX_COLS, MAX_INPUT, MAX_ROWS, Message, MetadataEvent, Modes, ReplyBody, RequestBody,
    RequestId, Row, Run, SUPPORTED, ServerPath, SessionId, SessionInfo, Size, Style, V1, Version,
    validate_input, validate_path, validate_size, validate_versions,
};
use serde::Deserialize;
use serde::de::value::{Error, SeqDeserializer, U64Deserializer};

#[test]
fn sizes_accept_both_boundaries_in_each_dimension() {
    assert_eq!(MAX_COLS, 4096);
    assert_eq!(MAX_ROWS, 1024);
    for cols in [1, 4096] {
        for rows in [1, 1024] {
            assert_eq!(validate_size(Size { cols, rows }), Ok(()));
        }
    }
}

#[test]
fn sizes_reject_zero_and_each_dimension_above_its_limit() {
    for size in [
        Size { cols: 0, rows: 1 },
        Size { cols: 1, rows: 0 },
        Size {
            cols: 4097,
            rows: 1,
        },
        Size {
            cols: 1,
            rows: 1025,
        },
        Size {
            cols: u16::MAX,
            rows: 1,
        },
        Size {
            cols: 1,
            rows: u16::MAX,
        },
    ] {
        assert_eq!(validate_size(size), Err(ErrorCode::BadSize), "{size:?}");
    }
}

#[test]
fn input_accepts_empty_and_one_mebibyte_but_rejects_one_more_byte() {
    assert_eq!(MAX_INPUT, 1_048_576);
    assert_eq!(validate_input(&[]), Ok(()));
    assert_eq!(validate_input(&vec![0; 1_048_576]), Ok(()));
    assert_eq!(
        validate_input(&vec![0; 1_048_577]),
        Err(ErrorCode::BadRequest)
    );
    assert_eq!(Message::Input(vec![0; 1_048_576]).validate(), Ok(()));
    assert_eq!(
        Message::Input(vec![0; 1_048_577]).validate(),
        Err(ErrorCode::BadRequest)
    );
}

#[test]
fn paths_reject_only_empty_bytes() -> Result<(), Error> {
    assert_eq!(
        validate_path(&ServerPath(Vec::new())),
        Err(ErrorCode::BadPath)
    );
    for bytes in [vec![b'/'], vec![0xff], vec![0], b"relative/path".to_vec()] {
        let path =
            ServerPath::deserialize(SeqDeserializer::<_, Error>::new(bytes.clone().into_iter()))?;
        assert_eq!(path.0, bytes);
        assert_eq!(validate_path(&path), Ok(()));
    }
    Ok(())
}

#[test]
fn version_lists_require_an_entry_without_negotiating_support() {
    assert_eq!(V1, Version(1));
    assert_eq!(SUPPORTED, &[V1]);
    assert_eq!(validate_versions(&[]), Err(ErrorCode::BadRequest));
    assert_eq!(validate_versions(&[V1]), Ok(()));
    assert_eq!(validate_versions(&[Version(2)]), Ok(()));
    for versions in [vec![], vec![V1], vec![Version(2), V1]] {
        let expected = if versions.is_empty() {
            Err(ErrorCode::BadRequest)
        } else {
            Ok(())
        };
        assert_eq!(
            Message::Hello {
                versions: versions.clone()
            }
            .validate(),
            expected
        );
        assert_eq!(Message::HelloReply { versions }.validate(), expected);
    }
}

#[test]
fn session_ids_are_nonzero_in_construction_and_deserialization() -> Result<(), Error> {
    assert_eq!(SessionId::new(0), None);
    assert!(SessionId::deserialize(U64Deserializer::<Error>::new(0)).is_err());
    for value in [1, u64::MAX] {
        let id = SessionId::deserialize(U64Deserializer::<Error>::new(value))?;
        assert_eq!(SessionId::new(value), Some(id));
        assert_eq!(id.get(), value);
    }
    assert_eq!(session_id().get(), 1);
    assert_eq!(CONTROL, ChannelId(0));
    Ok(())
}

#[test]
fn every_request_with_a_size_validates_it() {
    for (size, expected) in [
        (Size { cols: 1, rows: 1 }, Ok(())),
        (
            Size {
                cols: 4096,
                rows: 1024,
            },
            Ok(()),
        ),
        (Size { cols: 0, rows: 1 }, Err(ErrorCode::BadSize)),
        (
            Size {
                cols: 1,
                rows: 1025,
            },
            Err(ErrorCode::BadSize),
        ),
    ] {
        for body in [
            RequestBody::CreateSession {
                directory: directory(),
                size,
            },
            RequestBody::Attach {
                session: session_id(),
                size,
            },
            RequestBody::Resize {
                channel: ChannelId(1),
                size,
            },
        ] {
            assert_eq!(request(body).validate(), expected);
        }
    }
}

#[test]
fn every_message_with_a_path_validates_it() {
    for (directory, expected) in [
        (directory(), Ok(())),
        (ServerPath(Vec::new()), Err(ErrorCode::BadPath)),
    ] {
        let info = SessionInfo {
            id: session_id(),
            directory: directory.clone(),
        };
        let mut snapshot = snapshot();
        snapshot.directory = directory.clone();
        for message in [
            request(RequestBody::CreateSession {
                directory: directory.clone(),
                size: snapshot.size,
            }),
            reply(ReplyBody::SessionCreated(info.clone())),
            reply(ReplyBody::Sessions(vec![
                SessionInfo {
                    id: session_id(),
                    directory: self::directory(),
                },
                info,
            ])),
            reply(ReplyBody::Attached {
                snapshot: Box::new(snapshot),
                process: None,
            }),
            Message::Metadata(MetadataEvent::Directory(directory)),
        ] {
            assert_eq!(message.validate(), expected, "{message:?}");
        }
    }
}

#[test]
fn attach_snapshots_validate_size() {
    let mut snapshot = snapshot();
    snapshot.size.cols = 4097;
    assert_eq!(
        reply(ReplyBody::Attached {
            snapshot: Box::new(snapshot.clone()),
            process: None
        })
        .validate(),
        Err(ErrorCode::BadSize)
    );
    snapshot.size = Size { cols: 1, rows: 0 };
    assert_eq!(
        reply(ReplyBody::Attached {
            snapshot: Box::new(snapshot),
            process: None
        })
        .validate(),
        Err(ErrorCode::BadSize)
    );
}

#[test]
fn requests_without_limited_fields_are_valid() {
    for body in [
        RequestBody::ListSessions,
        RequestBody::EndSession(session_id()),
        RequestBody::Detach(ChannelId(1)),
        RequestBody::Ping,
    ] {
        assert_eq!(request(body).validate(), Ok(()));
    }
}

#[test]
fn replies_without_limited_fields_are_valid() {
    for body in [
        ReplyBody::Sessions(Vec::new()),
        ReplyBody::SessionEnded,
        ReplyBody::Detached,
        ReplyBody::Resized,
        ReplyBody::Pong,
        ReplyBody::Error(ErrorReply {
            code: ErrorCode::UnknownSession,
            message: "unknown session".to_owned(),
        }),
    ] {
        assert_eq!(reply(body).validate(), Ok(()));
    }
}

#[test]
fn metadata_and_exit_reasons_without_limited_fields_are_valid() {
    for event in [
        MetadataEvent::Title(String::new()),
        MetadataEvent::ForegroundProcess {
            name: "zsh".to_owned(),
            is_shell: true,
        },
        MetadataEvent::Bell,
    ] {
        assert_eq!(Message::Metadata(event).validate(), Ok(()));
    }
    for reason in [
        ExitReason::Exited(3),
        ExitReason::Signaled(15),
        ExitReason::Ended,
        ExitReason::ServerStopped,
    ] {
        assert_eq!(
            Message::SessionEnded {
                session: session_id(),
                reason
            }
            .validate(),
            Ok(())
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn samples_cover_every_message_variant_once_and_use_the_right_channel() {
    let mut seen = BTreeSet::new();
    for message in Message::samples() {
        let (name, channel) = match &message {
            Message::Request {
                body: RequestBody::SetTerminalColors(_),
                ..
            } => ("TerminalColorsRequest", ChannelKind::Control),
            Message::Reply {
                body: ReplyBody::TerminalColorsSet,
                ..
            } => ("TerminalColorsReply", ChannelKind::Control),
            Message::Request {
                body:
                    RequestBody::Search {
                        source: muxy_protocol::SearchSource::Live(_),
                        ..
                    },
                ..
            } => ("SearchRequest", ChannelKind::Control),
            Message::Request {
                body:
                    RequestBody::Search {
                        source: muxy_protocol::SearchSource::Saved(_),
                        ..
                    },
                ..
            } => ("SavedSearchRequest", ChannelKind::Control),
            Message::Reply {
                body: ReplyBody::SearchPage(_),
                ..
            } => ("SearchReply", ChannelKind::Control),
            Message::Hello { .. } => ("Hello", ChannelKind::Control),
            Message::Request {
                body: RequestBody::HistoryPage { .. },
                ..
            } => ("HistoryRequest", ChannelKind::Control),
            Message::Request {
                body: RequestBody::SavedHistoryPage { .. },
                ..
            } => ("SavedHistoryRequest", ChannelKind::Control),
            Message::Request { .. } => ("Request", ChannelKind::Control),
            Message::FrameAck { .. } => ("FrameAck", ChannelKind::Control),
            Message::HelloReply { .. } => ("HelloReply", ChannelKind::Control),
            Message::VersionUnsupported => ("VersionUnsupported", ChannelKind::Control),
            Message::Reply {
                body: ReplyBody::HistoryPage(_),
                ..
            } => ("HistoryReply", ChannelKind::Control),
            Message::Reply {
                body: ReplyBody::Attached { snapshot, .. },
                ..
            } if !snapshot.history.is_empty() => ("HistoryAttach", ChannelKind::Control),
            Message::Reply { .. } => ("Reply", ChannelKind::Control),
            Message::SessionEnded { .. } => ("SessionEnded", ChannelKind::Control),
            Message::Fatal(_) => ("Fatal", ChannelKind::Control),
            Message::Input(_) => ("Input", ChannelKind::Session),
            Message::Mouse(_) => ("Mouse", ChannelKind::Session),
            Message::Frame(_) => ("Frame", ChannelKind::Session),
            Message::Metadata(MetadataEvent::History { .. }) => {
                ("HistoryMetadata", ChannelKind::Session)
            }
            Message::Metadata(MetadataEvent::Links { .. }) => ("Links", ChannelKind::Session),
            Message::Metadata(MetadataEvent::InputModes(_)) => ("InputModes", ChannelKind::Session),
            Message::Metadata(MetadataEvent::CursorBlinking(_)) => {
                ("CursorBlinking", ChannelKind::Session)
            }
            Message::Metadata(_) => ("Metadata", ChannelKind::Session),
        };
        assert!(seen.insert(name), "duplicate sample: {name}");
        assert_eq!(message.channel_kind(), channel, "{name}");
        assert_eq!(message.validate(), Ok(()), "{name}");
    }
    assert_eq!(
        seen,
        BTreeSet::from([
            "Hello",
            "Request",
            "SearchRequest",
            "SavedSearchRequest",
            "SearchReply",
            "TerminalColorsRequest",
            "TerminalColorsReply",
            "HistoryRequest",
            "SavedHistoryRequest",
            "HistoryReply",
            "HistoryAttach",
            "HistoryMetadata",
            "Links",
            "FrameAck",
            "HelloReply",
            "VersionUnsupported",
            "Reply",
            "SessionEnded",
            "Fatal",
            "Input",
            "Mouse",
            "InputModes",
            "CursorBlinking",
            "Frame",
            "Metadata",
        ])
    );
}

fn session_id() -> SessionId {
    SessionId::from(NonZeroU64::MIN)
}

#[test]
fn mouse_events_require_consistent_actions_but_allow_positions_past_the_viewport() {
    use muxy_protocol::{Modifiers, MouseAction, MouseButton, MouseEvent, ScrollDirection};
    for action in [
        MouseAction::Press,
        MouseAction::Release,
        MouseAction::Motion,
        MouseAction::Scroll,
    ] {
        for button in [None, Some(MouseButton::Left)] {
            for scroll in [None, Some(ScrollDirection::Up)] {
                let expected = match action {
                    MouseAction::Press | MouseAction::Release => {
                        button.is_some() && scroll.is_none()
                    }
                    MouseAction::Motion => scroll.is_none(),
                    MouseAction::Scroll => button.is_none() && scroll.is_some(),
                };
                let message = Message::Mouse(MouseEvent {
                    action,
                    button,
                    scroll,
                    column: u16::MAX,
                    row: u16::MAX,
                    modifiers: Modifiers::default(),
                });
                assert_eq!(message.validate().is_ok(), expected, "{message:?}");
            }
        }
    }
}

fn directory() -> ServerPath {
    ServerPath(b"/tmp".to_vec())
}

fn request(body: RequestBody) -> Message {
    Message::Request {
        id: RequestId(1),
        body,
    }
}

fn reply(body: ReplyBody) -> Message {
    Message::Reply {
        id: RequestId(1),
        body,
    }
}

fn snapshot() -> AttachSnapshot {
    AttachSnapshot {
        channel: ChannelId(1),
        size: Size { cols: 1, rows: 1 },
        rows: vec![Row {
            index: 0,
            runs: vec![Run {
                text: " ".to_owned(),
                width: 1,
                style: Style::default(),
            }],
        }],
        cursor: Cursor {
            row: 0,
            col: 0,
            visible: true,
        },
        modes: Modes::default(),
        title: String::new(),
        directory: directory(),
        history: vec![],
        history_cursor: None,
        history_total: 0,
    }
}

#[test]
fn history_limits_and_page_shape_are_validated() {
    use muxy_protocol::{HistoryCursor, HistoryPage};
    for max_rows in [0, 1, 200, 500, 501, u16::MAX] {
        let expected = if (1..=500).contains(&max_rows) {
            Ok(())
        } else {
            Err(ErrorCode::BadRequest)
        };
        for body in [
            RequestBody::HistoryPage {
                channel: ChannelId(1),
                before: HistoryCursor(0),
                max_rows,
            },
            RequestBody::SavedHistoryPage {
                session: session_id(),
                before: HistoryCursor(12),
                max_rows,
            },
        ] {
            assert_eq!(request(body).validate(), expected);
        }
    }
    assert_eq!(
        request(RequestBody::HistoryPage {
            channel: CONTROL,
            before: HistoryCursor(0),
            max_rows: 500
        })
        .validate(),
        Err(ErrorCode::UnknownChannel)
    );
    let valid = HistoryPage {
        rows: snapshot().rows,
        next: Some(HistoryCursor(3)),
        total_rows: 5,
        screen: None,
    };
    assert_eq!(
        reply(ReplyBody::HistoryPage(valid.clone())).validate(),
        Ok(())
    );
    for page in [
        HistoryPage {
            next: Some(HistoryCursor(0)),
            ..valid.clone()
        },
        HistoryPage {
            total_rows: 0,
            ..valid.clone()
        },
        HistoryPage {
            rows: vec![],
            ..valid.clone()
        },
        HistoryPage {
            rows: vec![Row {
                index: 1,
                runs: vec![],
            }],
            ..valid.clone()
        },
        HistoryPage {
            rows: vec![Row {
                index: 0,
                runs: vec![Run {
                    text: "x".into(),
                    width: 0,
                    style: Style::default(),
                }],
            }],
            ..valid
        },
    ] {
        assert_eq!(
            reply(ReplyBody::HistoryPage(page)).validate(),
            Err(ErrorCode::BadRequest)
        );
    }
}
