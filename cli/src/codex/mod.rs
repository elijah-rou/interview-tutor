pub mod process;
pub mod prompt;
pub mod protocol;
pub mod session;

use crate::runner::CancellationToken;
use process::CodexProcess;
use prompt::Mode;
use session::{SessionTranscript, Speaker};
use std::path::{Path, PathBuf};
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

pub struct CodexSession {
    executable: PathBuf,
    control_pid: Arc<AtomicI32>,
    process: Option<CodexProcess>,
    interviewer_thread: String,
    hinter_thread: String,
    restart_remaining: bool,
    transcript: SessionTranscript,
    hint_revision: Option<u64>,
    hint_count: u8,
}

impl CodexSession {
    pub fn connect() -> Result<Self, String> {
        Self::connect_with_control(Arc::new(AtomicI32::new(0)))
    }

    pub fn connect_with_control(control_pid: Arc<AtomicI32>) -> Result<Self, String> {
        Self::connect_with_control_and_cancellation(control_pid, &CancellationToken::new())
    }

    pub fn connect_with_control_and_cancellation(
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        let executable = process::configured_executable()?;
        Self::connect_executable(executable, control_pid, cancellation)
    }

    fn connect_executable(
        executable: PathBuf,
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        let (process, interviewer_thread, hinter_thread) =
            establish(&executable, Arc::clone(&control_pid), cancellation)?;
        Ok(Self {
            executable,
            control_pid,
            process: Some(process),
            interviewer_thread,
            hinter_thread,
            restart_remaining: true,
            transcript: SessionTranscript::default(),
            hint_revision: None,
            hint_count: 0,
        })
    }

    pub fn ask(&mut self, request: InterviewRequest<'_>) -> Result<String, String> {
        self.ask_with_cancellation(request, &CancellationToken::new())
    }

    pub fn ask_with_cancellation(
        &mut self,
        request: InterviewRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<String, String> {
        self.ensure_connected(cancellation)?;
        if request.question.len() > session::MAX_USER_BYTES {
            return Err("question exceeds 16 KiB".into());
        }
        let mode = request.mode;
        if let Mode::Hint(level) = mode {
            if !(1..=3).contains(&level) {
                return Err("hint level must be 1 through 3".into());
            }
            if self.hint_revision != Some(request.source_revision) {
                self.hint_revision = Some(request.source_revision);
                self.hint_count = 0;
            }
            if self.hint_count >= 3 {
                return Err("maximum three hints reached for this revision".into());
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
            serde_json::to_string(&payload).map_err(|_| "cannot encode prompt")?
        );
        let thread = if matches!(mode, Mode::Hint(_)) {
            self.hinter_thread.clone()
        } else {
            self.interviewer_thread.clone()
        };
        let raw = match self.process.as_mut().expect("connection established").turn(
            &thread,
            input,
            prompt::output_schema(mode),
            cancellation,
        ) {
            Ok(raw) => raw,
            Err(error) => {
                self.note_process_state();
                return Err(error);
            }
        };
        let response = match prompt::parse_response(mode, &raw) {
            Ok(response) => response,
            Err(_) => {
                let correction = "Your prior response did not match the required JSON envelope. Return only one corrected JSON object, with no markdown or commentary.".to_string();
                let raw = match self.process.as_mut().expect("connection established").turn(
                    &thread,
                    correction,
                    prompt::output_schema(mode),
                    cancellation,
                ) {
                    Ok(raw) => raw,
                    Err(error) => {
                        self.note_process_state();
                        return Err(error);
                    }
                };
                prompt::parse_response(mode, &raw)
                    .map_err(|_| "Codex returned malformed structured output twice")?
            }
        };
        match mode {
            Mode::Hint(_) => {
                self.hint_count += 1;
                self.transcript.push(Speaker::Hinter, response.clone())?;
            }
            Mode::Interviewer => {
                self.transcript
                    .push(Speaker::User, request.question.to_string())?;
                self.transcript
                    .push(Speaker::Interviewer, response.clone())?;
            }
            Mode::SubmissionReview => self
                .transcript
                .push(Speaker::SubmissionReview, response.clone())?,
        }
        Ok(response)
    }

    fn ensure_connected(&mut self, cancellation: &CancellationToken) -> Result<(), String> {
        if self.process.as_ref().is_some_and(CodexProcess::is_usable) {
            return Ok(());
        }
        self.process.take();
        if !self.restart_remaining {
            return Err("Codex session restart limit reached".into());
        }
        self.restart_remaining = false;
        let (process, interviewer_thread, hinter_thread) = establish(
            &self.executable,
            Arc::clone(&self.control_pid),
            cancellation,
        )?;
        self.process = Some(process);
        self.interviewer_thread = interviewer_thread;
        self.hinter_thread = hinter_thread;
        Ok(())
    }

    fn note_process_state(&mut self) {
        if self
            .process
            .as_ref()
            .is_some_and(|process| !process.is_usable())
        {
            self.process.take();
            self.interviewer_thread.clear();
            self.hinter_thread.clear();
        }
    }

    pub fn clear(&mut self) {
        self.transcript.clear();
        self.hint_count = 0;
        self.hint_revision = None;
    }
}

fn establish(
    executable: &Path,
    control_pid: Arc<AtomicI32>,
    cancellation: &CancellationToken,
) -> Result<(CodexProcess, String, String), String> {
    let mut process =
        CodexProcess::start_executable(executable.to_path_buf(), control_pid, cancellation)?;
    if !process.account_ready_with_cancellation(cancellation)? {
        return Err("Codex authentication required; run `codex login`".into());
    }
    let interviewer_thread = process.start_thread_with_cancellation(cancellation)?;
    let hinter_thread = process.start_thread_with_cancellation(cancellation)?;
    Ok((process, interviewer_thread, hinter_thread))
}
