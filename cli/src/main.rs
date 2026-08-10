use clap::{Args, Parser, Subcommand};
use database::{NewProblem, Problem, ProblemUpdate};
use practice_cli::{catalog, database, runner};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};

#[derive(Parser)]
#[command(
    name = "practice",
    version,
    about = "Local algorithm practice catalog and progress CLI"
)]
struct Cli {
    #[arg(long, global = true, help = "database file or file: URL")]
    db: Option<String>,
    #[arg(
        long = "set",
        global = true,
        default_value = "blind75",
        value_name = "ID",
        help = "problem set for list/show/stats"
    )]
    problem_set: String,
    #[command(subcommand)]
    command: TopCommand,
}

#[derive(Subcommand)]
enum TopCommand {
    #[command(about = "list one problem set")]
    List(ListArgs),
    #[command(about = "show a set problem by slug or index")]
    Show { problem: String },
    #[command(about = "show set or global progress")]
    Stats(StatsArgs),
    #[command(about = "resolve and execute a problem")]
    Run(RunArgs),
    #[command(about = "compatibility execution command")]
    Test { language: String, problem: String },
    #[command(about = "initialize and show the database")]
    Db,
    #[command(about = "manage global problems")]
    Problems {
        #[command(subcommand)]
        command: ProblemsCommand,
    },
    #[command(about = "manage ordered problem sets")]
    Sets {
        #[command(subcommand)]
        command: SetsCommand,
    },
    #[command(name = "_record", hide = true)]
    Record(RecordArgs),
}

#[derive(Args)]
struct ListArgs {
    #[arg(long, value_parser = ["easy", "medium", "hard"])]
    difficulty: Option<String>,
    #[arg(long, help = "case-insensitive topic substring")]
    topic: Option<String>,
}

#[derive(Args)]
struct StatsArgs {
    #[arg(long, default_value = "any")]
    language: String,
    #[arg(long = "global", help = "deduplicate all global problems")]
    global_stats: bool,
}

#[derive(Args)]
struct RunArgs {
    language: String,
    #[arg(num_args = 1..)]
    selectors: Vec<String>,
}

#[derive(Args)]
struct RecordArgs {
    language: String,
    problem: String,
    #[arg(value_parser = ["pass", "fail", "error", "cancelled"])]
    result: String,
    duration_ms: i64,
    #[arg(long = "problem-set")]
    invoked_set: Option<String>,
    #[arg(long)]
    exit_code: Option<i32>,
}

#[derive(Subcommand)]
enum ProblemsCommand {
    List {
        #[arg(long = "all")]
        include_archived: bool,
    },
    Show {
        problem: String,
    },
    Add(ProblemAddArgs),
    Update(ProblemUpdateArgs),
    Delete {
        problem: String,
        #[arg(long, required = true)]
        yes: bool,
    },
    Adapter {
        problem: String,
        language: String,
        solution_path: String,
    },
}

#[derive(Args)]
struct ProblemAddArgs {
    problem: String,
    #[arg(long, required = true)]
    title: String,
    #[arg(long, required = true, value_parser = ["Easy", "Medium", "Hard"])]
    difficulty: String,
    #[arg(long, required = true)]
    topic: String,
    #[arg(long, conflicts_with = "statement_file", default_value = "")]
    statement: String,
    #[arg(long)]
    statement_file: Option<PathBuf>,
    #[arg(long)]
    leetcode_id: Option<i64>,
    #[arg(long, default_value = "")]
    leetcode_url: String,
    #[arg(long, default_value = "")]
    neetcode_url: String,
    #[arg(long)]
    premium: bool,
}

