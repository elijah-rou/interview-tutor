use clap::Parser;
use practice_cli::app::{AppState, Repository};
use practice_cli::{config, database, tui};
use std::path::PathBuf;
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
        value_name = "PATH",
        help = "required clean Neovim executable used by the embedded editor"
    )]
    neovim: Option<PathBuf>,
    #[arg(long, value_enum, value_name = "BACKEND", help = "interviewer backend")]
    interviewer: Option<practice_cli::interviewer::Backend>,
    #[arg(
        long,
        conflicts_with = "interviewer",
        help = "legacy alias for --interviewer none"
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
    let interviewer_backend =
        practice_cli::interviewer::selection::from_environment(cli.interviewer, cli.no_codex)?;
    let neovim_executable = practice_cli::neovim::resolve_executable(cli.neovim.as_deref())?;
    let state = AppState::new_with_interviewer(languages, language_index, interviewer_backend);
    #[cfg(debug_assertions)]
    let disposition_probe =
        practice_cli::signals::test_support::DispositionProbe::from_environment()?;
    let result = tui::runtime::run(
        state,
        Repository::new(connection),
        cli.problem_set,
        root,
        database_path,
        neovim_executable,
    );
    #[cfg(debug_assertions)]
    if let Some(probe) = disposition_probe {
        probe.verify()?;
    }
    result.map(ExitCode::from)
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
    fn interviewer_selector_and_legacy_flag_parse_without_resolution_side_effects() {
        let default = Cli::try_parse_from(["interview-tutor"]).unwrap();
        assert!(!default.no_codex);
        assert_eq!(default.interviewer, None);

        let disabled =
            Cli::try_parse_from(["interview-tutor", "--language", "python", "--no-codex"]).unwrap();
        assert!(disabled.no_codex);
        assert_eq!(disabled.language.as_deref(), Some("python"));

        let codex = Cli::try_parse_from(["interview-tutor", "--interviewer", "codex"]).unwrap();
        assert_eq!(
            codex.interviewer,
            Some(practice_cli::interviewer::Backend::Codex)
        );
        assert!(
            Cli::try_parse_from(["interview-tutor", "--interviewer", "codex", "--no-codex",])
                .is_err()
        );
        assert!(Cli::try_parse_from(["interview-tutor", "--interviewer", "Codex"]).is_err());

        let selected =
            Cli::try_parse_from(["interview-tutor", "--neovim", "/usr/bin/nvim"]).unwrap();
        assert_eq!(
            selected.neovim.as_deref(),
            Some(std::path::Path::new("/usr/bin/nvim"))
        );
    }
}
