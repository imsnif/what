use std::{io, sync::mpsc::SendError};

use crate::threading::messages::{ClockCmd, DisplayCmd, SnifferCmd, TrackerCmd};

/// Fatal errors that can be encountered by worker threads.
#[derive(Debug, thiserror::Error)]
pub enum ThreadError {
    #[error("terminal events iterator terminated unexpectedly")]
    TerminalEventsTerminated,
    #[error("terminal stop did not complete successfully")]
    TerminalStopFail(io::Error),

    #[error("all receivers of a ClockCmd channel have hung up")]
    ClockCmdSend,
    #[error("all receivers of a DisplayCmd channel have hung up")]
    DisplayCmdSend,
    #[error("all receivers of a TrackerCmd channel have hung up")]
    TrackerCmdSend,
    #[error("all receivers of a SnifferCmd channel have hung up")]
    SnifferCmdSend,

    #[error("all senders of a ClockCmd channel have hung up")]
    ClockCmdRecv,
    #[error("all senders of a DisplayCmd channel have hung up")]
    DisplayCmdRecv,
    #[error("all senders of a TrackerCmd channel have hung up")]
    TrackerCmdRecv,
    #[error("all senders of a SnifferCmd channel have hung up")]
    SnifferCmdRecv,
}
impl From<SendError<ClockCmd>> for ThreadError {
    fn from(_: SendError<ClockCmd>) -> Self {
        Self::ClockCmdSend
    }
}
impl From<SendError<DisplayCmd>> for ThreadError {
    fn from(_: SendError<DisplayCmd>) -> Self {
        Self::DisplayCmdSend
    }
}
impl From<SendError<TrackerCmd>> for ThreadError {
    fn from(_: SendError<TrackerCmd>) -> Self {
        Self::TrackerCmdSend
    }
}
impl From<SendError<SnifferCmd>> for ThreadError {
    fn from(_: SendError<SnifferCmd>) -> Self {
        Self::SnifferCmdSend
    }
}
