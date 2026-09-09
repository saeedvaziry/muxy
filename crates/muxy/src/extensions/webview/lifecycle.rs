use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(crate) const LIFECYCLE_ACKNOWLEDGEMENT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExtensionLifecycleVerdict {
    Allow,
    Prevent,
}

struct PendingLifecycleRequest {
    surface_id: String,
    acknowledgement_deadline: Instant,
    acknowledged: bool,
    completion: Option<Box<dyn FnOnce(ExtensionLifecycleVerdict)>>,
}

#[derive(Default)]
pub(crate) struct ExtensionLifecycleCoordinator {
    next_call_id: u64,
    pending: HashMap<String, PendingLifecycleRequest>,
}

impl ExtensionLifecycleCoordinator {
    pub(crate) fn begin(
        &mut self,
        surface_id: impl Into<String>,
        now: Instant,
        completion: impl FnOnce(ExtensionLifecycleVerdict) + 'static,
    ) -> String {
        self.next_call_id = self.next_call_id.wrapping_add(1).max(1);
        let call_id = self.next_call_id.to_string();
        self.pending.insert(
            call_id.clone(),
            PendingLifecycleRequest {
                surface_id: surface_id.into(),
                acknowledgement_deadline: now + LIFECYCLE_ACKNOWLEDGEMENT_TIMEOUT,
                acknowledged: false,
                completion: Some(Box::new(completion)),
            },
        );
        call_id
    }

    pub(crate) fn acknowledge(&mut self, surface_id: &str, call_id: &str) -> bool {
        let Some(request) = self.pending.get_mut(call_id) else {
            return false;
        };
        if request.surface_id != surface_id || request.acknowledged {
            return false;
        }
        request.acknowledged = true;
        true
    }

    pub(crate) fn resolve(&mut self, surface_id: &str, call_id: &str, prevent: bool) -> bool {
        if self
            .pending
            .get(call_id)
            .is_none_or(|request| request.surface_id != surface_id)
        {
            return false;
        }
        let request = self.pending.remove(call_id).unwrap();
        complete(
            request,
            if prevent {
                ExtensionLifecycleVerdict::Prevent
            } else {
                ExtensionLifecycleVerdict::Allow
            },
        );
        true
    }

    pub(crate) fn expire_unacknowledged(&mut self, call_id: &str, now: Instant) -> bool {
        let expired = self.pending.get(call_id).is_some_and(|request| {
            !request.acknowledged && now >= request.acknowledgement_deadline
        });
        if expired {
            let request = self.pending.remove(call_id).unwrap();
            complete(request, ExtensionLifecycleVerdict::Allow);
        }
        expired
    }

    pub(crate) fn allow_surface(&mut self, surface_id: &str) -> usize {
        let call_ids = self
            .pending
            .iter()
            .filter(|(_, request)| request.surface_id == surface_id)
            .map(|(call_id, _)| call_id.clone())
            .collect::<Vec<_>>();
        let count = call_ids.len();
        for call_id in call_ids {
            if let Some(request) = self.pending.remove(&call_id) {
                complete(request, ExtensionLifecycleVerdict::Allow);
            }
        }
        count
    }

    pub(crate) fn allow_all(&mut self) -> usize {
        let requests = self
            .pending
            .drain()
            .map(|(_, request)| request)
            .collect::<Vec<_>>();
        let count = requests.len();
        for request in requests {
            complete(request, ExtensionLifecycleVerdict::Allow);
        }
        count
    }
}

fn complete(mut request: PendingLifecycleRequest, verdict: ExtensionLifecycleVerdict) {
    if let Some(completion) = request.completion.take() {
        completion(verdict);
    }
}

impl Drop for ExtensionLifecycleCoordinator {
    fn drop(&mut self) {
        self.allow_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn acknowledgement_resolve_timeout_and_surface_retirement_complete_once() {
        let now = Instant::now();
        let outcomes = Rc::new(RefCell::new(Vec::new()));
        let mut coordinator = ExtensionLifecycleCoordinator::default();

        let first = coordinator.begin("surface-1", now, {
            let outcomes = outcomes.clone();
            move |verdict| outcomes.borrow_mut().push(("first", verdict))
        });
        assert!(!coordinator.acknowledge("surface-2", &first));
        assert!(coordinator.acknowledge("surface-1", &first));
        assert!(!coordinator.acknowledge("surface-1", &first));
        assert!(
            !coordinator.expire_unacknowledged(&first, now + LIFECYCLE_ACKNOWLEDGEMENT_TIMEOUT)
        );
        assert!(!coordinator.resolve("surface-2", &first, true));
        assert!(coordinator.resolve("surface-1", &first, true));
        assert!(!coordinator.resolve("surface-1", &first, false));

        let timeout = coordinator.begin("surface-2", now, {
            let outcomes = outcomes.clone();
            move |verdict| outcomes.borrow_mut().push(("timeout", verdict))
        });
        assert!(
            coordinator.expire_unacknowledged(&timeout, now + LIFECYCLE_ACKNOWLEDGEMENT_TIMEOUT)
        );

        coordinator.begin("surface-3", now, {
            let outcomes = outcomes.clone();
            move |verdict| outcomes.borrow_mut().push(("retired", verdict))
        });
        assert_eq!(coordinator.allow_surface("surface-3"), 1);

        assert_eq!(
            *outcomes.borrow(),
            [
                ("first", ExtensionLifecycleVerdict::Prevent),
                ("timeout", ExtensionLifecycleVerdict::Allow),
                ("retired", ExtensionLifecycleVerdict::Allow),
            ]
        );
    }
}