#[derive(Args)]
struct ProblemUpdateArgs {
    problem: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long, value_parser = ["Easy", "Medium", "Hard"])]
    difficulty: Option<String>,
    #[arg(long)]
    topic: Option<String>,
    #[arg(long, conflicts_with = "statement_file")]
    statement: Option<String>,
    #[arg(long)]
    statement_file: Option<PathBuf>,
    #[arg(long)]
    test_revision: Option<i64>,
    #[arg(long, conflicts_with = "clear_leetcode_id")]
    leetcode_id: Option<i64>,
    #[arg(long)]
    clear_leetcode_id: bool,
    #[arg(long, conflicts_with = "not_premium")]
    premium: bool,
    #[arg(long)]
    not_premium: bool,
    #[arg(long)]
    leetcode_url: Option<String>,
    #[arg(long)]
    neetcode_url: Option<String>,
}

#[derive(Subcommand)]
enum SetsCommand {
    List,
    Show {
        problem_set_id: String,
    },
    Create {
        problem_set_id: String,
        #[arg(long, required = true)]
        name: String,
        #[arg(long, default_value = "")]
        description: String,
    },
    Update {
        problem_set_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    Delete {
        problem_set_id: String,
        #[arg(long, required = true)]
        yes: bool,
    },
    Add {
        problem_set_id: String,
        problem: String,
        #[arg(long)]
        index: Option<i64>,
        #[arg(long)]
        section: Option<String>,
    },
    Move {
        problem_set_id: String,
        problem: String,
        #[arg(long, required = true)]
        index: i64,
    },
    Remove {
        problem_set_id: String,
        problem: String,
    },
}

struct Context {
    root: PathBuf,
    database_path: PathBuf,
    connection: Connection,
    problem_set: String,
}

fn resolve_root() -> Result<PathBuf, String> {
    if let Some(root) = env::var_os("PRACTICE_ROOT") {
        let root = PathBuf::from(root);
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

fn expand_home(value: &str) -> PathBuf {
    if let Some(remainder) = value.strip_prefix("~/")
        && let Some(home) = env::var_os("HOME")
    {
        return PathBuf::from(home).join(remainder);
    }
    PathBuf::from(value)
}

fn resolve_database_path(root: &Path, cli_path: Option<&str>) -> PathBuf {
    let configured = cli_path.map(str::to_string).or_else(|| {
        [
            "PRACTICE_DATABASE_URL",
            "PRACTICE_DB_PATH",
            "BLIND75_DATABASE_URL",
            "BLIND75_DB_PATH",
        ]
        .into_iter()
        .find_map(|name| env::var(name).ok())
    });
    let Some(configured) = configured else {
        return root.join(".turso/progress.db");
    };
    let configured = configured.strip_prefix("file:").unwrap_or(&configured);
    let path = expand_home(configured);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn print_table(headers: &[String], rows: &[Vec<String>]) -> Result<(), String> {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        if row.len() != headers.len() {
            return Err("table row does not match its header".to_string());
        }
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    println!(
        "{}",
        headers
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect::<Vec<_>>()
            .join("  ")
    );
    println!(
        "{}",
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for row in rows {
        println!(
            "{}",
            row.iter()
                .zip(&widths)
                .map(|(cell, width)| format!("{cell:<width$}"))
                .collect::<Vec<_>>()
                .join("  ")
        );
    }
    Ok(())
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

fn enabled_languages(connection: &Connection) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare("SELECT slug FROM languages WHERE enabled = 1 ORDER BY slug")
        .map_err(|error| error.to_string())?;
    statement
        .query_map([], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())
}

fn command_list(context: &Context, args: &ListArgs) -> Result<i32, String> {
    let members = database::list_set_members(&context.connection, &context.problem_set)?;
    let languages = enabled_languages(&context.connection)?;
    let mut completed = BTreeMap::new();
    for language in &languages {
        completed.insert(
            language.clone(),
            database::completed_problem_ids(&context.connection, Some(language))?,
        );
    }
    let difficulty = args.difficulty.as_deref();
    let topic = args.topic.as_ref().map(|value| value.to_lowercase());
    let mut rows = Vec::new();
    for problem in members {
        if difficulty.is_some_and(|value| problem.difficulty.to_lowercase() != value) {
            continue;
        }
        if topic
            .as_ref()
            .is_some_and(|value| !problem.topic.to_lowercase().contains(value))
        {
            continue;
        }
        let mut row = vec![
            problem.ordinal.unwrap_or_default().to_string(),
            problem.difficulty,
            problem.topic,
        ];
        for language in &languages {
            row.push(if completed[language].contains(&problem.id) {
                "yes".to_string()
            } else {
                "-".to_string()
            });
        }
        row.push(problem.slug);
        rows.push(row);
    }
    let mut headers = vec![
        "#".to_string(),
        "Difficulty".to_string(),
        "Topic".to_string(),
    ];
    headers.extend(languages.iter().map(|language| title_case(language)));
    headers.push("Problem".to_string());
    print_table(&headers, &rows)?;
    Ok(0)
}

fn print_problem(problem: &Problem, set_slug: Option<&str>) {
    println!("{}", problem.title);
    println!("Slug: {}", problem.slug);
    if let Some(set_slug) = set_slug {
        println!(
            "Problem set: {set_slug} #{}",
            problem.ordinal.unwrap_or_default()
        );
    }
    println!("Difficulty: {}", problem.difficulty);
    println!("Topic: {}", problem.topic);
    if problem.archived {
        println!("State: archived");
    }
    if !problem.leetcode_url.is_empty() {
        println!("LeetCode: {}", problem.leetcode_url);
    }
    if !problem.neetcode_url.is_empty() {
        println!("NeetCode: {}", problem.neetcode_url);
    }
    if !problem.statement_markdown.is_empty() {
        println!("\n{}", problem.statement_markdown);
    }
}

fn command_stats(context: &Context, args: &StatsArgs) -> Result<i32, String> {
    let (members, set_name) = if args.global_stats {
        (
            database::list_active_global_problems(&context.connection)?,
            "All Problems".to_string(),
        )
    } else {
        (
            database::list_set_members(&context.connection, &context.problem_set)?,
            database::get_problem_set(&context.connection, &context.problem_set)?.name,
        )
    };
    let language = (args.language != "any").then_some(args.language.as_str());
    if let Some(language) = language
        && !database::language_is_enabled(&context.connection, language)?
    {
        return Err(format!("unknown or disabled language: {language}"));
    }
    let completed = database::completed_problem_ids(&context.connection, language)?;
    let done = members
        .iter()
        .filter(|problem| completed.contains(&problem.id))
        .count();
    let total = members.len();
    let percentage = if total == 0 {
        0.0
    } else {
        done as f64 / total as f64 * 100.0
    };
    let label = language.unwrap_or("any language");
    println!("{set_name} progress ({label}): {done}/{total} ({percentage:.1}%)");

    print_stats_group(&members, &completed, true)?;
    print_stats_group(&members, &completed, false)?;
    Ok(0)
}

fn print_stats_group(
    members: &[Problem],
    completed: &HashSet<i64>,
    difficulty: bool,
) -> Result<(), String> {
    let mut groups: Vec<(String, Vec<&Problem>)> = Vec::new();
    for problem in members {
        let name = if difficulty {
            &problem.difficulty
        } else {
            &problem.topic
        };
        if let Some((_, group)) = groups.iter_mut().find(|(existing, _)| existing == name) {
            group.push(problem);
        } else {
            groups.push((name.clone(), vec![problem]));
        }
    }
    if difficulty {
        groups.sort_by_key(|(name, _)| {
            catalog::DIFFICULTIES
                .iter()
                .position(|difficulty| difficulty == name)
                .expect("database difficulty satisfies its CHECK constraint")
        });
    }
    if groups.is_empty() {
        return Ok(());
    }
    let heading = if difficulty {
        "By difficulty"
    } else {
        "By topic"
    };
    println!("\n{heading}");
    let rows = groups
        .iter()
        .map(|(name, group)| {
            let done = group
                .iter()
                .filter(|problem| completed.contains(&problem.id))
                .count();
            vec![
                name.clone(),
                done.to_string(),
                group.len().to_string(),
                format!("{:.1}%", done as f64 / group.len() as f64 * 100.0),
            ]
        })
        .collect::<Vec<_>>();
    print_table(
        &[
            title_case(heading.trim_start_matches("By ")),
            "Done".to_string(),
            "Total".to_string(),
            "Progress".to_string(),
        ],
        &rows,
    )
}

fn run_one(
    context: &Context,
    language: &str,
    reference: &str,
    set_slug: Option<&str>,
) -> Result<i32, String> {
    let plan = runner::plan_execution(
        &context.connection,
        &context.root,
        language,
        reference,
        set_slug,
    )?;
    let result = runner::execute_plan(&plan, &context.database_path)?;
    runner::record_execution(&context.connection, &plan, &result)?;
    Ok(result.status_code)
}

fn command_run(context: &Context, args: &RunArgs) -> Result<i32, String> {
    match args.selectors.as_slice() {
        [reference] => run_one(context, &args.language, reference, None),
        [set_slug, reference] => run_one(context, &args.language, reference, Some(set_slug)),
        _ => Err("run expects LANGUAGE PROBLEM or LANGUAGE SET PROBLEM_OR_INDEX".to_string()),
    }
}

fn command_test(context: &Context, language: &str, problem: &str) -> Result<i32, String> {
    if problem != "all" {
        return run_one(context, language, problem, Some(&context.problem_set));
    }
    let mut status = 0;
    for problem in database::list_set_members(&context.connection, &context.problem_set)? {
        let result = run_one(context, language, &problem.slug, Some(&context.problem_set))?;
        if result != 0 {
            status = result;
        }
    }
    Ok(status)
}

fn command_problems(context: &Context, command: &ProblemsCommand) -> Result<i32, String> {
    match command {
        ProblemsCommand::List { include_archived } => {
            let rows = database::list_global_problems(&context.connection, *include_archived)?;
            print_table(
                &[
                    "Difficulty".to_string(),
                    "Topic".to_string(),
                    "Languages".to_string(),
                    "State".to_string(),
                    "Problem".to_string(),
                ],
                &rows
                    .into_iter()
                    .map(|row| {
                        vec![
                            row.problem.difficulty,
                            row.problem.topic,
                            row.languages,
                            if row.problem.archived {
                                "archived"
                            } else {
                                "active"
                            }
                            .to_string(),
                            row.problem.slug,
                        ]
                    })
                    .collect::<Vec<_>>(),
            )?;
        }
        ProblemsCommand::Show { problem } => {
            let problem = database::resolve_problem(&context.connection, problem, None)?;
            print_problem(&problem, None);
            let adapters = {
                let mut statement = context
                    .connection
                    .prepare(
                        "SELECT l.slug, i.solution_path \
                         FROM problem_implementations AS i \
                         JOIN languages AS l ON l.id = i.language_id \
                         WHERE i.problem_id = ? AND i.enabled = 1 ORDER BY l.slug",
                    )
                    .map_err(|error| error.to_string())?;
                statement
                    .query_map(params![problem.id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|error| error.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?
            };
            if !adapters.is_empty() {
                println!("\nAdapters");
                for (language, path) in adapters {
                    println!("{language}: {path}");
                }
            }
        }
        ProblemsCommand::Add(args) => {
            let statement = read_statement(args.statement_file.as_deref(), Some(&args.statement))?;
            database::create_problem(
                &context.connection,
                &NewProblem {
                    slug: &args.problem,
                    title: &args.title,
                    difficulty: &args.difficulty,
                    topic: &args.topic,
                    statement_markdown: &statement,
                    leetcode_id: args.leetcode_id,
                    leetcode_url: &args.leetcode_url,
                    neetcode_url: &args.neetcode_url,
                    premium: args.premium,
                },
            )?;
            println!("Added problem: {}", args.problem);
        }
        ProblemsCommand::Update(args) => {
            let statement = if args.statement_file.is_some() || args.statement.is_some() {
                Some(read_statement(
                    args.statement_file.as_deref(),
                    args.statement.as_ref(),
                )?)
            } else {
                None
            };
            let leetcode_id = if args.clear_leetcode_id {
                Some(None)
            } else {
                args.leetcode_id.map(Some)
            };
            let premium = if args.premium {
                Some(true)
            } else if args.not_premium {
                Some(false)
            } else {
                None
            };
            database::update_problem(
                &context.connection,
                &args.problem,
                &ProblemUpdate {
                    title: args.title.as_deref(),
                    difficulty: args.difficulty.as_deref(),
                    topic: args.topic.as_deref(),
                    statement_markdown: statement.as_deref(),
                    test_revision: args.test_revision,
                    leetcode_id,
                    premium,
                    leetcode_url: args.leetcode_url.as_deref(),
                    neetcode_url: args.neetcode_url.as_deref(),
                },
            )?;
            println!("Updated problem: {}", args.problem);
        }
        ProblemsCommand::Delete { problem, yes } => {
            assert!(*yes, "clap requires --yes");
            database::delete_problem(&context.connection, problem)?;
            println!("Deleted problem: {problem}");
        }
        ProblemsCommand::Adapter {
            problem,
            language,
            solution_path,
        } => {
            register_adapter(context, problem, language, solution_path)?;
            println!("Registered {language} adapter for {problem}");
        }
    }
    Ok(0)
}

fn read_statement(path: Option<&Path>, direct: Option<&String>) -> Result<String, String> {
    if let Some(path) = path {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let maximum_bytes = u64::try_from(database::MAX_STATEMENT_LENGTH)
            .expect("statement limit fits u64")
            .saturating_mul(4)
            .saturating_add(1);
        if metadata.len() > maximum_bytes {
            return Err(format!(
                "problem statement exceeds {} characters",
                database::MAX_STATEMENT_LENGTH
            ));
        }
        fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
    } else {
        Ok(direct.cloned().unwrap_or_default())
    }
}

fn register_adapter(
    context: &Context,
    problem: &str,
    language: &str,
    solution_path: &str,
) -> Result<(), String> {
    let solution = context.root.join(solution_path);
    if !solution.is_file() {
        return Err(format!(
            "solution file does not exist: {}",
            solution.display()
        ));
    }
    let runner_path: Option<String> = context
        .connection
        .query_row(
            "SELECT runner_path FROM languages WHERE slug = ? AND enabled = 1",
            params![language],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(runner_path) = runner_path else {
        return Err(format!("unknown or disabled language: {language}"));
    };
    let runner = context.root.join(runner_path);
    let output = ProcessCommand::new(&runner)
        .arg("--list")
        .current_dir(
            runner
                .parent()
                .ok_or_else(|| "language runner has no parent".to_string())?,
        )
        .output()
        .map_err(|error| format!("language runner discovery failed: {language}: {error}"))?;
    if !output.status.success() {
        return Err(format!("language runner discovery failed: {language}"));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| format!("language runner emitted invalid UTF-8: {language}"))?;
    if !stdout.lines().any(|slug| slug == problem) {
        return Err(format!(
            "{language} runner does not expose problem adapter: {problem}"
        ));
    }
    database::add_implementation(&context.connection, problem, language, solution_path)
}

fn command_sets(context: &Context, command: &SetsCommand) -> Result<i32, String> {
    match command {
        SetsCommand::List => {
            let rows = database::list_problem_sets(&context.connection)?;
            print_table(
                &["ID".to_string(), "Name".to_string(), "Problems".to_string()],
                &rows
                    .into_iter()
                    .map(|(set, count)| vec![set.slug, set.name, count.to_string()])
                    .collect::<Vec<_>>(),
            )?;
        }
        SetsCommand::Show { problem_set_id } => {
            let problem_set = database::get_problem_set(&context.connection, problem_set_id)?;
            println!("{}", problem_set.name);
            println!("ID: {}", problem_set.slug);
            if !problem_set.description.is_empty() {
                println!("{}", problem_set.description);
            }
            let members = database::list_set_members(&context.connection, problem_set_id)?;
            if !members.is_empty() {
                println!();
                print_table(
                    &[
                        "#".to_string(),
                        "Difficulty".to_string(),
                        "Topic".to_string(),
                        "Problem".to_string(),
                    ],
                    &members
                        .into_iter()
                        .map(|problem| {
                            vec![
                                problem.ordinal.unwrap_or_default().to_string(),
                                problem.difficulty,
                                problem.topic,
                                problem.slug,
                            ]
                        })
                        .collect::<Vec<_>>(),
                )?;
            }
        }
        SetsCommand::Create {
            problem_set_id,
            name,
            description,
        } => {
            database::create_problem_set(&context.connection, problem_set_id, name, description)?;
            println!("Created problem set: {problem_set_id}");
        }
        SetsCommand::Update {
            problem_set_id,
            name,
            description,
        } => {
            database::update_problem_set(
                &context.connection,
                problem_set_id,
                name.as_deref(),
                description.as_deref(),
            )?;
            println!("Updated problem set: {problem_set_id}");
        }
        SetsCommand::Delete {
            problem_set_id,
            yes,
        } => {
            assert!(*yes, "clap requires --yes");
            database::delete_problem_set(&context.connection, problem_set_id)?;
            println!("Deleted problem set: {problem_set_id}");
        }
        SetsCommand::Add {
            problem_set_id,
            problem,
            index,
            section,
        } => {
            database::add_set_member(
                &context.connection,
                problem_set_id,
                problem,
                *index,
                section.as_deref(),
            )?;
            println!("Added {problem} to {problem_set_id}");
        }
        SetsCommand::Move {
            problem_set_id,
            problem,
            index,
        } => {
            database::move_set_member(&context.connection, problem_set_id, problem, *index)?;
            println!("Moved {problem} to #{index} in {problem_set_id}");
        }
        SetsCommand::Remove {
            problem_set_id,
            problem,
        } => {
            database::remove_set_member(&context.connection, problem_set_id, problem)?;
            println!("Removed {problem} from {problem_set_id}");
        }
    }
    Ok(0)
}

fn dispatch(cli: Cli) -> Result<i32, String> {
    let root = resolve_root()?;
    let database_path = resolve_database_path(&root, cli.db.as_deref());
    let connection = database::open_database(&database_path, &root)?;
    let context = Context {
        root,
        database_path,
        connection,
        problem_set: cli.problem_set,
    };
    match &cli.command {
        TopCommand::List(args) => command_list(&context, args),
        TopCommand::Show { problem } => {
            let problem = database::resolve_problem(
                &context.connection,
                problem,
                Some(&context.problem_set),
            )?;
            print_problem(&problem, Some(&context.problem_set));
            Ok(0)
        }
        TopCommand::Stats(args) => command_stats(&context, args),
        TopCommand::Run(args) => command_run(&context, args),
        TopCommand::Test { language, problem } => command_test(&context, language, problem),
        TopCommand::Db => {
            println!("{}", context.database_path.display());
            println!(
                "Turso local server: turso dev --db-file {}",
                context.database_path.display()
            );
            Ok(0)
        }
        TopCommand::Problems { command } => command_problems(&context, command),
        TopCommand::Sets { command } => command_sets(&context, command),
        TopCommand::Record(args) => {
            database::record_attempt(
                &context.connection,
                &args.problem,
                &args.language,
                &args.result,
                args.duration_ms,
                args.exit_code,
                args.invoked_set.as_deref(),
            )?;
            Ok(0)
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(code) => ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(2)),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}
