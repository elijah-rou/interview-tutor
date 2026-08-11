use std::env;
use std::path::{Path, PathBuf};

const DATABASE_ENVIRONMENT_VARIABLES: [&str; 4] = [
    "PRACTICE_DATABASE_URL",
    "PRACTICE_DB_PATH",
    "BLIND75_DATABASE_URL",
    "BLIND75_DB_PATH",
];

pub fn resolve_root() -> Result<PathBuf, String> {
    if let Some(configured_root) = env::var_os("PRACTICE_ROOT")
        && !configured_root.is_empty()
    {
        let root = PathBuf::from(configured_root);
        if root.is_dir() {
            return Ok(root);
        }
        return Err(format!("project root does not exist: {}", root.display()));
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "CLI manifest has no project parent".to_string())
}

pub fn resolve_database_path(root: &Path, cli_path: Option<&str>) -> Result<PathBuf, String> {
    if cli_path == Some("") {
        return Err("database path must not be empty".to_string());
    }
    let configured = cli_path.map(str::to_string).or_else(|| {
        DATABASE_ENVIRONMENT_VARIABLES
            .into_iter()
            .find_map(|name| env::var(name).ok().filter(|value| !value.is_empty()))
    });
    let Some(configured) = configured else {
        return Ok(root.join(".turso/progress.db"));
    };
    let configured = configured.strip_prefix("file:").unwrap_or(&configured);
    if configured.is_empty() {
        return Err("database path must not be empty".to_string());
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
