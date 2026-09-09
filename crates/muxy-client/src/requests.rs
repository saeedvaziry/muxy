use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard, PoisonError};

use muxy_protocol::{ReplyBody, RequestId};

use crate::ClientError;

#[derive(Debug, Default)]
struct State {
    next: u32,
    waiting: HashMap<RequestId, Sender<ReplyBody>>,
    closed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Pending {
    state: Mutex<State>,
}

impl Pending {
    pub(crate) fn register(&self) -> Result<(RequestId, Receiver<ReplyBody>), ClientError> {
        let mut state = self.lock();
        if state.closed {
            return Err(ClientError::Disconnected);
        }
        let id = loop {
            let id = RequestId(state.next);
            state.next = state.next.wrapping_add(1);
            if !state.waiting.contains_key(&id) {
                break id;
            }
        };
        let (sender, receiver) = mpsc::channel();
        state.waiting.insert(id, sender);
        Ok((id, receiver))
    }

    pub(crate) fn resolve(&self, id: RequestId, body: ReplyBody) {
        if let Some(sender) = self.lock().waiting.remove(&id) {
            let _ = sender.send(body);
        }
    }

    pub(crate) fn forget(&self, id: RequestId) {
        self.lock().waiting.remove(&id);
    }

    pub(crate) fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.waiting.clear();
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.lock().closed
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::TryRecvError;

    use super::*;

    #[test]
    fn replies_reach_their_request_and_closing_fails_the_rest() -> Result<(), ClientError> {
        let pending = Pending::default();
        let (first, first_reply) = pending.register()?;
        let (second, second_reply) = pending.register()?;
        assert_ne!(first, second);
        pending.resolve(second, ReplyBody::Pong);
        pending.resolve(RequestId(99), ReplyBody::Detached);
        assert_eq!(second_reply.try_recv(), Ok(ReplyBody::Pong));
        assert_eq!(first_reply.try_recv(), Err(TryRecvError::Empty));
        pending.close();
        assert_eq!(first_reply.try_recv(), Err(TryRecvError::Disconnected));
        assert!(matches!(pending.register(), Err(ClientError::Disconnected)));
        Ok(())
    }

    #[test]
    fn forgotten_requests_ignore_late_replies() -> Result<(), ClientError> {
        let pending = Pending::default();
        let (id, reply) = pending.register()?;
        pending.forget(id);
        pending.resolve(id, ReplyBody::Pong);
        assert_eq!(reply.try_recv(), Err(TryRecvError::Disconnected));
        Ok(())
    }
}
