use super::*;
use std::error::Error;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

type TestResult = Result<(), Box<dyn Error>>;

struct Connection {
    requests: Requests,
    replies: Receiver<(RequestId, ReplyBody)>,
    pump: Option<JoinHandle<()>>,
}

impl Connection {
    fn new() -> Result<Self, Box<dyn Error>> {
        let (events, _) = mpsc::channel();
        let registry = Arc::new(Registry::new(
            crate::ServerSettings {
                default_shell: Some("/bin/sh".into()),
                ..crate::ServerSettings::default()
            },
            events,
        ));
        let outbox = Arc::new(Outbox::new(muxy_protocol::V1));
        let output = Arc::clone(&outbox);
        let (sender, replies) = mpsc::channel();
        let pump = thread::spawn(move || {
            while let Some((_, message)) = output.next() {
                if let Message::Reply { id, body } = message
                    && sender.send((id, body)).is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            requests: Requests {
                registry,
                outbox,
                workers: WorkerPool::new("ordering-test", 1, 32)?,
                search_cache: Arc::new(Mutex::new(SearchCache::default())),
                version: muxy_protocol::V1,
                last_channel: Arc::new(AtomicU32::new(0)),
            },
            replies,
            pump: Some(pump),
        })
    }

    fn send(&mut self, id: u32, body: RequestBody) -> TestResult {
        let id = RequestId(id);
        if let Some(body) = self.requests.route(body, id)? {
            self.requests
                .outbox
                .push_control(Message::Reply { id, body });
        }
        Ok(())
    }

    fn reply(&self) -> Result<(RequestId, ReplyBody), Box<dyn Error>> {
        Ok(self.replies.recv_timeout(Duration::from_secs(5))?)
    }

    fn block(&self) -> Result<mpsc::Sender<()>, Box<dyn Error>> {
        let (release, gate) = mpsc::channel();
        let (started, ready) = mpsc::channel();
        self.requests.workers.try_spawn(move || {
            let _ = started.send(());
            let _ = gate.recv();
        })?;
        ready.recv_timeout(Duration::from_secs(2))?;
        Ok(release)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.requests.outbox.close();
        self.requests.registry.shutdown();
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
    }
}

#[test]
fn queued_end_precedes_later_attach_while_ping_bypasses_work() -> TestResult {
    let mut connection = Connection::new()?;
    let size = Size { cols: 80, rows: 24 };
    let session = connection
        .requests
        .registry
        .create(&std::env::temp_dir(), size)?
        .id;
    let release = connection.block()?;
    connection.send(1, RequestBody::EndSession(session))?;
    connection.send(2, RequestBody::Attach { session, size })?;
    connection.send(3, RequestBody::Ping)?;
    let first = connection.reply();
    release.send(())?;
    assert_eq!(first?, (RequestId(3), ReplyBody::Pong));
    assert_eq!(connection.reply()?, (RequestId(1), ReplyBody::SessionEnded));
    let (id, body) = connection.reply()?;
    assert_eq!(id, RequestId(2));
    assert!(matches!(body, ReplyBody::Error(error) if error.code == ErrorCode::UnknownSession));
    Ok(())
}

#[test]
fn queued_history_precedes_later_detach() -> TestResult {
    let mut connection = Connection::new()?;
    let size = Size { cols: 80, rows: 24 };
    let session = connection
        .requests
        .registry
        .create(&std::env::temp_dir(), size)?
        .id;
    connection.send(1, RequestBody::Attach { session, size })?;
    let (_, body) = connection.reply()?;
    let ReplyBody::Attached { snapshot, .. } = body else {
        return Err("expected attachment".into());
    };
    let release = connection.block()?;
    connection.send(
        2,
        RequestBody::HistoryPage {
            channel: snapshot.channel,
            before: HistoryCursor(0),
            max_rows: 100,
        },
    )?;
    connection.send(3, RequestBody::Detach(snapshot.channel))?;
    release.send(())?;
    let (id, body) = connection.reply()?;
    assert_eq!(id, RequestId(2));
    assert!(matches!(body, ReplyBody::HistoryPage(_)));
    assert_eq!(connection.reply()?, (RequestId(3), ReplyBody::Detached));
    Ok(())
}
