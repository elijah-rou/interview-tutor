pub mod process;
pub mod prompt;
pub mod protocol;
pub mod session;

use crate::interviewer::{Backend, InterviewerError, Mode, Transport};
use crate::runner::CancellationToken;
use process::CodexProcess;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicI32;

pub use crate::interviewer::InterviewRequest;

pub struct CodexTransport {
    executable: PathBuf,
    control_pid: Arc<AtomicI32>,
    process: Option<CodexProcess>,
    interviewer_thread: String,
    hinter_thread: String,
    restart_remaining: bool,
}

impl CodexTransport {
    pub fn connect(
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, InterviewerError> {
        let executable = process::configured_executable()
            .map_err(|error| InterviewerError::configuration(Backend::Codex, error))?;
        Self::connect_executable(executable, control_pid, cancellation)
    }

    fn connect_executable(
        executable: PathBuf,
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, InterviewerError> {
        let (process, interviewer_thread, hinter_thread) =
            establish(&executable, Arc::clone(&control_pid), cancellation)?;
        Ok(Self {
            executable,
            control_pid,
            process: Some(process),
            interviewer_thread,
            hinter_thread,
            restart_remaining: true,
        })
    }

    fn ensure_connected(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), InterviewerError> {
        if self.process.as_ref().is_some_and(CodexProcess::is_usable) {
            if self.interviewer_thread.is_empty() {
                self.interviewer_thread = match self
                    .process
                    .as_mut()
                    .expect("usable Codex process exists")
                    .start_thread_with_cancellation(cancellation)
                {
                    Ok(thread) => thread,
                    Err(error) => {
                        return Err(self.invalidate_after_turn_failure(format!(
                            "cannot replace rejected Codex interviewer thread: {error}"
                        )));
                    }
                };
            }
            if self.hinter_thread.is_empty() {
                self.hinter_thread = match self
                    .process
                    .as_mut()
                    .expect("usable Codex process exists")
                    .start_thread_with_cancellation(cancellation)
                {
                    Ok(thread) => thread,
                    Err(error) => {
                        return Err(self.invalidate_after_turn_failure(format!(
                            "cannot replace rejected Codex hinter thread: {error}"
                        )));
                    }
                };
            }
            return Ok(());
        }
        if let Some(mut process) = self.process.take()
            && let Err(error) = process.shutdown()
        {
            return Err(InterviewerError::transport(
                Backend::Codex,
                format!("cannot restart Codex session; cleanup failed: {error}"),
            ));
        }
        if !self.restart_remaining {
            return Err(InterviewerError::transport(
                Backend::Codex,
                "Codex session restart limit reached",
            ));
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

    fn invalidate_after_turn_failure(&mut self, primary: String) -> InterviewerError {
        self.interviewer_thread.clear();
        self.hinter_thread.clear();
        let cleanup = self.process.as_mut().map_or(Ok(()), CodexProcess::shutdown);
        let message = match cleanup {
            Ok(()) => primary,
            Err(error) => format!("{primary}; cleanup failed: {error}"),
        };
        InterviewerError::protocol(Backend::Codex, message)
    }
}

impl Transport for CodexTransport {
    fn backend(&self) -> Backend {
        Backend::Codex
    }

    fn prepare_next_operation(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), InterviewerError> {
        self.ensure_connected(cancellation)
    }

    fn raw_turn(
        &mut self,
        mode: Mode,
        _guidance: crate::interviewer::GuidanceMode,
        input: String,
        output_schema: Value,
        _correction: bool,
        cancellation: &CancellationToken,
    ) -> Result<String, InterviewerError> {
        self.ensure_connected(cancellation)?;
        let thread = if matches!(mode, Mode::Hint(_)) {
            self.hinter_thread.clone()
        } else {
            self.interviewer_thread.clone()
        };
        match self.process.as_mut().expect("connection established").turn(
            &thread,
            input,
            output_schema,
            cancellation,
        ) {
            Ok(raw) => Ok(raw),
            Err(error) => Err(self.invalidate_after_turn_failure(error)),
        }
    }

    fn finish_operation(&mut self) -> Result<(), InterviewerError> {
        Ok(())
    }

    fn reject_operation(&mut self, mode: Mode) {
        if matches!(mode, Mode::Hint(_)) {
            self.hinter_thread.clear();
        } else {
            self.interviewer_thread.clear();
        }
    }

    fn invalidate_operation(&mut self) -> Result<(), InterviewerError> {
        self.interviewer_thread.clear();
        self.hinter_thread.clear();
        self.process
            .as_mut()
            .map_or(Ok(()), CodexProcess::shutdown)
            .map_err(|error| InterviewerError::transport(Backend::Codex, error))
    }

    fn requires_restart(&self) -> bool {
        self.process
            .as_ref()
            .is_none_or(|process| !process.is_usable())
    }
}

pub struct CodexSession(crate::interviewer::InterviewerSession);

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
        let transport = CodexTransport::connect(control_pid, cancellation).map_err(string_error)?;
        Ok(Self(
            crate::interviewer::InterviewerSession::from_transport(Box::new(transport)),
        ))
    }

    #[cfg(test)]
    fn connect_executable(
        executable: PathBuf,
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        let transport = CodexTransport::connect_executable(executable, control_pid, cancellation)
            .map_err(string_error)?;
        Ok(Self(
            crate::interviewer::InterviewerSession::from_transport(Box::new(transport)),
        ))
    }

    pub fn prepare_next_operation(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        self.0
            .prepare_next_operation(cancellation)
            .map_err(string_error)
    }

    pub fn requires_restart(&self) -> bool {
        self.0.requires_restart()
    }

    pub fn ask(&mut self, request: InterviewRequest<'_>) -> Result<String, String> {
        self.0.ask(request).map_err(string_error)
    }

    pub fn ask_with_cancellation(
        &mut self,
        request: InterviewRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<String, String> {
        self.0
            .ask_with_cancellation(request, cancellation)
            .map_err(string_error)
    }

    pub fn ask_deferred_with_cancellation(
        &mut self,
        request: InterviewRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<String, String> {
        self.0
            .ask_deferred_with_cancellation(request, cancellation)
            .map_err(string_error)
    }

    pub fn reject_response(&mut self, mode: Mode) {
        self.0.reject_response(mode);
    }

    pub fn commit_response(
        &mut self,
        mode: Mode,
        guidance: crate::interviewer::GuidanceMode,
        source_revision: u64,
        question: &str,
        response: String,
    ) {
        self.0
            .commit_response(mode, guidance, source_revision, question, response);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

fn string_error(error: InterviewerError) -> String {
    error.to_string()
}

fn establish(
    executable: &Path,
    control_pid: Arc<AtomicI32>,
    cancellation: &CancellationToken,
) -> Result<(CodexProcess, String, String), InterviewerError> {
    let mut process =
        CodexProcess::start_executable(executable.to_path_buf(), control_pid, cancellation)
            .map_err(|error| InterviewerError::transport(Backend::Codex, error))?;
    let account_ready = match process.account_ready_with_cancellation(cancellation) {
        Ok(account_ready) => account_ready,
        Err(primary) => {
            return Err(InterviewerError::transport(
                Backend::Codex,
                shutdown_after_failure(&mut process, primary),
            ));
        }
    };
    if !account_ready {
        return Err(InterviewerError::authentication(
            Backend::Codex,
            shutdown_after_failure(
                &mut process,
                "Codex authentication required; run `codex login`".into(),
            ),
        ));
    }
    let interviewer_thread = match process.start_thread_with_cancellation(cancellation) {
        Ok(thread) => thread,
        Err(primary) => {
            return Err(InterviewerError::transport(
                Backend::Codex,
                shutdown_after_failure(&mut process, primary),
            ));
        }
    };
    let hinter_thread = match process.start_thread_with_cancellation(cancellation) {
        Ok(thread) => thread,
        Err(primary) => {
            return Err(InterviewerError::transport(
                Backend::Codex,
                shutdown_after_failure(&mut process, primary),
            ));
        }
    };
    Ok((process, interviewer_thread, hinter_thread))
}

fn shutdown_after_failure(process: &mut CodexProcess, primary: String) -> String {
    match process.shutdown() {
        Ok(()) => primary,
        Err(cleanup) => format!("{primary}; cleanup failed: {cleanup}"),
    }
}
