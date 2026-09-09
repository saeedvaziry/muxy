use std::num::NonZeroU64;

use crate::{
    AttachSnapshot, ChannelId, Color, Cursor, ErrorCode, ErrorReply, ExitReason, HistoryCursor,
    HistoryPage, InputModes, Message, MetadataEvent, Modes, Modifiers, MouseAction, MouseButton,
    MouseEvent, ReplyBody, RequestBody, RequestId, Row, Run, ScreenFrame, SearchMatch, SearchPage,
    SearchSource, ServerPath, SessionId, Size, Style, TerminalColors, V1,
};

impl Message {
    pub fn samples() -> Vec<Self> {
        let session = SessionId::from(NonZeroU64::MIN);
        let channel = ChannelId(1);
        let size = Size { cols: 4, rows: 1 };
        let directory = ServerPath(b"/tmp".to_vec());
        let rows = vec![Row {
            index: 0,
            runs: vec![Run {
                text: "Muxy".to_owned(),
                width: 4,
                style: Style {
                    fg: Color::Rgb(80, 180, 240),
                    bg: Color::Indexed(0),
                    bold: true,
                    ..Style::default()
                },
            }],
        }];
        let cursor = Cursor {
            row: 0,
            col: 3,
            visible: true,
        };
        let modes = Modes {
            application_cursor_keys: true,
            bracketed_paste: true,
        };

        let mut samples = vec![
            Self::Hello { versions: vec![V1] },
            Self::Request {
                id: RequestId(1),
                body: RequestBody::CreateSession {
                    directory: directory.clone(),
                    size,
                },
            },
            Self::FrameAck { channel, seq: 1 },
            Self::HelloReply { versions: vec![V1] },
            Self::VersionUnsupported,
            Self::Reply {
                id: RequestId(2),
                body: ReplyBody::Attached {
                    snapshot: Box::new(AttachSnapshot {
                        channel,
                        size,
                        rows: rows.clone(),
                        cursor,
                        modes,
                        title: "Muxy".to_owned(),
                        directory,
                        history: Vec::new(),
                        history_cursor: None,
                        history_total: 0,
                    }),
                    process: None,
                },
            },
            Self::SessionEnded {
                session,
                reason: ExitReason::Exited(0),
            },
            Self::Fatal(ErrorReply {
                code: ErrorCode::BadRequest,
                message: "expected hello".to_owned(),
            }),
            Self::Input(b"pwd\r".to_vec()),
            Self::Frame(ScreenFrame {
                seq: 1,
                reset: true,
                rows,
                cursor,
                modes,
            }),
            Self::Metadata(MetadataEvent::Title("Muxy".to_owned())),
        ];
        let snapshot = samples.iter().find_map(|message| match message {
            Self::Reply {
                body: ReplyBody::Attached { snapshot, .. },
                ..
            } => Some(*snapshot.clone()),
            _ => None,
        });
        samples.extend(snapshot.into_iter().flat_map(history_samples));
        samples.extend(input_samples());
        samples.extend(search_samples(session, channel));
        samples.extend(color_samples());
        samples.push(Self::Metadata(MetadataEvent::CursorBlinking(true)));
        samples
    }
}

fn search_samples(session: SessionId, channel: ChannelId) -> Vec<Message> {
    vec![
        Message::Request {
            id: RequestId(6),
            body: RequestBody::Search {
                source: SearchSource::Live(channel),
                query: "Muxy".into(),
                ignore_case: false,
                before: HistoryCursor(0),
                max_results: 500,
            },
        },
        Message::Request {
            id: RequestId(7),
            body: RequestBody::Search {
                source: SearchSource::Saved(session),
                query: "muxy".into(),
                ignore_case: true,
                before: HistoryCursor(42),
                max_results: 100,
            },
        },
        Message::Reply {
            id: RequestId(6),
            body: ReplyBody::SearchPage(SearchPage {
                matches: vec![SearchMatch {
                    row: 100,
                    start: 2,
                    end: 6,
                }],
                next: Some(HistoryCursor(42)),
                total_rows: 100,
                scanned_rows: 2000,
            }),
        },
    ]
}

fn history_samples(mut snapshot: AttachSnapshot) -> Vec<Message> {
    snapshot.history.clone_from(&snapshot.rows);
    snapshot.history_total = 2;
    snapshot.history_cursor = Some(HistoryCursor(42));
    vec![
        Message::Metadata(MetadataEvent::History { total_rows: 5000 }),
        Message::Request {
            id: RequestId(3),
            body: RequestBody::HistoryPage {
                channel: snapshot.channel,
                before: HistoryCursor(42),
                max_rows: 500,
            },
        },
        Message::Request {
            id: RequestId(4),
            body: RequestBody::SavedHistoryPage {
                session: SessionId::from(NonZeroU64::MIN),
                before: HistoryCursor(0),
                max_rows: 200,
            },
        },
        Message::Reply {
            id: RequestId(3),
            body: ReplyBody::HistoryPage(HistoryPage {
                rows: snapshot.history.clone(),
                next: None,
                total_rows: 2,
                screen: None,
            }),
        },
        Message::Reply {
            id: RequestId(5),
            body: ReplyBody::Attached {
                snapshot: Box::new(snapshot),
                process: None,
            },
        },
    ]
}

fn color_samples() -> [Message; 2] {
    [
        Message::Request {
            id: RequestId(8),
            body: RequestBody::SetTerminalColors(TerminalColors {
                foreground: [0xc9, 0xc2, 0xd9],
                background: [0x19, 0x17, 0x1f],
                cursor: [0xc3, 0x70, 0xd3],
                ansi: [[0x12, 0x34, 0x56]; 16],
            }),
        },
        Message::Reply {
            id: RequestId(8),
            body: ReplyBody::TerminalColorsSet,
        },
    ]
}

fn input_samples() -> [Message; 2] {
    [
        Message::Mouse(MouseEvent {
            action: MouseAction::Press,
            button: Some(MouseButton::Left),
            column: 3,
            row: 1,
            scroll: None,
            modifiers: Modifiers {
                shift: false,
                alt: true,
                ctrl: false,
            },
        }),
        Message::Metadata(MetadataEvent::InputModes(InputModes {
            mouse_tracking: true,
            alternate_scroll: false,
            focus_events: true,
        })),
    ]
}
