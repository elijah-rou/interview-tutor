pub mod process;
pub mod prompt;
pub mod protocol;
pub mod session;

use process::CodexProcess;
use prompt::Mode;
use session::{SessionTranscript, Speaker};
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
    process: CodexProcess,
    interviewer_thread: String,
    hinter_thread: String,
    transcript: SessionTranscript,
    hint_revision: Option<u64>,
    hint_count: u8,
}

impl CodexSession {
    pub fn connect() -> Result<Self, String> {
        Self::connect_with_control(Arc::new(AtomicI32::new(0)))
    }

    pub fn connect_with_control(control_pid: Arc<AtomicI32>) -> Result<Self, String> {
        let mut process = CodexProcess::start_with_control(control_pid)?;
        if !process.account_ready()? {
            return Err("Codex authentication required; run `codex login`".into());
        }
        let interviewer_thread = process.start_thread()?;
        let hinter_thread = process.start_thread()?;
        Ok(Self {
            process,
            interviewer_thread,
            hinter_thread,
            transcript: SessionTranscript::default(),
            hint_revision: None,
            hint_count: 0,
        })
    }

    pub fn ask(&mut self, request: InterviewRequest<'_>) -> Result<String, String> {
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
            &self.hinter_thread
        } else {
            &self.interviewer_thread
        };
        let raw = self
            .process
            .turn(thread, input, prompt::output_schema(mode))?;
        let response = match prompt::parse_response(mode, &raw) {
            Ok(response) => response,
            Err(_) => {
                let correction = "Your prior response did not match the required JSON envelope. Return only one corrected JSON object, with no markdown or commentary.".to_string();
                let raw = self
                    .process
                    .turn(thread, correction, prompt::output_schema(mode))?;
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

    pub fn clear(&mut self) {
        self.transcript.clear();
        self.hint_count = 0;
        self.hint_revision = None;
    }
}
