use super::{ClientEvent, Update};

const MAX_DEFERRED_EVENTS: usize = 1024;

#[derive(Default, PartialEq)]
enum Disconnect {
    #[default]
    Open,
    Pending,
    Delivered,
}

#[derive(Default)]
pub(super) struct Delivery {
    pub(super) pending: usize,
    flushing: bool,
    installed_through: u32,
    deferred: Vec<ClientEvent>,
    disconnect: Disconnect,
}

impl Delivery {
    pub(super) fn event(&mut self, event: ClientEvent) -> Result<Vec<Update>, &'static str> {
        if matches!(event, ClientEvent::Disconnected) {
            return Ok(self.disconnected());
        }
        if self.ready(&event) {
            return Ok(vec![Update::Event(event)]);
        }
        if let ClientEvent::Metadata { channel, event: metadata } = &event
            && let Some(previous) = self.deferred.iter_mut().find(|previous| {
                matches!(previous, ClientEvent::Metadata { channel: old_channel, event: old }
                    if old_channel == channel && std::mem::discriminant(old) == std::mem::discriminant(metadata))
            })
        {
            *previous = event;
            return Ok(Vec::new());
        }
        if self.deferred.len() == MAX_DEFERRED_EVENTS {
            return Err("too many events arrived before request completion");
        }
        self.deferred.push(event);
        Ok(Vec::new())
    }

    pub(super) fn complete(&mut self, update: Option<Update>) -> Vec<Update> {
        self.pending = self.pending.saturating_sub(1);
        if let Some(Update::Attached { attachment, .. }) = &update {
            self.installed_through = self.installed_through.max(attachment.channel.0);
        }
        let mut updates: Vec<_> = update.into_iter().collect();
        for event in std::mem::take(&mut self.deferred) {
            if self.ready(&event) {
                updates.push(Update::Event(event));
            } else {
                self.deferred.push(event);
            }
        }
        if self.pending == 0 && self.disconnect == Disconnect::Pending {
            self.disconnect = Disconnect::Delivered;
            updates.push(Update::Event(ClientEvent::Disconnected));
        }
        if self.pending == 0 && self.flushing {
            self.flushing = false;
            updates.push(Update::Flushed);
        }
        updates
    }

    pub(super) fn flush(&mut self) -> Vec<Update> {
        self.flushing = self.pending > 0;
        if self.flushing {
            Vec::new()
        } else {
            vec![Update::Flushed]
        }
    }

    fn disconnected(&mut self) -> Vec<Update> {
        if self.disconnect != Disconnect::Open {
            return Vec::new();
        }
        if self.pending > 0 {
            self.disconnect = Disconnect::Pending;
            Vec::new()
        } else {
            self.disconnect = Disconnect::Delivered;
            vec![Update::Event(ClientEvent::Disconnected)]
        }
    }

    fn ready(&self, event: &ClientEvent) -> bool {
        self.pending == 0
            || match event {
                ClientEvent::Frame { channel, .. } | ClientEvent::Metadata { channel, .. } => {
                    channel.0 <= self.installed_through
                }
                ClientEvent::SessionEnded { .. } | ClientEvent::Disconnected => false,
            }
    }
}
