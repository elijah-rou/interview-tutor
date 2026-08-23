use super::Backend;
use super::error::{ErrorKind, InterviewerError};
use super::prompt::{self, Mode};
use super::transcript::{self, SessionTranscript, Speaker};
use crate::runner::CancellationToken;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;

pub struct InterviewRequest<'a> {
    pub mode: Mode,
    pub statement: &'a str,
    pub source: &'a str,
    pub latest_output: &'a str,
    pub question: &'a str,
    pub source_revision: u64,
    pub solved: bool,
}

pub trait Transport: Send {
    fn backend(&self) -> Backend;

    fn prepare_next_operation(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), InterviewerError>;

    fn raw_turn(
        &mut self,
        mode: Mode,
        input: String,
        output_schema: Value,
        correction: bool,
        cancellation: &CancellationToken,
    ) -> Result<String, InterviewerError>;

    fn finish_operation(&mut self) -> Result<(), InterviewerError>;

    fn invalidate_operation(&mut self) -> Result<(), InterviewerError>;

    fn requires_restart(&self) -> bool;
}

pub struct InterviewerSession {
    transport: Box<dyn Transport>,
    transcript: SessionTranscript,
    hint_revision: Option<u64>,
    hint_count: u8,
}

impl InterviewerSession {
    pub fn connect(
        backend: Backend,
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, InterviewerError> {
        let transport: Box<dyn Transport> = match backend {
            Backend::Pi => Box::new(crate::pi::PiTransport::connect(control_pid, cancellation)?),
            Backend::Codex => Box::new(crate::codex::CodexTransport::connect(
                control_pid,
                cancellation,
            )?),
            Backend::None => {
                return Err(InterviewerError::configuration(
                    Backend::Pi,
                    "disabled interviewer cannot connect",
                ));
            }
        };
        Ok(Self::from_transport(transport))
    }

    pub(crate) fn from_transport(transport: Box<dyn Transport>) -> Self {
        assert!(transport.backend() != Backend::None);
        Self {
            transport,
            transcript: SessionTranscript::default(),
            hint_revision: None,
            hint_count: 0,
        }
    }

    pub fn backend(&self) -> Backend {
        self.transport.backend()
    }

    pub fn prepare_next_operation(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), InterviewerError> {
        self.transport.prepare_next_operation(cancellation)
    }

    pub fn requires_restart(&self) -> bool {
        self.transport.requires_restart()
    }

    pub fn ask(&mut self, request: InterviewRequest<'_>) -> Result<String, InterviewerError> {
        self.ask_with_cancellation(request, &CancellationToken::new())
    }

    pub fn ask_with_cancellation(
        &mut self,
        request: InterviewRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<String, InterviewerError> {
        self.ask_internal(request, cancellation, true)
    }

    pub(crate) fn ask_deferred_with_cancellation(
        &mut self,
        request: InterviewRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<String, InterviewerError> {
        self.ask_internal(request, cancellation, false)
    }

    fn ask_internal(
        &mut self,
        request: InterviewRequest<'_>,
        cancellation: &CancellationToken,
        record_response: bool,
    ) -> Result<String, InterviewerError> {
        self.prepare_next_operation(cancellation)?;
        let backend = self.backend();
        if request.question.len() > transcript::MAX_USER_BYTES {
            return Err(InterviewerError::configuration(
                backend,
                "question exceeds 16 KiB",
            ));
        }
        let mode = request.mode;
        if let Mode::Hint(level) = mode {
            if !(1..=3).contains(&level) {
                return Err(InterviewerError::configuration(
                    backend,
                    "hint level must be 1 through 3",
                ));
            }
            if self.hint_revision != Some(request.source_revision) {
                self.hint_revision = Some(request.source_revision);
                self.hint_count = 0;
            }
            if self.hint_count >= 3 {
                return Err(InterviewerError::configuration(
                    backend,
                    "maximum three hints reached for this revision",
                ));
            }
        }
        let transcript = if matches!(mode, Mode::Hint(_)) {
            String::new()
        } else {
            self.transcript
                .entries()
                .map(|entry| {
                    let label = match entry.speaker {
                        Speaker::User => "user",
                        Speaker::Interviewer => "interviewer",
                        Speaker::Hinter => "hinter",
                        Speaker::SubmissionReview => "review",
                    };
                    format!("{label}: {}", entry.text)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let payload = prompt::user_payload(
            request.statement,
            request.source,
            request.latest_output,
            &transcript,
            request.question,
        );
        let input = format!(
            "{}\nINPUT_JSON:{}",
            prompt::system_contract(mode, request.solved),
            serde_json::to_string(&payload).map_err(|_| {
                InterviewerError::configuration(backend, "cannot encode interviewer prompt")
            })?
        );
        let raw = self.transport.raw_turn(
            mode,
            input,
            prompt::output_schema(mode),
            false,
            cancellation,
        );
        let raw = match raw {
            Ok(raw) => raw,
            Err(error) => {
                return Err(combine_finish_error(
                    error,
                    self.transport.finish_operation(),
                ));
            }
        };
        let response = match prompt::parse_response(mode, &raw) {
            Ok(response) => response,
            Err(_) => {
                let correction = "Your prior response did not match the required JSON envelope. Return only one corrected JSON object, with no markdown or commentary.".to_string();
                let corrected = self.transport.raw_turn(
                    mode,
                    correction,
                    prompt::output_schema(mode),
                    true,
                    cancellation,
                );
                let corrected = match corrected {
                    Ok(corrected) => corrected,
                    Err(error) => {
                        return Err(combine_finish_error(
                            error,
                            self.transport.finish_operation(),
                        ));
                    }
                };
                match prompt::parse_response(mode, &corrected) {
                    Ok(response) => response,
                    Err(_) => {
                        let error = InterviewerError::protocol(
                            backend,
                            format!(
                                "{} returned malformed structured output twice",
                                backend.display_name()
                            ),
                        );
                        return Err(combine_finish_error(
                            error,
                            self.transport.invalidate_operation(),
                        ));
                    }
                }
            }
        };
        self.transport.finish_operation()?;
        if record_response {
            self.commit_response(
                mode,
                request.source_revision,
                request.question,
                response.clone(),
            );
        }
        Ok(response)
    }

    pub(crate) fn commit_response(
        &mut self,
        mode: Mode,
        source_revision: u64,
        question: &str,
        response: String,
    ) {
        match mode {
            Mode::Hint(_) => {
                if self.hint_revision != Some(source_revision) {
                    self.hint_revision = Some(source_revision);
                    self.hint_count = 0;
                }
                self.hint_count = self.hint_count.saturating_add(1);
                self.transcript
                    .push(Speaker::Hinter, response)
                    .expect("validated hinter response");
            }
            Mode::Interviewer => {
                self.transcript
                    .push(Speaker::User, question.to_string())
                    .expect("validated interviewer question");
                self.transcript
                    .push(Speaker::Interviewer, response)
                    .expect("validated interviewer response");
            }
            Mode::SubmissionReview => self
                .transcript
                .push(Speaker::SubmissionReview, response)
                .expect("validated submission review"),
        }
    }

    pub fn clear(&mut self) {
        self.transcript.clear();
        self.hint_count = 0;
        self.hint_revision = None;
    }
}

fn combine_finish_error(
    primary: InterviewerError,
    cleanup: Result<(), InterviewerError>,
) -> InterviewerError {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup) => InterviewerError::new(
            primary.backend(),
            if primary.kind() == ErrorKind::Authentication {
                ErrorKind::Authentication
            } else {
                primary.kind()
            },
            format!("{primary}; cleanup failed: {cleanup}"),
        ),
    }
}
