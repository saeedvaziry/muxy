use std::collections::VecDeque;

use super::{Client, Update, Work, Worker, WorkerPool, delivery::Delivery, rejected, schedule};

const MAX_QUEUED_REQUESTS: usize = 128;

pub(super) struct Requests {
    pool: WorkerPool,
    queued: VecDeque<Work>,
    pub(super) running: bool,
}

impl Requests {
    pub(super) fn new() -> std::io::Result<Self> {
        Ok(Self {
            pool: WorkerPool::new("muxy-app-requests", 1, 1)?,
            queued: VecDeque::new(),
            running: false,
        })
    }

    pub(super) fn reset(&mut self) {
        self.queued.clear();
    }

    pub(super) fn push(&mut self, work: Work, delivery: &mut Delivery) -> Option<Update> {
        if let Work::Resize(channel, _) = &work {
            let previous = self
                .queued
                .iter()
                .enumerate()
                .rev()
                .take_while(|(_, work)| matches!(work, Work::Resize(_, _)))
                .find_map(|(index, work)| {
                    matches!(work, Work::Resize(previous, _) if previous == channel)
                        .then_some(index)
                });
            if let Some(index) = previous {
                self.queued.remove(index);
                self.queued.push_back(work);
                return None;
            }
            // Keep one latest size per channel in the trailing resize run even at capacity.
            // Other requests are ordering barriers and cannot extend a full queue.
        } else if self.queued.len() >= MAX_QUEUED_REQUESTS {
            return Some(rejected(
                work,
                std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "background work queue is full",
                )
                .into(),
            ));
        }
        self.queued.push_back(work);
        delivery.pending += 1;
        None
    }

    pub(super) fn start(
        &mut self,
        client: &Client,
        generation: u64,
        completed: &Worker,
        delivery: &mut Delivery,
    ) -> Vec<Update> {
        let mut updates = Vec::new();
        while !self.running {
            let Some(work) = self.queued.pop_front() else {
                break;
            };
            if let Some(update) = schedule(
                &self.pool,
                work,
                client.clone(),
                generation,
                completed.clone(),
            ) {
                updates.extend(delivery.complete(Some(update)));
            } else {
                self.running = true;
            }
        }
        updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_protocol::{ChannelId, Size};

    #[test]
    fn saturation_bounds_regular_work_but_retains_each_channels_latest_size() -> std::io::Result<()>
    {
        let mut requests = Requests::new()?;
        let mut delivery = Delivery::default();
        for _ in 0..MAX_QUEUED_REQUESTS {
            assert!(
                requests
                    .push(Work::Detach(ChannelId(9)), &mut delivery)
                    .is_none()
            );
        }
        assert!(
            requests
                .push(Work::Detach(ChannelId(9)), &mut delivery)
                .is_some()
        );
        for cols in 1..=1000 {
            for channel in [ChannelId(1), ChannelId(2)] {
                assert!(
                    requests
                        .push(
                            Work::Resize(channel, Size { cols, rows: 24 }),
                            &mut delivery
                        )
                        .is_none()
                );
            }
        }
        assert_eq!(requests.queued.len(), MAX_QUEUED_REQUESTS + 2);
        assert_eq!(delivery.pending, MAX_QUEUED_REQUESTS + 2);
        assert!(matches!(
            requests.queued.back(),
            Some(Work::Resize(
                ChannelId(2),
                Size {
                    cols: 1000,
                    rows: 24
                }
            ))
        ));
        Ok(())
    }
}
