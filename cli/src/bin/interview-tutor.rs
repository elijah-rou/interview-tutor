use clap::Parser;
use practice_cli::app::{AppState, Repository};
use practice_cli::{config, database, tui};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "interview-tutor",
    version,
    about = "Interactive algorithm practice browser"
)]
struct Cli {
    #[arg(long, help = "database file or file: URL")]
    db: Option<String>,
    #[arg(
        long = "set",
        value_name = "ID",
        help = "open this problem set at startup"
    )]
    problem_set: Option<String>,
    #[arg(long, value_name = "ID", help = "select an enabled language")]
    language: Option<String>,
    #[arg(
        long,
        help = "disable Codex without probing or spawning its executable"
    )]
    no_codex: bool,
}

fn run() -> Result<ExitCode, String> {
    let cli = Cli::parse();
    let root = config::resolve_root()?;
    let database_path = config::resolve_database_path(&root, cli.db.as_deref())?;
    let connection = database::open_database(&database_path, &root)?;
    if let Some(problem_set) = &cli.problem_set {
        database::get_problem_set(&connection, problem_set)?;
    }
    let languages = database::list_enabled_languages_bounded(
        &connection,
        database::RowLimit::new(practice_cli::app::model::MAX_ROWS)?,
    )?;
    if languages.is_empty() {
        return Err("no enabled languages".to_string());
    }
    let language_index = match cli.language {
        Some(requested) => languages
            .iter()
            .position(|item| item.slug == requested)
            .ok_or_else(|| format!("unknown or disabled language: {requested}"))?,
        None => languages
            .iter()
            .position(|item| item.slug == "python")
            .unwrap_or(0),
    };
    let mut state = AppState::new(languages, language_index);
    if cli.no_codex {
        state.disable_codex();
    }
    tui::runtime::run(
        state,
        Repository::new(connection),
        cli.problem_set,
        root,
        database_path,
    )
    .map(ExitCode::from)
}

fn main() -> ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_codex_is_an_explicit_opt_in_flag() {
        let default = Cli::try_parse_from(["interview-tutor"]).unwrap();
        assert!(!default.no_codex);

        let disabled =
            Cli::try_parse_from(["interview-tutor", "--language", "python", "--no-codex"]).unwrap();
        assert!(disabled.no_codex);
        assert_eq!(disabled.language.as_deref(), Some("python"));
    }
}
