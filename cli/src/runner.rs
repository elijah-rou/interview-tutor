use crate::database;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub struct ExecutionPlan {
    pub language: String,
    pub problem_slug: String,
    pub set_slug: Option<String>,
    pub runner_path: PathBuf,
}

pub struct ExecutionResult {
    pub exit_code: Option<i32>,
    pub status_code: i32,
    pub duration_ms: i64,
}

pub fn plan_execution(
    connection: &Connection,
    root: &Path,
    language: &str,
    problem_reference: &str,
    set_slug: Option<&str>,
) -> Result<ExecutionPlan, String> {
    let problem = database::resolve_problem(connection, problem_reference, set_slug)?;
    let implementation = database::get_implementation(connection, problem.id, language)?;
    let runner_path = root.join(implementation.runner_path);
    let solution_path = root.join(implementation.solution_path);
    if !runner_path.is_file() {
        return Err(format!(
            "language runner is not installed: {}",
            runner_path.display()
        ));
    }
    if !solution_path.is_file() {
        return Err(format!(
            "solution file is not installed: {}",
            solution_path.display()
        ));
    }
    Ok(ExecutionPlan {
        language: language.to_string(),
        problem_slug: problem.slug,
        set_slug: set_slug.map(str::to_string),
        runner_path,
    })
}

pub fn execute_plan(plan: &ExecutionPlan, database_path: &Path) -> Result<ExecutionResult, String> {
    let parent = plan
        .runner_path
        .parent()
        .ok_or_else(|| "language runner has no parent directory".to_string())?;
    let started = Instant::now();
    let status = Command::new(&plan.runner_path)
        .arg("--problem")
        .arg(&plan.problem_slug)
        .current_dir(parent)
        .env("PRACTICE_NO_RECORD", "1")
        .env("PRACTICE_DB_PATH", database_path)
        .status()
        .map_err(|error| format!("cannot execute {}: {error}", plan.runner_path.display()))?;
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    let exit_code = status.code();
    Ok(ExecutionResult {
        exit_code,
        status_code: exit_code.unwrap_or(2),
        duration_ms,
    })
}

fn outcome(exit_code: Option<i32>) -> &'static str {
    match exit_code {
        Some(0) => "pass",
        Some(2) => "error",
        Some(128..=255) | None => "cancelled",
        Some(_) => "fail",
    }
}

pub fn record_execution(
    connection: &Connection,
    plan: &ExecutionPlan,
    result: &ExecutionResult,
) -> Result<(), String> {
    let outcome = outcome(result.exit_code);
    database::record_attempt(
        connection,
        &plan.problem_slug,
        &plan.language,
        outcome,
        result.duration_ms,
        result.exit_code,
        plan.set_slug.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::outcome;

    #[test]
    fn execution_outcomes_preserve_errors_and_signal_termination() {
        assert_eq!(outcome(Some(0)), "pass");
        assert_eq!(outcome(Some(1)), "fail");
        assert_eq!(outcome(Some(2)), "error");
        assert_eq!(outcome(Some(130)), "cancelled");
        assert_eq!(outcome(None), "cancelled");
    }
}
