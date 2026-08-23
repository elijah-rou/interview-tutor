pub mod process;

use crate::interviewer::{Backend, InterviewerError, Mode, Transport};
use crate::runner::CancellationToken;
use process::{ExecutableIdentity, PiProcess};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;

pub struct PiTransport {
    executable: PathBuf,
    identity: ExecutableIdentity,
    control_pid: Arc<AtomicI32>,
    process: Option<PiProcess>,
}

impl PiTransport {
    pub fn connect(
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, InterviewerError> {
        let executable = process::configured_executable()
            .map_err(|error| InterviewerError::configuration(Backend::Pi, error))?;
        let identity = process::trusted_executable_identity(&executable)
            .map_err(|error| InterviewerError::configuration(Backend::Pi, error))?;
        process::validate_version(&executable, cancellation)
            .map_err(|error| InterviewerError::configuration(Backend::Pi, error))?;
        let current = process::trusted_executable_identity(&executable)
            .map_err(|error| InterviewerError::configuration(Backend::Pi, error))?;
        if current != identity {
            return Err(InterviewerError::configuration(
                Backend::Pi,
                "Pi executable changed after version probe",
            ));
        }
        Ok(Self {
            executable,
            identity,
            control_pid,
            process: None,
        })
    }
}

impl Transport for PiTransport {
    fn backend(&self) -> Backend {
        Backend::Pi
    }

    fn prepare_next_operation(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), InterviewerError> {
        if cancellation.is_cancelled() {
            return Err(InterviewerError::cancelled(
                Backend::Pi,
                "Pi operation cancelled",
            ));
        }
        if self.process.is_some() {
            return Err(InterviewerError::protocol(
                Backend::Pi,
                "Pi retained a process outside its correction turn",
            ));
        }
        Ok(())
    }

    fn raw_turn(
        &mut self,
        _mode: Mode,
        input: String,
        _output_schema: Value,
        correction: bool,
        cancellation: &CancellationToken,
    ) -> Result<String, InterviewerError> {
        if correction {
            let process = self.process.as_mut().ok_or_else(|| {
                InterviewerError::protocol(Backend::Pi, "Pi correction has no active process")
            })?;
            return process.turn(input, cancellation);
        }
        if self.process.is_some() {
            return Err(InterviewerError::protocol(
                Backend::Pi,
                "Pi application turn attempted to reuse a process",
            ));
        }
        self.process = Some(PiProcess::start(
            self.executable.clone(),
            &self.identity,
            Arc::clone(&self.control_pid),
            cancellation,
        )?);
        self.process
            .as_mut()
            .expect("Pi process inserted")
            .turn(input, cancellation)
    }

    fn finish_operation(&mut self) -> Result<(), InterviewerError> {
        let Some(mut process) = self.process.take() else {
            return Ok(());
        };
        process.shutdown()
    }

    fn invalidate_operation(&mut self) -> Result<(), InterviewerError> {
        self.finish_operation()
    }

    fn requires_restart(&self) -> bool {
        false
    }
}

impl Drop for PiTransport {
    fn drop(&mut self) {
        if let Some(process) = self.process.as_mut() {
            let _ = process.shutdown();
        }
    }
}
