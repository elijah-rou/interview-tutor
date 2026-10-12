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

pub const RECONNECT_STEP: &str = "press i in the Interview pane to reconnect";
pub const RESTART_STEP: &str = "restart ./interview to reconnect";
pub const CLEAR_SESSION_STEP: &str = "press Space r, then i, in the Interview pane";
pub const SHORTEN_PROMPT_STEP: &str = "shorten the source or clear the transcript with Space r";
const PI_AUTHENTICATION_STEP: &str =
    "set up a model and credentials in `pi`, then restart ./interview";
const CODEX_AUTHENTICATION_STEP: &str = "run `codex login`, then restart ./interview";
const PI_INSTALL_STEP: &str = "install Pi 0.84.4 or set INTERVIEW_TUTOR_PI_EXECUTABLE";
const CODEX_INSTALL_STEP: &str = "install Codex 0.147.0 or set INTERVIEW_TUTOR_CODEX_EXECUTABLE";

/// A typed interviewer failure. `Display` renders `<message>; <next step>`, where the step
/// derives from backend and kind unless a leaf overrides it with `with_step`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterviewerError {
    backend: Backend,
    kind: ErrorKind,
    message: String,
    step: Option<&'static str>,
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
            step: None,
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

    /// Replaces the kind-level next step for a leaf whose recovery is more specific.
    pub fn with_step(mut self, step: &'static str) -> Self {
        assert!(!step.is_empty(), "interviewer next step must not be empty");
        self.step = Some(step);
        self
    }

    /// Appends a cleanup failure to the message, keeping backend, kind, and next step.
    pub fn with_cleanup_failure(mut self, cleanup: &str) -> Self {
        self.message = format!("{}; cleanup failed: {cleanup}", self.message);
        self
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// What went wrong, without the next step.
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn contains(&self, pattern: &str) -> bool {
        self.message.contains(pattern)
    }

    pub fn next_step(&self) -> &'static str {
        if let Some(step) = self.step {
            return step;
        }
        match (self.backend, self.kind) {
            (Backend::Pi, ErrorKind::Authentication) => PI_AUTHENTICATION_STEP,
            (Backend::Codex, ErrorKind::Authentication) => CODEX_AUTHENTICATION_STEP,
            (Backend::Pi, ErrorKind::Configuration) => PI_INSTALL_STEP,
            (Backend::Codex, ErrorKind::Configuration) => CODEX_INSTALL_STEP,
            (
                Backend::Pi | Backend::Codex,
                ErrorKind::Cancelled
                | ErrorKind::Protocol
                | ErrorKind::Timeout
                | ErrorKind::Transport,
            ) => RECONNECT_STEP,
            (Backend::None, _) => {
                unreachable!("disabled interviewer cannot produce backend errors")
            }
        }
    }
}

impl fmt::Display for InterviewerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}; {}", self.message, self.next_step())
    }
}

impl std::error::Error for InterviewerError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_ends_with_exactly_one_kind_level_step() {
        let cases = [
            (
                InterviewerError::authentication(Backend::Codex, "Codex authentication required"),
                "Codex authentication required; run `codex login`, then restart ./interview",
            ),
            (
                InterviewerError::authentication(
                    Backend::Pi,
                    "Pi authentication required: no model selected",
                ),
                "Pi authentication required: no model selected; set up a model and credentials in `pi`, then restart ./interview",
            ),
            (
                InterviewerError::configuration(Backend::Pi, "Pi CLI not found"),
                "Pi CLI not found; install Pi 0.84.4 or set INTERVIEW_TUTOR_PI_EXECUTABLE",
            ),
            (
                InterviewerError::configuration(
                    Backend::Codex,
                    "unsupported Codex CLI codex-cli 9.9.9",
                ),
                "unsupported Codex CLI codex-cli 9.9.9; install Codex 0.147.0 or set INTERVIEW_TUTOR_CODEX_EXECUTABLE",
            ),
            (
                InterviewerError::protocol(Backend::Pi, "Pi record is not an object"),
                "Pi record is not an object; press i in the Interview pane to reconnect",
            ),
            (
                InterviewerError::transport(Backend::Codex, "Codex closed stdout"),
                "Codex closed stdout; press i in the Interview pane to reconnect",
            ),
            (
                InterviewerError::timeout(Backend::Pi, "Pi turn timed out after 120000ms"),
                "Pi turn timed out after 120000ms; press i in the Interview pane to reconnect",
            ),
            (
                InterviewerError::cancelled(Backend::Pi, "Pi turn cancelled"),
                "Pi turn cancelled; press i in the Interview pane to reconnect",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
            assert!(!error.message().contains(error.next_step()));
        }
    }

    #[test]
    fn overrides_and_cleanup_failures_keep_a_single_step() {
        let error =
            InterviewerError::transport(Backend::Codex, "Codex session restart limit reached")
                .with_step(RESTART_STEP)
                .with_cleanup_failure("cannot remove temporary directory");
        assert_eq!(
            error.to_string(),
            "Codex session restart limit reached; cleanup failed: cannot remove temporary directory; restart ./interview to reconnect"
        );
        assert_eq!(error.kind(), ErrorKind::Transport);
        assert_eq!(
            error.message(),
            "Codex session restart limit reached; cleanup failed: cannot remove temporary directory"
        );
        assert!(!error.contains("restart ./interview"));
    }

    #[test]
    fn install_steps_name_the_accepted_versions() {
        assert!(PI_INSTALL_STEP.contains(crate::pi::process::SUPPORTED_PI_VERSION));
        assert!(
            crate::codex::process::SUPPORTED_CODEX_VERSIONS
                .iter()
                .any(|version| version.ends_with(" 0.147.0"))
        );
        assert!(CODEX_INSTALL_STEP.contains("0.147.0"));
    }
}
