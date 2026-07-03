#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interrupt {
    Message(String),
}

impl Interrupt {
    pub fn message(msg: impl Into<String>) -> Self {
        Interrupt::Message(msg.into())
    }
}
