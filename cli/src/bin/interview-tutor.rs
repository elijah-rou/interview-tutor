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
}

fn run() -> Result<(), String> {
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
    let state = AppState::new(languages, language_index);
    tui::runtime::run(state, Repository::new(connection), cli.problem_set)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
