use super::Backend;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Authentication,
    Cancelled,
    Configuration,
    Protocol,
    Timeout,
    Transport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterviewerError {
    backend: Backend,
    kind: ErrorKind,
    message: String,
}

impl InterviewerError {
    pub fn new(backend: Backend, kind: ErrorKind, message: impl Into<String>) -> Self {
        assert!(
            backend != Backend::None,
            "disabled interviewer cannot produce backend errors"
        );
        let message = message.into();
        assert!(
            !message.is_empty(),
            "interviewer error message must not be empty"
        );
        Self {
            backend,
            kind,
            message,
        }
    }

    pub fn authentication(backend: Backend, message: impl Into<String>) -> Self {
        Self::new(backend, ErrorKind::Authentication, message)
    }

    pub fn protocol(backend: Backend, message: impl Into<String>) -> Self {
        Self::new(backend, ErrorKind::Protocol, message)
    }

    pub fn transport(backend: Backend, message: impl Into<String>) -> Self {
        Self::new(backend, ErrorKind::Transport, message)
    }

    pub fn configuration(backend: Backend, message: impl Into<String>) -> Self {
        Self::new(backend, ErrorKind::Configuration, message)
    }

    pub fn cancelled(backend: Backend, message: impl Into<String>) -> Self {
        Self::new(backend, ErrorKind::Cancelled, message)
    }

    pub fn timeout(backend: Backend, message: impl Into<String>) -> Self {
        Self::new(backend, ErrorKind::Timeout, message)
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn contains(&self, pattern: &str) -> bool {
        self.message.contains(pattern)
    }
}

impl fmt::Display for InterviewerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for InterviewerError {}
