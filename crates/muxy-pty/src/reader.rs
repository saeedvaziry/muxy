use std::io::{ErrorKind, Read};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

const READ_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PtyEvent {
    Output(Vec<u8>),
    Closed,
}

#[derive(Debug)]
pub struct ReaderHandle {
    thread: JoinHandle<()>,
}

impl ReaderHandle {
    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    pub fn join(self) -> thread::Result<()> {
        self.thread.join()
    }
}

pub(crate) fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    sink: Sender<PtyEvent>,
) -> ReaderHandle {
    let thread = thread::spawn(move || {
        let mut buffer = vec![0; READ_BUFFER_SIZE];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if sink
                        .send(PtyEvent::Output(buffer[..count].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let _ = sink.send(PtyEvent::Closed);
    });
    ReaderHandle { thread }
}
