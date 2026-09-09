use super::*;

#[test]
fn saturated_requests_retain_latest_sizes_without_blocking_input_or_flush() -> TestResult {
    check_resize_delivery(true)
}

#[test]
fn resize_coalescing_preserves_request_and_detach_order() -> TestResult {
    check_resize_delivery(false)
}

fn check_resize_delivery(saturated: bool) -> TestResult {
    let directory = PathBuf::from(format!(
        "/tmp/muxy-resize-{}-{saturated}",
        std::process::id(),
    ));
    fs::create_dir(&directory)?;
    let socket = directory.join("server.sock");
    let listener = UnixListener::bind(&socket)?;
    let (progress, received) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let server = thread::spawn(move || resize_server(&listener, &progress, &gate));
    let (work, updates) = bridge(socket)?;
    work.send((1, Work::Connect))?;
    assert!(matches!(updates.recv_blocking()?.1, Update::Connected(_)));
    let session = SessionId::from(std::num::NonZeroU64::MIN);
    let read = || Work::ReadSaved {
        pane: PaneId::new(),
        session,
    };
    work.send((1, read()))?;
    received.recv_timeout(Duration::from_secs(2))?;
    let mut expected = Vec::new();
    if saturated {
        for _ in 0..128 {
            work.send((1, read()))?;
            expected.push(RequestBody::ReadSavedScreen(session));
        }
    }
    for cols in 101..=230 {
        for channel in [ChannelId(1), ChannelId(2)] {
            work.send((1, Work::Resize(channel, Size { cols, rows: 24 })))?;
        }
    }
    work.send((
        1,
        Work::Resize(
            ChannelId(1),
            Size {
                cols: 231,
                rows: 24,
            },
        ),
    ))?;
    for (channel, cols) in [(ChannelId(2), 230), (ChannelId(1), 231)] {
        expected.push(RequestBody::Resize {
            channel,
            size: Size { cols, rows: 24 },
        });
    }
    if !saturated {
        work.send((1, read()))?;
        expected.push(RequestBody::ReadSavedScreen(session));
        for cols in 231..=260 {
            work.send((1, Work::Resize(ChannelId(1), Size { cols, rows: 24 })))?;
        }
        expected.push(RequestBody::Resize {
            channel: ChannelId(1),
            size: Size {
                cols: 260,
                rows: 24,
            },
        });
        work.send((1, Work::Detach(ChannelId(1))))?;
        expected.push(RequestBody::Detach(ChannelId(1)));
    }
    work.send((1, Work::Flush))?;
    work.send((1, Work::Input(ChannelId(1), b"input".to_vec())))?;
    work.send((1, Work::Ack(ChannelId(1), 17)))?;
    let fast_path = received.recv_timeout(Duration::from_secs(2));
    let early = updates.try_recv();
    release.send(())?;
    fast_path?;
    assert!(
        early.is_err(),
        "flush or rejection overtook retained work: {early:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut saved = 0;
    let mut flushed = false;
    while !flushed && Instant::now() < deadline {
        match updates.try_recv() {
            Ok((1, Update::Saved { .. })) => saved += 1,
            Ok((1, Update::Flushed)) => flushed = true,
            Ok(other) => return Err(format!("unexpected update: {other:?}").into()),
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
    work.send((1, Work::Stop))?;
    let observed = server.join().map_err(|_| "fake resize server panicked")??;
    fs::remove_dir_all(directory)?;
    assert!(flushed, "flush must include the retained final resize");
    assert_eq!(saved, if saturated { 129 } else { 2 });
    assert_eq!(observed, expected);
    Ok(())
}

fn resize_server(
    listener: &UnixListener,
    progress: &Sender<()>,
    gate: &mpsc::Receiver<()>,
) -> Result<Vec<RequestBody>, Box<dyn Error + Send + Sync>> {
    let (socket, _) = listener.accept()?;
    socket.set_read_timeout(Some(Duration::from_secs(4)))?;
    let mut decoder = Decoder::new(socket.try_clone()?);
    let mut encoder = Encoder::new(socket);
    assert!(matches!(decoder.next()?, (CONTROL, Message::Hello { .. })));
    encoder.send(
        CONTROL,
        &Message::HelloReply {
            versions: SUPPORTED.to_vec(),
        },
    )?;
    let mut blocked = true;
    let mut observed = Vec::new();
    while let Ok((CONTROL, Message::Request { id, body })) = decoder.next() {
        let reply = match body {
            RequestBody::ListSessions => ReplyBody::Sessions(Vec::new()),
            RequestBody::ReadSavedScreen(_) => {
                if blocked {
                    blocked = false;
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
                } else {
                    observed.push(body);
                }
                ReplyBody::Error(muxy_protocol::ErrorReply {
                    code: ErrorCode::SavedContentUnavailable,
                    message: "test record".into(),
                })
            }
            RequestBody::Resize { .. } => {
                observed.push(body);
                ReplyBody::Resized
            }
            RequestBody::Detach(_) => {
                observed.push(body);
                ReplyBody::Detached
            }
            other => return Err(format!("unexpected request: {other:?}").into()),
        };
        encoder.send(CONTROL, &Message::Reply { id, body: reply })?;
    }
    Ok(observed)
}

#[test]
fn reconnect_discards_old_resizes_and_stale_completion_unblocks_new_work() -> TestResult {
    let directory = PathBuf::from(format!("/tmp/muxy-resize-reconnect-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let socket = directory.join("server.sock");
    let listener = UnixListener::bind(&socket)?;
    let (started, running) = mpsc::channel();
    let server = thread::spawn(move || reconnect_server(&listener, &started));
    let (work, updates) = bridge(socket)?;
    work.send((1, Work::Connect))?;
    assert!(matches!(
        updates.recv_blocking()?,
        (1, Update::Connected(_))
    ));
    work.send((
        1,
        Work::ReadSaved {
            pane: PaneId::new(),
            session: SessionId::from(std::num::NonZeroU64::MIN),
        },
    ))?;
    running.recv_timeout(Duration::from_secs(2))?;
    work.send((
        1,
        Work::Resize(
            ChannelId(1),
            Size {
                cols: 150,
                rows: 24,
            },
        ),
    ))?;
    work.send((2, Work::Connect))?;
    work.send((
        2,
        Work::Resize(
            ChannelId(1),
            Size {
                cols: 250,
                rows: 24,
            },
        ),
    ))?;
    work.send((2, Work::Flush))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut connected = false;
    let mut flushed = false;
    while !flushed && Instant::now() < deadline {
        match updates.try_recv() {
            Ok((2, Update::Connected(_))) => connected = true,
            Ok((2, Update::Flushed)) => {
                assert!(connected);
                flushed = true;
            }
            Ok(other) => return Err(format!("unexpected update: {other:?}").into()),
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
    work.send((2, Work::Stop))?;
    server
        .join()
        .map_err(|_| "fake reconnect server panicked")??;
    fs::remove_dir_all(directory)?;
    assert!(
        flushed,
        "stale completion must release the request lane without completing new work"
    );
    Ok(())
}

fn reconnect_server(listener: &UnixListener, started: &Sender<()>) -> TestResult {
    for generation in 1..=2 {
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
        if generation == 1 {
            assert!(matches!(
                decoder.next()?,
                (
                    CONTROL,
                    Message::Request {
                        body: RequestBody::ReadSavedScreen(_),
                        ..
                    }
                )
            ));
            started.send(())?;
        } else {
            let (CONTROL, Message::Request { id, body }) = decoder.next()? else {
                return Err("expected resize".into());
            };
            assert_eq!(
                body,
                RequestBody::Resize {
                    channel: ChannelId(1),
                    size: Size {
                        cols: 250,
                        rows: 24
                    }
                }
            );
            encoder.send(
                CONTROL,
                &Message::Reply {
                    id,
                    body: ReplyBody::Resized,
                },
            )?;
        }
        assert!(
            decoder.next().is_err(),
            "no old-generation requests may follow"
        );
    }
    Ok(())
}
