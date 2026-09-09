use std::io::{self, Read, Write};

pub trait ByteStream: Send {
    fn cancellation(&self) -> io::Result<Box<dyn StreamCancellation>>;
    #[allow(clippy::type_complexity)]
    fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)>;
}

pub trait StreamCancellation: Send + Sync {
    fn cancel(&self);
}

pub trait Listener: Send {
    fn accept(&self) -> io::Result<Box<dyn ByteStream>>;
    fn close(&self);
}
