//! Errors starting, resuming, or sending to a session.

use dovetail_driver::DriverError;

/// Starting, resuming or sending to a session.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// No such agent.
    #[error("no agent {0}")]
    NoAgent(String),
    /// The agent has no running session.
    #[error("{0} is not running")]
    NotRunning(String),
    /// A slice whose session never started cannot be resumed.
    #[error("slice {0} never started; archive it and start a new one")]
    NeverStarted(String),
    /// An Ask session starts with a question from reading mode (ADR 0018).
    #[error("{0} starts when you ask it something in reading mode")]
    AskNotStarted(String),
    /// Subagents run inside their parent's session.
    #[error("subagents have no session of their own")]
    Subagent,
    /// The project's Claude profile cannot be used (ADR 0009).
    #[error(transparent)]
    Profile(#[from] crate::profile::ProfileError),
    /// The launch files could not be written.
    #[error("writing launch settings: {0}")]
    LaunchFiles(#[source] std::io::Error),
    /// More images than one message may carry.
    #[error("at most {0} images per message")]
    TooManyImages(usize),
    /// A pasted image could not be read from its saved file.
    #[error("could not read image {path}: {source}")]
    ImageUnreadable {
        /// The saved file.
        path: std::path::PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// A saved image is over [`dovetail_api::agents::conversation::MAX_ATTACHMENT_BYTES`].
    #[error(
        "image {} is over {}",
        .0.display(),
        dovetail_api::agents::conversation::attachment_limit_text()
    )]
    ImageTooLarge(std::path::PathBuf),
    /// The session is taken over in a native terminal (M14): headless does not start.
    #[error(
        "{0} is taken over in a native terminal: write to it there; Dovetail resumes it \
         headless when that claude exits"
    )]
    TakenOver(String),
    /// The driver failed.
    #[error("{name}: {source}")]
    Driver {
        /// The agent's name.
        name: String,
        /// The cause.
        source: DriverError,
    },
    /// No driver for this session's CLI was registered.
    #[error("no session driver for {}", .0.label())]
    NoDriver(dovetail_api::agents::model::DriverKind),
}
