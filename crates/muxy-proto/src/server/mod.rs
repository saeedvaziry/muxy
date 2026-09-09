#[cfg(any(target_os = "macos", target_os = "linux"))]
mod session;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod unsupported;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::extension::{
    ExtensionBroadcast, ExtensionLocalEvent, InvokeOutcome, InvokeRequest, InvokeResult,
    ModalQuery, ModalResult,
};
use crate::framing::DEFAULT_MAX_INPUT_BYTES;
use crate::hook::AgentHookEvent;

pub const DEFAULT_MAX_IN_FLIGHT_COMMANDS: usize = 8;
pub const DEFAULT_INCOMING_REQUEST_CAPACITY: usize = 256;
pub const DEFAULT_DROPPED_NOTIFICATION_DISCONNECT_THRESHOLD: usize = 100;
pub const DEFAULT_INVOKE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerLimits {
    pub max_input_bytes: usize,
    pub max_in_flight_commands: usize,
    pub dropped_notification_disconnect_threshold: usize,
    pub invoke_timeout: Duration,
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_in_flight_commands: DEFAULT_MAX_IN_FLIGHT_COMMANDS,
            dropped_notification_disconnect_threshold:
                DEFAULT_DROPPED_NOTIFICATION_DISCONNECT_THRESHOLD,
            invoke_timeout: DEFAULT_INVOKE_TIMEOUT,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionAccess {
    Allowed,
    Denied(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtensionSnapshotEntry {
    pub token: String,
    pub granted_permissions: BTreeSet<String>,
    pub subscription_access: BTreeMap<String, SubscriptionAccess>,
    pub can_write_notifications: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtensionSnapshot {
    pub entries: BTreeMap<String, ExtensionSnapshotEntry>,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub socket_path: PathBuf,
    pub recognized_command_heads: HashSet<String>,
    pub no_response_command_routes: Vec<String>,
    pub limits: ServerLimits,
    pub initial_extension_snapshot: ExtensionSnapshot,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RequestOrigin {
    pub extension_id: Option<String>,
    pub granted_permissions: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyNotificationIngress {
    pub notification_type: String,
    pub pane_id: Option<String>,
    pub title: String,
    pub body: String,
    pub sender_extension_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoResponseCommand {
    pub head: String,
    pub payload: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionLocalEventIngress {
    pub extension_id: String,
    pub event: ExtensionLocalEvent,
}

pub struct AppCommandRequest {
    pub command: String,
    pub origin: RequestOrigin,
    pub responder: CommandResponder,
}

pub enum IncomingRequest {
    AppCommand(AppCommandRequest),
    NoResponseCommand(NoResponseCommand),
    LegacyNotification(LegacyNotificationIngress),
    AgentHook(AgentHookEvent),
    ExtensionLocalEvent(ExtensionLocalEventIngress),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandReply {
    pub text: String,
}

impl CommandReply {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

pub struct CommandResponder {
    request_id: u64,
    sender: Option<ControlSender>,
}

impl CommandResponder {
    pub fn respond(mut self, reply: CommandReply) {
        if let Some(sender) = self.sender.take() {
            sender.send(Control::Complete {
                request_id: self.request_id,
                reply,
            });
        }
    }
}

impl Drop for CommandResponder {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            sender.send(Control::Complete {
                request_id: self.request_id,
                reply: CommandReply::new("error:internal command handler dropped response"),
            });
        }
    }
}

#[derive(Clone)]
struct ControlSender {
    sender: mpsc::Sender<Control>,
    worker: Arc<Mutex<Option<Thread>>>,
}

impl ControlSender {
    fn send(&self, control: Control) {
        let _ = self.sender.send(control);
        if let Ok(worker) = self.worker.lock()
            && let Some(worker) = worker.as_ref()
        {
            worker.unpark();
        }
    }
}

enum Control {
    Complete {
        request_id: u64,
        reply: CommandReply,
    },
    ReplaceExtensionSnapshot {
        snapshot: ExtensionSnapshot,
        completion: mpsc::Sender<()>,
    },
    Broadcast(ExtensionBroadcast),
    PushExtensionEvent {
        extension_id: String,
        event: ExtensionLocalEvent,
    },
    PushModalResult {
        extension_id: String,
        result: ModalResult,
    },
    PushModalQuery {
        extension_id: String,
        query: ModalQuery,
    },
    Invoke {
        extension_id: String,
        request: InvokeRequest,
        completion: mpsc::Sender<InvokeOutcome>,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct SocketServerHandle {
    sender: ControlSender,
}

impl SocketServerHandle {
    pub fn replace_extension_snapshot(&self, snapshot: ExtensionSnapshot) {
        let (completion, receiver) = mpsc::channel();
        self.sender.send(Control::ReplaceExtensionSnapshot {
            snapshot,
            completion,
        });
        let _ = receiver.recv();
    }

    pub fn broadcast(&self, event: ExtensionBroadcast) {
        self.sender.send(Control::Broadcast(event));
    }

    pub fn push_extension_event(
        &self,
        extension_id: impl Into<String>,
        event: ExtensionLocalEvent,
    ) {
        self.sender.send(Control::PushExtensionEvent {
            extension_id: extension_id.into(),
            event,
        });
    }

    pub fn push_modal_result(&self, extension_id: impl Into<String>, result: ModalResult) {
        self.sender.send(Control::PushModalResult {
            extension_id: extension_id.into(),
            result,
        });
    }

    pub fn push_modal_query(&self, extension_id: impl Into<String>, query: ModalQuery) {
        self.sender.send(Control::PushModalQuery {
            extension_id: extension_id.into(),
            query,
        });
    }

    pub fn invoke(
        &self,
        extension_id: impl Into<String>,
        request: InvokeRequest,
    ) -> mpsc::Receiver<InvokeOutcome> {
        let (completion, receiver) = mpsc::channel();
        self.sender.send(Control::Invoke {
            extension_id: extension_id.into(),
            request,
            completion,
        });
        receiver
    }
}

pub struct SocketServer {
    sender: ControlSender,
    worker: Option<JoinHandle<()>>,
}

impl SocketServer {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn start(
        config: ServerConfig,
    ) -> Result<
        (
            Self,
            SocketServerHandle,
            async_channel::Receiver<IncomingRequest>,
        ),
        ServerError,
    > {
        let bound = unix::BoundSocket::bind(&config.socket_path)?;
        let (incoming_sender, incoming_receiver) =
            async_channel::bounded(DEFAULT_INCOMING_REQUEST_CAPACITY);
        let incoming_eviction_receiver = incoming_receiver.clone();
        let (control_sender, control_receiver) = mpsc::channel();
        let worker_thread = Arc::new(Mutex::new(None));
        let sender = ControlSender {
            sender: control_sender,
            worker: worker_thread.clone(),
        };
        let worker_sender = sender.clone();
        let worker = thread::Builder::new()
            .name("muxy-socket-server".to_owned())
            .spawn(move || {
                run_worker(
                    bound,
                    config,
                    incoming_sender,
                    incoming_eviction_receiver,
                    control_receiver,
                    worker_sender,
                );
            })
            .map_err(ServerError::WorkerThread)?;
        if let Ok(mut slot) = worker_thread.lock() {
            *slot = Some(worker.thread().clone());
        }
        let handle = SocketServerHandle {
            sender: sender.clone(),
        };
        Ok((
            Self {
                sender,
                worker: Some(worker),
            },
            handle,
            incoming_receiver,
        ))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn start(
        _config: ServerConfig,
    ) -> Result<
        (
            Self,
            SocketServerHandle,
            async_channel::Receiver<IncomingRequest>,
        ),
        ServerError,
    > {
        unsupported::unsupported()
    }
}

impl Drop for SocketServer {
    fn drop(&mut self) {
        self.sender.send(Control::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("socket transport is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("could not prepare socket parent {path}: {source}")]
    ParentDirectory { path: PathBuf, source: io::Error },
    #[error("could not lock socket parent {path}: {source}")]
    DirectoryLock { path: PathBuf, source: io::Error },
    #[error("socket path is not a Unix socket: {0}")]
    PathNotSocket(PathBuf),
    #[error("socket already has an active listener: {0}")]
    ActiveListener(PathBuf),
    #[error("socket probe failed for {path}: {source}")]
    SocketProbe { path: PathBuf, source: io::Error },
    #[error("socket path changed during stale probe: {0}")]
    SocketChanged(PathBuf),
    #[error("could not remove stale socket {path}: {source}")]
    RemoveStaleSocket { path: PathBuf, source: io::Error },
    #[error("could not bind socket {path}: {source}")]
    SocketBind { path: PathBuf, source: io::Error },
    #[error("could not set socket permissions for {path}: {source}")]
    SocketPermissions { path: PathBuf, source: io::Error },
    #[error("could not read socket metadata for {path}: {source}")]
    SocketMetadata { path: PathBuf, source: io::Error },
    #[error("could not set socket nonblocking for {path}: {source}")]
    SocketNonblocking { path: PathBuf, source: io::Error },
    #[error("could not start socket worker: {0}")]
    WorkerThread(io::Error),
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Clone, Copy)]
enum ReplyFraming {
    Cli,
    Extension,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct Correlation {
    session_id: u64,
    framing: ReplyFraming,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct PendingInvoke {
    owner_session_id: u64,
    deadline: Instant,
    completion: mpsc::Sender<InvokeOutcome>,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_worker(
    bound: unix::BoundSocket,
    config: ServerConfig,
    incoming_sender: async_channel::Sender<IncomingRequest>,
    incoming_eviction_receiver: async_channel::Receiver<IncomingRequest>,
    control_receiver: mpsc::Receiver<Control>,
    control_sender: ControlSender,
) {
    use std::collections::HashMap;
    use std::io::Read;

    use crate::framing::{InputRecord, MAX_READ_BYTES, frame_cli_reply};

    let mut sessions = HashMap::<u64, session::Session>::new();
    let mut correlations = HashMap::<u64, Correlation>::new();
    let mut next_session_id = 1_u64;
    let mut next_request_id = 1_u64;
    let mut extension_snapshot = config.initial_extension_snapshot.clone();
    let mut live_sessions = HashMap::<String, u64>::new();
    let mut pending_invokes = HashMap::<String, PendingInvoke>::new();
    let mut recent_hook_ids = crate::hook::RecentAgentHookEventIds::default();

    loop {
        let mut shutdown = false;
        while let Ok(control) = control_receiver.try_recv() {
            match control {
                Control::Complete { request_id, reply } => {
                    let Some(correlation) = correlations.remove(&request_id) else {
                        continue;
                    };
                    let Some(session) = sessions.get_mut(&correlation.session_id) else {
                        continue;
                    };
                    session.pending_requests = session.pending_requests.saturating_sub(1);
                    match correlation.framing {
                        ReplyFraming::Cli => {
                            session.enqueue(frame_cli_reply(&reply.text));
                            session.close_after_flush = true;
                        }
                        ReplyFraming::Extension => {
                            session.enqueue(format!("{}\n", reply.text).bytes());
                        }
                    }
                }
                Control::ReplaceExtensionSnapshot {
                    snapshot,
                    completion,
                } => {
                    replace_snapshot(
                        snapshot,
                        &mut extension_snapshot,
                        &mut sessions,
                        &mut live_sessions,
                        &mut pending_invokes,
                    );
                    let _ = completion.send(());
                }
                Control::Broadcast(event) => {
                    let line = format!("{}\n", event.encode());
                    for session in sessions.values_mut().filter(|session| {
                        !session.read_eof && session.subscriptions.contains(&event.name)
                    }) {
                        session.enqueue(line.bytes());
                    }
                }
                Control::PushExtensionEvent {
                    extension_id,
                    event,
                } => {
                    if extension_snapshot.entries.contains_key(&extension_id)
                        && let Some(session) =
                            live_session_mut(&extension_id, &live_sessions, &mut sessions)
                        && let Some(line) = event.encode()
                    {
                        session.enqueue(format!("{line}\n").bytes());
                    }
                }
                Control::PushModalResult {
                    extension_id,
                    result,
                } => {
                    if let Some(session) =
                        live_session_mut(&extension_id, &live_sessions, &mut sessions)
                        && let Some(line) = result.encode()
                    {
                        session.enqueue(format!("{line}\n").bytes());
                    }
                }
                Control::PushModalQuery {
                    extension_id,
                    query,
                } => {
                    if let Some(session) =
                        live_session_mut(&extension_id, &live_sessions, &mut sessions)
                        && let Some(line) = query.encode()
                    {
                        session.enqueue(format!("{line}\n").bytes());
                    }
                }
                Control::Invoke {
                    extension_id,
                    request,
                    completion,
                } => {
                    if pending_invokes.contains_key(&request.call_id) {
                        let _ = completion
                            .send(InvokeOutcome::Error("duplicate invoke call ID".to_owned()));
                    } else if let Some(session_id) = live_sessions.get(&extension_id).copied()
                        && let Some(session) = sessions
                            .get_mut(&session_id)
                            .filter(|session| !session.read_eof)
                        && let Some(line) = request.encode()
                    {
                        pending_invokes.insert(
                            request.call_id.clone(),
                            PendingInvoke {
                                owner_session_id: session_id,
                                deadline: Instant::now() + config.limits.invoke_timeout,
                                completion,
                            },
                        );
                        session.enqueue(format!("{line}\n").bytes());
                    } else {
                        let _ = completion.send(InvokeOutcome::Unavailable);
                    }
                }
                Control::Shutdown => shutdown = true,
            }
        }
        if shutdown {
            fail_all_invokes(&mut pending_invokes, InvokeOutcome::Unavailable);
            break;
        }

        loop {
            match bound.listener.accept() {
                Ok((stream, _)) => {
                    if unix::prepare_stream(&stream).is_err() {
                        continue;
                    }
                    let id = next_session_id;
                    next_session_id = next_session_id.wrapping_add(1).max(1);
                    sessions.insert(
                        id,
                        session::Session::new(stream, config.limits.max_input_bytes),
                    );
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let session_ids = sessions.keys().copied().collect::<Vec<_>>();
        let mut records = Vec::<(u64, InputRecord)>::new();
        let mut eof_sessions = Vec::new();
        for session_id in session_ids {
            let Some(session) = sessions.get_mut(&session_id) else {
                continue;
            };
            let was_eof = session.read_eof;
            loop {
                let mut bytes = [0_u8; MAX_READ_BYTES];
                match session.stream.read(&mut bytes) {
                    Ok(0) => {
                        session.read_eof = true;
                        break;
                    }
                    Ok(count) => match session.input.push(&bytes[..count]) {
                        Ok(incoming) => {
                            records.extend(incoming.into_iter().map(|record| (session_id, record)));
                        }
                        Err(_) => {
                            session.read_eof = true;
                            break;
                        }
                    },
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        session.read_eof = true;
                        break;
                    }
                }
            }
            if !was_eof && session.read_eof {
                eof_sessions.push(session_id);
            }
        }
        for (session_id, record) in records {
            route_record(
                session_id,
                record,
                &mut RouteContext {
                    config: &config,
                    incoming_sender: &incoming_sender,
                    incoming_eviction_receiver: &incoming_eviction_receiver,
                    control_sender: &control_sender,
                    sessions: &mut sessions,
                    correlations: &mut correlations,
                    next_request_id: &mut next_request_id,
                    extension_snapshot: &extension_snapshot,
                    live_sessions: &mut live_sessions,
                    pending_invokes: &mut pending_invokes,
                    recent_hook_ids: &mut recent_hook_ids,
                },
            );
        }

        for session_id in eof_sessions {
            detach_session(
                session_id,
                &mut sessions,
                &mut live_sessions,
                &mut pending_invokes,
            );
        }

        let now = Instant::now();
        let expired = pending_invokes
            .iter()
            .filter(|(_, invoke)| invoke.deadline <= now)
            .map(|(call_id, _)| call_id.clone())
            .collect::<Vec<_>>();
        for call_id in expired {
            if let Some(invoke) = pending_invokes.remove(&call_id) {
                let _ = invoke.completion.send(InvokeOutcome::Timeout);
            }
        }

        let session_ids = sessions.keys().copied().collect::<Vec<_>>();
        let mut remove = Vec::new();
        for session_id in session_ids {
            let Some(session) = sessions.get_mut(&session_id) else {
                continue;
            };
            let mut failed = false;
            while !session.writes.is_empty() {
                let bytes = session.writes.make_contiguous();
                match unix::send_no_sigpipe(&session.stream, bytes) {
                    Ok(0) => {
                        failed = true;
                        break;
                    }
                    Ok(count) => {
                        session.writes.drain(..count);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            if failed || session.can_close() {
                remove.push(session_id);
            }
        }
        for session_id in remove {
            detach_session(
                session_id,
                &mut sessions,
                &mut live_sessions,
                &mut pending_invokes,
            );
            sessions.remove(&session_id);
            correlations.retain(|_, correlation| correlation.session_id != session_id);
        }

        thread::park_timeout(Duration::from_millis(2));
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn live_session_mut<'a>(
    extension_id: &str,
    live_sessions: &std::collections::HashMap<String, u64>,
    sessions: &'a mut std::collections::HashMap<u64, session::Session>,
) -> Option<&'a mut session::Session> {
    let session_id = live_sessions.get(extension_id)?;
    sessions
        .get_mut(session_id)
        .filter(|session| !session.read_eof)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn fail_all_invokes(
    pending_invokes: &mut std::collections::HashMap<String, PendingInvoke>,
    outcome: InvokeOutcome,
) {
    for (_, invoke) in pending_invokes.drain() {
        let _ = invoke.completion.send(outcome.clone());
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn fail_session_invokes(
    session_id: u64,
    pending_invokes: &mut std::collections::HashMap<String, PendingInvoke>,
) {
    let call_ids = pending_invokes
        .iter()
        .filter(|(_, invoke)| invoke.owner_session_id == session_id)
        .map(|(call_id, _)| call_id.clone())
        .collect::<Vec<_>>();
    for call_id in call_ids {
        if let Some(invoke) = pending_invokes.remove(&call_id) {
            let _ = invoke.completion.send(InvokeOutcome::Unavailable);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn detach_session(
    session_id: u64,
    sessions: &mut std::collections::HashMap<u64, session::Session>,
    live_sessions: &mut std::collections::HashMap<String, u64>,
    pending_invokes: &mut std::collections::HashMap<String, PendingInvoke>,
) {
    if let Some(session) = sessions.get_mut(&session_id) {
        session.subscriptions.clear();
        if let Some(extension_id) = session.extension_id.as_ref()
            && live_sessions.get(extension_id) == Some(&session_id)
        {
            live_sessions.remove(extension_id);
        }
    }
    fail_session_invokes(session_id, pending_invokes);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn replace_snapshot(
    snapshot: ExtensionSnapshot,
    current: &mut ExtensionSnapshot,
    sessions: &mut std::collections::HashMap<u64, session::Session>,
    live_sessions: &mut std::collections::HashMap<String, u64>,
    pending_invokes: &mut std::collections::HashMap<String, PendingInvoke>,
) {
    let session_ids = sessions.keys().copied().collect::<Vec<_>>();
    for session_id in session_ids {
        let authorization_changed = sessions
            .get(&session_id)
            .and_then(|session| session.extension_id.as_ref())
            .is_some_and(|extension_id| {
                current.entries.get(extension_id).map(|entry| &entry.token)
                    != snapshot.entries.get(extension_id).map(|entry| &entry.token)
            });
        if authorization_changed {
            detach_session(session_id, sessions, live_sessions, pending_invokes);
            if let Some(session) = sessions.get_mut(&session_id) {
                session.extension_id = None;
            }
        }
    }
    *current = snapshot;
    let session_ids = sessions.keys().copied().collect::<Vec<_>>();
    for session_id in session_ids {
        let removed = sessions
            .get(&session_id)
            .and_then(|session| session.extension_id.as_ref())
            .is_some_and(|extension_id| !current.entries.contains_key(extension_id));
        if removed {
            detach_session(session_id, sessions, live_sessions, pending_invokes);
            if let Some(session) = sessions.get_mut(&session_id) {
                session.extension_id = None;
            }
            continue;
        }
        if let Some(session) = sessions.get_mut(&session_id)
            && let Some(extension_id) = session.extension_id.as_ref()
            && let Some(entry) = current.entries.get(extension_id)
        {
            session.subscriptions.retain(|event| {
                matches!(
                    entry.subscription_access.get(event),
                    Some(SubscriptionAccess::Allowed)
                )
            });
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct RouteContext<'a> {
    config: &'a ServerConfig,
    incoming_sender: &'a async_channel::Sender<IncomingRequest>,
    incoming_eviction_receiver: &'a async_channel::Receiver<IncomingRequest>,
    control_sender: &'a ControlSender,
    sessions: &'a mut std::collections::HashMap<u64, session::Session>,
    correlations: &'a mut std::collections::HashMap<u64, Correlation>,
    next_request_id: &'a mut u64,
    extension_snapshot: &'a ExtensionSnapshot,
    live_sessions: &'a mut std::collections::HashMap<String, u64>,
    pending_invokes: &'a mut std::collections::HashMap<String, PendingInvoke>,
    recent_hook_ids: &'a mut crate::hook::RecentAgentHookEventIds,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn send_incoming(request: IncomingRequest, context: &RouteContext<'_>) {
    match context.incoming_sender.try_send(request) {
        Ok(()) | Err(async_channel::TrySendError::Closed(_)) => {}
        Err(async_channel::TrySendError::Full(request)) => {
            let _ = context.incoming_eviction_receiver.try_recv();
            let _ = context.incoming_sender.try_send(request);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn route_record(
    session_id: u64,
    record: crate::framing::InputRecord,
    context: &mut RouteContext<'_>,
) {
    let trimmed = record.trimmed();
    let unidentified = context
        .sessions
        .get(&session_id)
        .is_some_and(|session| session.extension_id.is_none());
    if unidentified && let Ok(event) = crate::hook::parse_agent_hook_event(trimmed) {
        if let Some(session) = context.sessions.get_mut(&session_id)
            && let Ok(bytes) = crate::hook::encode_agent_hook_acknowledgement(
                &crate::hook::AgentHookAcknowledgement::success(),
            )
        {
            session.enqueue(bytes);
        }
        if context
            .recent_hook_ids
            .register_and_check_is_first_delivery(event.id.as_deref())
        {
            send_incoming(IncomingRequest::AgentHook(event), context);
        }
        return;
    }

    let head = trimmed.split_once('|').map_or(trimmed, |(head, _)| head);
    if matches!(head, "identify" | "subscribe") {
        let reply = evaluate_sticky(session_id, head, trimmed, context);
        if let Some(session) = context.sessions.get_mut(&session_id) {
            session.enqueue(format!("{reply}\n").bytes());
        }
        return;
    }

    if context.config.recognized_command_heads.contains(head) {
        let Some(session) = context.sessions.get_mut(&session_id) else {
            return;
        };
        let (framing, origin) = if let Some(extension_id) = session.extension_id.as_ref() {
            let grants = context
                .extension_snapshot
                .entries
                .get(extension_id)
                .map(|entry| entry.granted_permissions.clone())
                .unwrap_or_default();
            (
                ReplyFraming::Extension,
                RequestOrigin {
                    extension_id: Some(extension_id.clone()),
                    granted_permissions: grants,
                },
            )
        } else {
            (ReplyFraming::Cli, RequestOrigin::default())
        };
        if session.pending_requests >= context.config.limits.max_in_flight_commands {
            match framing {
                ReplyFraming::Cli => {
                    session.enqueue(crate::framing::frame_cli_reply(
                        "error:too many concurrent commands",
                    ));
                    session.close_after_flush = true;
                }
                ReplyFraming::Extension => {
                    session.enqueue(b"error:too many concurrent commands\n".iter().copied());
                }
            }
            return;
        }
        let request_id = *context.next_request_id;
        *context.next_request_id = context.next_request_id.wrapping_add(1).max(1);
        session.pending_requests += 1;
        context.correlations.insert(
            request_id,
            Correlation {
                session_id,
                framing,
            },
        );
        send_incoming(
            IncomingRequest::AppCommand(AppCommandRequest {
                command: trimmed.to_owned(),
                origin,
                responder: CommandResponder {
                    request_id,
                    sender: Some(context.control_sender.clone()),
                },
            }),
            context,
        );
        return;
    }

    if head == crate::extension::INVOKE_RESULT_HEAD {
        if let Some(result) = InvokeResult::parse(trimmed)
            && context
                .pending_invokes
                .get(&result.call_id)
                .is_some_and(|invoke| invoke.owner_session_id == session_id)
            && let Some(invoke) = context.pending_invokes.remove(&result.call_id)
        {
            let _ = invoke.completion.send(result.outcome());
        }
        return;
    }

    if head == crate::extension::EXTENSION_LOCAL_EVENT_HEAD {
        let response = match context
            .sessions
            .get(&session_id)
            .and_then(|session| session.extension_id.clone())
        {
            None => "error:identify required".to_owned(),
            Some(extension_id)
                if !context
                    .extension_snapshot
                    .entries
                    .contains_key(&extension_id) =>
            {
                format!("error:extension {extension_id} is no longer loaded")
            }
            Some(extension_id) => match ExtensionLocalEvent::parse(trimmed) {
                Some(event) => {
                    send_incoming(
                        IncomingRequest::ExtensionLocalEvent(ExtensionLocalEventIngress {
                            extension_id,
                            event,
                        }),
                        context,
                    );
                    "ok".to_owned()
                }
                None => "error:invalid extension event".to_owned(),
            },
        };
        if let Some(session) = context.sessions.get_mut(&session_id) {
            session.enqueue(format!("{response}\n").bytes());
        }
        return;
    }

    for route in &context.config.no_response_command_routes {
        let prefix = format!("{route}|");
        if let Some(payload) = record.original().strip_prefix(&prefix) {
            send_incoming(
                IncomingRequest::NoResponseCommand(NoResponseCommand {
                    head: route.clone(),
                    payload: payload.to_owned(),
                }),
                context,
            );
            return;
        }
    }

    let Some(mut notification) = parse_legacy_notification(record.original()) else {
        return;
    };
    let mut disconnect = false;
    if let Some(session) = context.sessions.get_mut(&session_id)
        && let Some(extension_id) = session.extension_id.as_ref()
    {
        notification.sender_extension_id = Some(extension_id.clone());
        let allowed = context
            .extension_snapshot
            .entries
            .get(extension_id)
            .is_some_and(|entry| entry.can_write_notifications);
        if !allowed {
            session.dropped_notifications += 1;
            disconnect = session.dropped_notifications
                >= context
                    .config
                    .limits
                    .dropped_notification_disconnect_threshold;
            if disconnect {
                session.read_eof = true;
            }
        } else {
            send_incoming(IncomingRequest::LegacyNotification(notification), context);
        }
        if !allowed && disconnect {
            detach_session(
                session_id,
                context.sessions,
                context.live_sessions,
                context.pending_invokes,
            );
        }
    } else {
        send_incoming(IncomingRequest::LegacyNotification(notification), context);
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn evaluate_sticky(
    session_id: u64,
    head: &str,
    message: &str,
    context: &mut RouteContext<'_>,
) -> String {
    match head {
        "identify" => {
            let parts = message.splitn(3, '|').collect::<Vec<_>>();
            if parts.len() < 2 || parts[1].is_empty() {
                return "error:usage identify|<extension-id>|<token>".to_owned();
            }
            let extension_id = parts[1];
            let Some(entry) = context.extension_snapshot.entries.get(extension_id) else {
                return format!("error:unknown extension {extension_id}");
            };
            let token = parts.get(2).copied().unwrap_or_default();
            if entry.token.is_empty() || token != entry.token {
                return "error:invalid extension token".to_owned();
            }
            if let Some(session) = context.sessions.get_mut(&session_id)
                && let Some(previous) = session.extension_id.replace(extension_id.to_owned())
                && context.live_sessions.get(&previous) == Some(&session_id)
            {
                context.live_sessions.remove(&previous);
            }
            context
                .live_sessions
                .insert(extension_id.to_owned(), session_id);
            "ok".to_owned()
        }
        "subscribe" => {
            let parts = message.splitn(2, '|').collect::<Vec<_>>();
            if parts.len() != 2 || parts[1].is_empty() {
                return "error:usage subscribe|<event>".to_owned();
            }
            let event = parts[1];
            if let Some(extension_id) = context
                .sessions
                .get(&session_id)
                .and_then(|session| session.extension_id.as_ref())
            {
                let Some(entry) = context.extension_snapshot.entries.get(extension_id) else {
                    return format!("error:extension {extension_id} is no longer loaded");
                };
                match entry.subscription_access.get(event) {
                    None => return format!("error:event {event} not declared in manifest"),
                    Some(SubscriptionAccess::Denied(error)) => {
                        return format!("error:{error}");
                    }
                    Some(SubscriptionAccess::Allowed) => {}
                }
            }
            if let Some(session) = context.sessions.get_mut(&session_id) {
                session.subscriptions.insert(event.to_owned());
            }
            "ok".to_owned()
        }
        _ => format!("error:unknown sticky command {head}"),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn parse_legacy_notification(message: &str) -> Option<LegacyNotificationIngress> {
    let parts = message.splitn(4, '|').collect::<Vec<_>>();
    if parts.len() < 3 {
        return None;
    }
    Some(LegacyNotificationIngress {
        notification_type: parts[0].to_owned(),
        pane_id: (!parts[1].is_empty()).then(|| parts[1].to_owned()),
        title: if parts[2].is_empty() {
            "Task completed!".to_owned()
        } else {
            parts[2].to_owned()
        },
        body: parts.get(3).copied().unwrap_or_default().to_owned(),
        sender_extension_id: None,
    })
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use std::fs;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::Shutdown;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;

    fn directory() -> TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target/test-verification");
        fs::create_dir_all(&root).unwrap();
        TempDir::new_in(root.canonicalize().unwrap()).unwrap()
    }

    fn config(path: &Path) -> ServerConfig {
        ServerConfig {
            socket_path: path.to_path_buf(),
            recognized_command_heads: HashSet::from(["sample-command".to_owned()]),
            no_response_command_routes: vec!["sample-ingress".to_owned()],
            limits: ServerLimits::default(),
            initial_extension_snapshot: ExtensionSnapshot::default(),
        }
    }

    fn start(
        path: &Path,
    ) -> (
        SocketServer,
        SocketServerHandle,
        async_channel::Receiver<IncomingRequest>,
    ) {
        SocketServer::start(config(path)).unwrap()
    }

    fn connect(path: &Path) -> UnixStream {
        for _ in 0..100 {
            match UnixStream::connect(path) {
                Ok(stream) => return stream,
                Err(_) => thread::sleep(Duration::from_millis(2)),
            }
        }
        panic!("socket did not accept connections")
    }

    fn read_all(mut stream: UnixStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn extension_entry(can_write_notifications: bool) -> ExtensionSnapshotEntry {
        ExtensionSnapshotEntry {
            token: "secret".to_owned(),
            granted_permissions: BTreeSet::from(["panes:read".to_owned()]),
            subscription_access: BTreeMap::from([
                ("allowed".to_owned(), SubscriptionAccess::Allowed),
                (
                    "denied".to_owned(),
                    SubscriptionAccess::Denied("permission denied (events:read)".to_owned()),
                ),
            ]),
            can_write_notifications,
        }
    }

    fn extension_config(path: &Path, can_write_notifications: bool) -> ServerConfig {
        let mut server_config = config(path);
        server_config.initial_extension_snapshot = ExtensionSnapshot {
            entries: BTreeMap::from([(
                "sample.extension".to_owned(),
                extension_entry(can_write_notifications),
            )]),
        };
        server_config
    }

    fn read_line(reader: &mut BufReader<UnixStream>) -> String {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        line
    }

    fn identify(writer: &mut UnixStream, reader: &mut BufReader<UnixStream>) {
        writer
            .write_all(b"identify|sample.extension|secret\n")
            .unwrap();
        assert_eq!(read_line(reader), "ok\n");
    }

    #[test]
    fn binds_owner_only_socket_and_cleans_up_after_drop() {
        let directory = directory();
        let parent = directory.path().join("support");
        let path = parent.join("main.sock");
        let (server, _, _) = start(&path);
        let metadata = fs::symlink_metadata(&path).unwrap();
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o700
        );
        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn preserves_permissions_of_a_preexisting_parent() {
        let directory = directory();
        let parent = directory.path().join("support");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        let path = parent.join("main.sock");
        let (server, _, _) = start(&path);
        assert_eq!(
            fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o755
        );
        drop(server);
    }

    #[test]
    fn refuses_an_active_listener_without_changing_its_identity() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (server, _, _) = start(&path);
        let before = fs::symlink_metadata(&path).unwrap();
        let error = SocketServer::start(config(&path)).err().unwrap();
        assert!(matches!(error, ServerError::ActiveListener(_)));
        let after = fs::symlink_metadata(&path).unwrap();
        assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
        drop(server);
    }

    #[test]
    fn concurrent_binders_produce_one_owner_and_one_refusal() {
        let directory = directory();
        let path = Arc::new(directory.path().join("main.sock"));
        let barrier = Arc::new(Barrier::new(2));
        let threads = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    SocketServer::start(config(&path))
                })
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(ServerError::ActiveListener(_))))
                .count(),
            1
        );
        drop(results);
        assert!(!path.exists());
    }

    #[test]
    fn replaces_only_a_conclusively_stale_socket() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let stale = UnixListener::bind(&path).unwrap();
        let stale_identity = fs::symlink_metadata(&path).unwrap().ino();
        drop(stale);
        let (server, _, _) = start(&path);
        assert_ne!(fs::symlink_metadata(&path).unwrap().ino(), stale_identity);
        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn refuses_a_regular_file_without_removing_it() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        fs::write(&path, b"keep").unwrap();
        let error = SocketServer::start(config(&path)).err().unwrap();
        assert!(matches!(error, ServerError::PathNotSocket(_)));
        assert_eq!(fs::read(&path).unwrap(), b"keep");
    }

    #[test]
    fn cleanup_leaves_a_replacement_path_untouched() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (server, _, _) = start(&path);
        fs::remove_file(&path).unwrap();
        let mut replacement = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        replacement.write_all(b"replacement").unwrap();
        drop(replacement);
        drop(server);
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
    }

    #[test]
    fn command_replies_are_nul_terminated_then_closed_after_half_close() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = start(&path);
        let mut client = connect(&path);
        client.write_all(b"sample-command|payload\n").unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let request = incoming.recv_blocking().unwrap();
        let IncomingRequest::AppCommand(request) = request else {
            panic!("expected app command")
        };
        assert_eq!(request.command, "sample-command|payload");
        assert_eq!(request.origin, RequestOrigin::default());
        request.responder.respond(CommandReply::new("ok"));
        assert_eq!(read_all(client), b"ok\0");
    }

    #[test]
    fn command_limit_returns_the_retained_error_without_dispatching_overflow() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = SocketServer::start(config(&path)).unwrap();
        let mut client = connect(&path);
        for index in 0..=DEFAULT_MAX_IN_FLIGHT_COMMANDS {
            writeln!(client, "sample-command|{index}").unwrap();
        }
        client.shutdown(Shutdown::Write).unwrap();
        let requests = (0..DEFAULT_MAX_IN_FLIGHT_COMMANDS)
            .map(|_| {
                let IncomingRequest::AppCommand(request) = incoming.recv_blocking().unwrap() else {
                    panic!("expected app command")
                };
                request
            })
            .collect::<Vec<_>>();
        assert!(incoming.try_recv().is_err());
        for request in requests {
            request.responder.respond(CommandReply::new("ok"));
        }
        let mut expected = b"error:too many concurrent commands\0".to_vec();
        for _ in 0..DEFAULT_MAX_IN_FLIGHT_COMMANDS {
            expected.extend_from_slice(b"ok\0");
        }
        assert_eq!(read_all(client), expected);
    }

    #[test]
    fn dropped_responder_returns_an_internal_error_instead_of_hanging() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = start(&path);
        let mut client = connect(&path);
        client.write_all(b"sample-command\n").unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let request = incoming.recv_blocking().unwrap();
        let IncomingRequest::AppCommand(request) = request else {
            panic!("expected app command")
        };
        drop(request.responder);
        assert_eq!(
            read_all(client),
            b"error:internal command handler dropped response\0"
        );
    }

    #[test]
    fn a_peer_that_closes_before_its_reply_does_not_stop_the_server() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = start(&path);
        let mut first = connect(&path);
        first.write_all(b"sample-command\n").unwrap();
        let first_request = incoming.recv_blocking().unwrap();
        drop(first);
        let IncomingRequest::AppCommand(first_request) = first_request else {
            panic!("expected app command")
        };
        first_request
            .responder
            .respond(CommandReply::new("ignored"));
        thread::sleep(Duration::from_millis(10));

        let mut second = connect(&path);
        second.write_all(b"sample-command\n").unwrap();
        let second_request = incoming.recv_blocking().unwrap();
        let IncomingRequest::AppCommand(second_request) = second_request else {
            panic!("expected app command")
        };
        second_request.responder.respond(CommandReply::new("ok"));
        assert_eq!(read_all(second), b"ok\0");
    }

    #[test]
    fn no_response_routes_preserve_the_original_payload() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = start(&path);
        let mut client = connect(&path);
        client
            .write_all(b"  type|pane|title\nsample-ingress|path|with|pipes\n")
            .unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let first = incoming.recv_blocking().unwrap();
        assert!(matches!(first, IncomingRequest::LegacyNotification(_)));
        let second = incoming.recv_blocking().unwrap();
        let IncomingRequest::NoResponseCommand(command) = second else {
            panic!("expected no response command")
        };
        assert_eq!(command.head, "sample-ingress");
        assert_eq!(command.payload, "path|with|pipes");
        assert!(read_all(client).is_empty());
    }

    #[test]
    fn legacy_notifications_use_max_three_splits_and_default_title() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = start(&path);
        let mut client = connect(&path);
        client
            .write_all(b"finished|PANE||body|with|pipes\n")
            .unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let request = incoming.recv_blocking().unwrap();
        let IncomingRequest::LegacyNotification(notification) = request else {
            panic!("expected legacy notification")
        };
        assert_eq!(
            notification,
            LegacyNotificationIngress {
                notification_type: "finished".to_owned(),
                pane_id: Some("PANE".to_owned()),
                title: "Task completed!".to_owned(),
                body: "body|with|pipes".to_owned(),
                sender_extension_id: None
            }
        );
        assert!(read_all(client).is_empty());
    }

    #[test]
    fn incoming_bridge_is_bounded_and_evicts_the_oldest_request() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = start(&path);
        let mut client = connect(&path);
        for index in 0..=DEFAULT_INCOMING_REQUEST_CAPACITY {
            writeln!(client, "finished|PANE|Title {index}|Body").unwrap();
        }
        client.shutdown(Shutdown::Write).unwrap();
        assert!(read_all(client).is_empty());
        assert_eq!(incoming.len(), DEFAULT_INCOMING_REQUEST_CAPACITY);
        let first = incoming.recv_blocking().unwrap();
        let IncomingRequest::LegacyNotification(first) = first else {
            panic!("expected legacy notification")
        };
        assert_eq!(first.title, "Title 1");
    }

    #[test]
    fn replacement_snapshots_do_not_interrupt_cli_commands() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, handle, incoming) = start(&path);
        let mut client = connect(&path);
        client.write_all(b"sample-command\n").unwrap();
        let request = incoming.recv_blocking().unwrap();
        handle.replace_extension_snapshot(ExtensionSnapshot::default());
        let IncomingRequest::AppCommand(request) = request else {
            panic!("expected app command")
        };
        request.responder.respond(CommandReply::new("ok"));
        assert_eq!(read_all(client), b"ok\0");
    }

    #[test]
    fn sticky_sessions_apply_identity_access_framing_and_typed_local_events() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let mut server_config = extension_config(&path, true);
        server_config.initial_extension_snapshot.entries.insert(
            "empty.extension".to_owned(),
            ExtensionSnapshotEntry::default(),
        );
        let (_server, handle, incoming) = SocketServer::start(server_config).unwrap();
        let mut writer = connect(&path);
        let mut reader = BufReader::new(writer.try_clone().unwrap());

        writer.write_all(b"identify||secret\n").unwrap();
        assert_eq!(
            read_line(&mut reader),
            "error:usage identify|<extension-id>|<token>\n"
        );
        writer.write_all(b"identify|missing|secret\n").unwrap();
        assert_eq!(read_line(&mut reader), "error:unknown extension missing\n");
        writer
            .write_all(b"identify|sample.extension|wrong\n")
            .unwrap();
        assert_eq!(read_line(&mut reader), "error:invalid extension token\n");
        writer.write_all(b"identify|empty.extension|\n").unwrap();
        assert_eq!(read_line(&mut reader), "error:invalid extension token\n");
        writer.write_all(b"subscribe|unidentified\n").unwrap();
        assert_eq!(read_line(&mut reader), "ok\n");
        identify(&mut writer, &mut reader);

        writer.write_all(b"subscribe|missing\n").unwrap();
        assert_eq!(
            read_line(&mut reader),
            "error:event missing not declared in manifest\n"
        );
        writer.write_all(b"subscribe|denied\n").unwrap();
        assert_eq!(
            read_line(&mut reader),
            "error:permission denied (events:read)\n"
        );
        writer.write_all(b"subscribe|allowed\n").unwrap();
        assert_eq!(read_line(&mut reader), "ok\n");

        writer.write_all(b"sample-command|payload\n").unwrap();
        let IncomingRequest::AppCommand(request) = incoming.recv_blocking().unwrap() else {
            panic!("expected app command")
        };
        assert_eq!(
            request.origin,
            RequestOrigin {
                extension_id: Some("sample.extension".to_owned()),
                granted_permissions: BTreeSet::from(["panes:read".to_owned()])
            }
        );
        request.responder.respond(CommandReply::new("command-ok"));
        assert_eq!(read_line(&mut reader), "command-ok\n");

        let local = ExtensionLocalEvent {
            name: "extension.ready".to_owned(),
            payload: b"local".to_vec(),
        };
        writeln!(writer, "{}", local.encode().unwrap()).unwrap();
        let IncomingRequest::ExtensionLocalEvent(ingress) = incoming.recv_blocking().unwrap()
        else {
            panic!("expected local event")
        };
        assert_eq!(ingress.extension_id, "sample.extension");
        assert_eq!(ingress.event, local);
        assert_eq!(read_line(&mut reader), "ok\n");

        writer.write_all(b"finished|pane|Title|Body\n").unwrap();
        let IncomingRequest::LegacyNotification(notification) = incoming.recv_blocking().unwrap()
        else {
            panic!("expected identified notification")
        };
        assert_eq!(
            notification.sender_extension_id.as_deref(),
            Some("sample.extension")
        );

        handle.broadcast(ExtensionBroadcast {
            name: "allowed".to_owned(),
            payload: BTreeMap::from([("z".to_owned(), "value".to_owned())]),
        });
        assert_eq!(read_line(&mut reader), "event|allowed|z=value\n");
        handle.push_extension_event(
            "sample.extension",
            ExtensionLocalEvent {
                name: "extension.pushed".to_owned(),
                payload: b"push".to_vec(),
            },
        );
        assert_eq!(
            read_line(&mut reader),
            "extension-event|ZXh0ZW5zaW9uLnB1c2hlZA==|cHVzaA==\n"
        );
        handle.push_modal_result(
            "sample.extension",
            ModalResult {
                request_id: "result".to_owned(),
                payload: b"done".to_vec(),
            },
        );
        assert_eq!(read_line(&mut reader), "modal-result|result|ZG9uZQ==\n");
        handle.push_modal_query(
            "sample.extension",
            ModalQuery {
                request_id: "query".to_owned(),
                query_id: 7,
                query: "ready?".to_owned(),
                options: BTreeMap::new(),
            },
        );
        assert_eq!(read_line(&mut reader), "modal-query|query|7|cmVhZHk/\n");
    }

    #[test]
    fn local_events_retain_their_identified_extension_isolation() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let mut server_config = extension_config(&path, true);
        let mut second = extension_entry(true);
        second.token = "other-secret".to_owned();
        server_config
            .initial_extension_snapshot
            .entries
            .insert("other.extension".to_owned(), second);
        let (_server, _, incoming) = SocketServer::start(server_config).unwrap();
        for (extension_id, token) in [
            ("sample.extension", "secret"),
            ("other.extension", "other-secret"),
        ] {
            let mut writer = connect(&path);
            let mut reader = BufReader::new(writer.try_clone().unwrap());
            writeln!(writer, "identify|{extension_id}|{token}").unwrap();
            assert_eq!(read_line(&mut reader), "ok\n");
            writer
                .write_all(b"extension-event|ZXh0ZW5zaW9uLnJlYWR5|cGF5bG9hZA==\n")
                .unwrap();
            let IncomingRequest::ExtensionLocalEvent(event) = incoming.recv_blocking().unwrap()
            else {
                panic!("expected extension event")
            };
            assert_eq!(event.extension_id, extension_id);
            assert_eq!(event.event.payload, b"payload");
            assert_eq!(read_line(&mut reader), "ok\n");
        }
    }

    #[test]
    fn replacement_filters_subscriptions_and_clears_removed_identities() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, handle, incoming) =
            SocketServer::start(extension_config(&path, true)).unwrap();
        let mut writer = connect(&path);
        let mut reader = BufReader::new(writer.try_clone().unwrap());
        identify(&mut writer, &mut reader);
        writer.write_all(b"subscribe|allowed\n").unwrap();
        assert_eq!(read_line(&mut reader), "ok\n");

        let mut replacement = extension_entry(true);
        replacement.subscription_access.clear();
        handle.replace_extension_snapshot(ExtensionSnapshot {
            entries: BTreeMap::from([("sample.extension".to_owned(), replacement)]),
        });
        handle.broadcast(ExtensionBroadcast {
            name: "allowed".to_owned(),
            payload: BTreeMap::new(),
        });
        writer.write_all(b"subscribe|allowed\n").unwrap();
        assert_eq!(
            read_line(&mut reader),
            "error:event allowed not declared in manifest\n"
        );

        handle.replace_extension_snapshot(ExtensionSnapshot::default());
        writer.write_all(b"sample-command\n").unwrap();
        writer.shutdown(Shutdown::Write).unwrap();
        let IncomingRequest::AppCommand(request) = incoming.recv_blocking().unwrap() else {
            panic!("expected app command")
        };
        assert_eq!(request.origin, RequestOrigin::default());
        request.responder.respond(CommandReply::new("cli"));
        assert_eq!(read_all(reader.into_inner()), b"cli\0");
    }

    #[test]
    fn snapshot_token_rotation_requires_identification_with_the_new_token() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, handle, _) = SocketServer::start(extension_config(&path, true)).unwrap();
        let mut writer = connect(&path);
        let mut reader = BufReader::new(writer.try_clone().unwrap());
        identify(&mut writer, &mut reader);

        let mut replacement = extension_entry(true);
        replacement.token = "rotated-secret".to_owned();
        handle.replace_extension_snapshot(ExtensionSnapshot {
            entries: BTreeMap::from([("sample.extension".to_owned(), replacement)]),
        });

        writer
            .write_all(b"identify|sample.extension|secret\n")
            .unwrap();
        assert_eq!(read_line(&mut reader), "error:invalid extension token\n");
        writer
            .write_all(b"identify|sample.extension|rotated-secret\n")
            .unwrap();
        assert_eq!(read_line(&mut reader), "ok\n");
    }

    #[test]
    fn invokes_enforce_newest_owner_result_owner_timeout_disconnect_and_removal() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let mut server_config = extension_config(&path, true);
        server_config.limits.invoke_timeout = Duration::from_millis(20);
        let (_server, handle, _) = SocketServer::start(server_config).unwrap();
        let mut first = connect(&path);
        let mut first_reader = BufReader::new(first.try_clone().unwrap());
        identify(&mut first, &mut first_reader);
        let mut newest = connect(&path);
        let mut newest_reader = BufReader::new(newest.try_clone().unwrap());
        identify(&mut newest, &mut newest_reader);

        let request = InvokeRequest::new("sample.action", b"payload".to_vec());
        let call_id = request.call_id.clone();
        let result = handle.invoke("sample.extension", request);
        assert_eq!(
            read_line(&mut newest_reader),
            format!("invoke|{call_id}|sample.action|cGF5bG9hZA==\n")
        );
        writeln!(first, "invoke-result|{call_id}|ok|d3Jvbmc=").unwrap();
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        writeln!(newest, "invoke-result|{call_id}|ok|cmlnaHQ=").unwrap();
        assert_eq!(
            result.recv_timeout(Duration::from_secs(1)).unwrap(),
            InvokeOutcome::Success(b"right".to_vec())
        );

        let error_request = InvokeRequest::new("error", Vec::new());
        let error_call_id = error_request.call_id.clone();
        let error = handle.invoke("sample.extension", error_request);
        let _ = read_line(&mut newest_reader);
        writeln!(newest, "invoke-result|{error_call_id}|err|YmFkIHJlc3VsdA==").unwrap();
        assert_eq!(
            error.recv_timeout(Duration::from_secs(1)).unwrap(),
            InvokeOutcome::Error("bad result".to_owned())
        );

        let timeout_request = InvokeRequest::new("slow", Vec::new());
        let timeout = handle.invoke("sample.extension", timeout_request);
        let _ = read_line(&mut newest_reader);
        assert_eq!(
            timeout.recv_timeout(Duration::from_secs(1)).unwrap(),
            InvokeOutcome::Timeout
        );

        let disconnected_request = InvokeRequest::new("disconnect", Vec::new());
        let disconnected = handle.invoke("sample.extension", disconnected_request);
        let _ = read_line(&mut newest_reader);
        drop(newest_reader);
        drop(newest);
        assert_eq!(
            disconnected.recv_timeout(Duration::from_secs(1)).unwrap(),
            InvokeOutcome::Unavailable
        );

        identify(&mut first, &mut first_reader);
        let replacement_request = InvokeRequest::new("replacement", Vec::new());
        let replacement = handle.invoke("sample.extension", replacement_request);
        let _ = read_line(&mut first_reader);
        handle.replace_extension_snapshot(ExtensionSnapshot::default());
        assert_eq!(
            replacement.recv_timeout(Duration::from_secs(1)).unwrap(),
            InvokeOutcome::Unavailable
        );
    }

    #[test]
    fn final_invoke_result_is_routed_before_half_close_detaches_its_owner() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, handle, _) = SocketServer::start(extension_config(&path, true)).unwrap();
        let mut writer = connect(&path);
        let mut reader = BufReader::new(writer.try_clone().unwrap());
        identify(&mut writer, &mut reader);

        let request = InvokeRequest::new("sample.action", b"payload".to_vec());
        let call_id = request.call_id.clone();
        let result = handle.invoke("sample.extension", request);
        assert_eq!(
            read_line(&mut reader),
            format!("invoke|{call_id}|sample.action|cGF5bG9hZA==\n")
        );
        writeln!(writer, "invoke-result|{call_id}|ok|cmlnaHQ=").unwrap();
        writer.shutdown(Shutdown::Write).unwrap();

        assert_eq!(
            result.recv_timeout(Duration::from_secs(1)).unwrap(),
            InvokeOutcome::Success(b"right".to_vec())
        );
    }

    #[test]
    fn valid_hooks_ack_before_dedup_and_invalid_hooks_continue_routing() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = start(&path);
        let hook = r#"{"v":3,"kind":"agent_event","id":"same","provider":"sample","paneID":"123e4567-e89b-12d3-a456-426614174000","phase":"finished","title":"Done","body":"Ready","pids":[42],"ts":7}"#;
        for index in 0..2 {
            let mut client = connect(&path);
            writeln!(client, "{hook}").unwrap();
            client.shutdown(Shutdown::Write).unwrap();
            assert_eq!(
                read_all(client),
                b"{\"kind\":\"ack\",\"ok\":true,\"v\":3}\n"
            );
            if index == 0 {
                let IncomingRequest::AgentHook(event) = incoming.recv_blocking().unwrap() else {
                    panic!("expected hook")
                };
                assert_eq!(
                    event.pane_id.as_deref(),
                    Some("123E4567-E89B-12D3-A456-426614174000")
                );
            } else {
                assert!(incoming.try_recv().is_err());
            }
        }

        let mut client = connect(&path);
        client.write_all(b"{invalid}|pane|title|body\n").unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let IncomingRequest::LegacyNotification(notification) = incoming.recv_blocking().unwrap()
        else {
            panic!("expected notification fallback")
        };
        assert_eq!(notification.notification_type, "{invalid}");
        assert!(read_all(client).is_empty());
    }

    #[test]
    fn denied_notification_disconnects_on_the_hundredth_valid_drop() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = SocketServer::start(extension_config(&path, false)).unwrap();
        let mut writer = connect(&path);
        let mut reader = BufReader::new(writer.try_clone().unwrap());
        identify(&mut writer, &mut reader);
        for _ in 0..99 {
            writer.write_all(b"finished|pane|title|body\n").unwrap();
        }
        writer.write_all(b"subscribe|allowed\n").unwrap();
        assert_eq!(read_line(&mut reader), "ok\n");
        assert!(incoming.try_recv().is_err());
        writer.write_all(b"malformed\n").unwrap();
        writer.write_all(b"finished|pane|title|body\n").unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert!(bytes.is_empty());
        assert!(incoming.try_recv().is_err());
    }

    #[test]
    fn identified_command_limit_uses_newlines_and_keeps_the_session_live() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, _, incoming) = SocketServer::start(extension_config(&path, true)).unwrap();
        let mut writer = connect(&path);
        let mut reader = BufReader::new(writer.try_clone().unwrap());
        identify(&mut writer, &mut reader);
        for index in 0..=DEFAULT_MAX_IN_FLIGHT_COMMANDS {
            writeln!(writer, "sample-command|{index}").unwrap();
        }
        let requests = (0..DEFAULT_MAX_IN_FLIGHT_COMMANDS)
            .map(|_| {
                let IncomingRequest::AppCommand(request) = incoming.recv_blocking().unwrap() else {
                    panic!("expected app command")
                };
                request
            })
            .collect::<Vec<_>>();
        assert!(incoming.try_recv().is_err());
        assert_eq!(
            read_line(&mut reader),
            "error:too many concurrent commands\n"
        );
        for request in requests {
            request.responder.respond(CommandReply::new("ok"));
        }
        for _ in 0..DEFAULT_MAX_IN_FLIGHT_COMMANDS {
            assert_eq!(read_line(&mut reader), "ok\n");
        }
        writer.write_all(b"subscribe|allowed\n").unwrap();
        assert_eq!(read_line(&mut reader), "ok\n");
    }

    #[test]
    fn identified_eof_keeps_owed_reply_but_rejects_pushes_and_invokes() {
        let directory = directory();
        let path = directory.path().join("main.sock");
        let (_server, handle, incoming) =
            SocketServer::start(extension_config(&path, true)).unwrap();
        let mut writer = connect(&path);
        let mut reader = BufReader::new(writer.try_clone().unwrap());
        identify(&mut writer, &mut reader);
        let invoke_request = InvokeRequest::new("pending", Vec::new());
        let invoke = handle.invoke("sample.extension", invoke_request);
        let _ = read_line(&mut reader);
        writer.write_all(b"sample-command\n").unwrap();
        let IncomingRequest::AppCommand(request) = incoming.recv_blocking().unwrap() else {
            panic!("expected app command")
        };
        writer.shutdown(Shutdown::Write).unwrap();
        assert_eq!(
            invoke.recv_timeout(Duration::from_secs(1)).unwrap(),
            InvokeOutcome::Unavailable
        );
        handle.broadcast(ExtensionBroadcast {
            name: "allowed".to_owned(),
            payload: BTreeMap::new(),
        });
        handle.push_extension_event(
            "sample.extension",
            ExtensionLocalEvent {
                name: "extension.after-eof".to_owned(),
                payload: Vec::new(),
            },
        );
        request.responder.respond(CommandReply::new("owed"));
        assert_eq!(read_all(reader.into_inner()), b"owed\n");
    }
}
