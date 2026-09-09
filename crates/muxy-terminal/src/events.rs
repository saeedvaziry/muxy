use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    Title(String),
    Directory(String),
    Bell,
}

#[derive(Debug, Default)]
pub(crate) struct Events {
    pub(crate) title: Rc<Cell<bool>>,
    pub(crate) directory: Rc<Cell<bool>>,
    pub(crate) bell: Rc<Cell<bool>>,
}
