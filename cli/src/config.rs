use std::env;
use std::path::{Path, PathBuf};

const DATABASE_ENVIRONMENT_VARIABLES: [&str; 4] = [
    "PRACTICE_DATABASE_URL",
    "PRACTICE_DB_PATH",
    "BLIND75_DATABASE_URL",
    "BLIND75_DB_PATH",
];

/// Next step for failures with no user recovery other than reporting them.
pub const REPORT_BUG_STEP: &str =
    "report this bug at https://github.com/elijah-rou/interview-tutor/issues";
pub(crate) const ROOT_STEP: &str = "set PRACTICE_ROOT to the interview-tutor checkout";

pub fn resolve_root() -> Result<PathBuf, String> {
    if let Some(configured_root) = env::var_os("PRACTICE_ROOT")
        && !configured_root.is_empty()
    {
        let root = PathBuf::from(configured_root);
        if root.is_dir() {
            return Ok(root);
        }
        return Err(format!(
            "project root does not exist: {}; {ROOT_STEP}",
            root.display()
        ));
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("CLI manifest has no project parent; {ROOT_STEP}"))
}

pub fn resolve_database_path(root: &Path, cli_path: Option<&str>) -> Result<PathBuf, String> {
    if cli_path == Some("") {
        return Err("database path must not be empty; pass --db with a file path".to_string());
    }
    let configured = match cli_path {
        Some(path) => Some(("--db", path.to_string())),
        None => DATABASE_ENVIRONMENT_VARIABLES.into_iter().find_map(|name| {
            env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| (name, value))
        }),
    };
    let Some((source, configured)) = configured else {
        return Ok(root.join(".turso/progress.db"));
    };
    let configured = configured.strip_prefix("file:").unwrap_or(&configured);
    if configured.is_empty() {
        let step = if source == "--db" {
            "pass --db with a file path after file:".to_string()
        } else {
            format!("set {source} to a file path or unset it")
        };
        return Err(format!("database path must not be empty; {step}"));
    }

    let path = if let Some(remainder) = configured.strip_prefix("~/")
        && let Some(home) = env::var_os("HOME")
    {
        PathBuf::from(home).join(remainder)
    } else {
        PathBuf::from(configured)
    };
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(root.join(path))
    }
}
