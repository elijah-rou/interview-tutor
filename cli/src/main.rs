use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{Args, Parser, Subcommand};
use database::{AttemptOutcome, Difficulty, NewProblem, Problem, ProblemUpdate, ProgressScope};
use practice_cli::{config, database, runner};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn difficulty_parser() -> impl TypedValueParser<Value = Difficulty> {
    PossibleValuesParser::new(["Easy", "Medium", "Hard"])
        .map(|value| Difficulty::from_str(&value).expect("possible difficulty values parse"))
}

fn attempt_outcome_parser() -> impl TypedValueParser<Value = AttemptOutcome> {
    PossibleValuesParser::new(["pass", "fail", "error", "cancelled"]).map(|value| {
        AttemptOutcome::from_str(&value).expect("possible attempt outcome values parse")
    })
}

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
    #[arg(value_parser = attempt_outcome_parser())]
    result: AttemptOutcome,
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
    #[arg(long, required = true, value_parser = difficulty_parser())]
    difficulty: Difficulty,
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
    #[arg(long, value_parser = difficulty_parser())]
    difficulty: Option<Difficulty>,
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

fn command_list(context: &Context, args: &ListArgs) -> Result<i32, String> {
    let members = database::list_set_members(&context.connection, &context.problem_set)?;
    let languages = database::list_enabled_languages(&context.connection)?;
    let mut completed = BTreeMap::new();
    for language in &languages {
        completed.insert(
            language.slug.clone(),
            database::completed_problem_ids(&context.connection, Some(&language.slug))?,
        );
    }
    let difficulty = args.difficulty.as_deref();
    let topic = args.topic.as_ref().map(|value| value.to_lowercase());
    let mut rows = Vec::new();
    for member in members {
        let problem = member.problem;
        if difficulty.is_some_and(|value| problem.difficulty.to_string().to_lowercase() != value) {
            continue;
        }
        if topic
            .as_ref()
            .is_some_and(|value| !problem.topic.to_lowercase().contains(value))
        {
            continue;
        }
        let mut row = vec![
            member.ordinal.get().to_string(),
            problem.difficulty.to_string(),
            problem.topic,
        ];
        for language in &languages {
            row.push(if completed[&language.slug].contains(&problem.id) {
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
    headers.extend(
        languages
            .iter()
            .map(|language| language.display_name.clone()),
    );
    headers.push("Problem".to_string());
    print_table(&headers, &rows)?;
    Ok(0)
}

fn print_problem(problem: &Problem, set_membership: Option<(&str, database::PositiveOrdinal)>) {
    println!("{}", problem.title);
    println!("Slug: {}", problem.slug);
    if let Some((set_slug, ordinal)) = set_membership {
        println!("Problem set: {set_slug} #{}", ordinal.get());
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
    let (scope, set_name) = if args.global_stats {
        (ProgressScope::Global, "All Problems".to_string())
    } else {
        (
            ProgressScope::ProblemSet(&context.problem_set),
            database::get_problem_set(&context.connection, &context.problem_set)?.name,
        )
    };
    let language = (args.language != "any").then_some(args.language.as_str());
    let summary = database::progress_summary(&context.connection, scope, language)?;
    let overall_percentage = percentage(summary.completed, summary.total);
    let label = language.unwrap_or("any language");
    println!(
        "{set_name} progress ({label}): {}/{} ({overall_percentage:.1}%)",
        summary.completed, summary.total
    );

    if !summary.by_difficulty.is_empty() {
        println!("\nBy difficulty");
        let rows = summary
            .by_difficulty
            .iter()
            .map(|group| {
                vec![
                    group.difficulty.to_string(),
                    group.completed.to_string(),
                    group.total.to_string(),
                    format!("{:.1}%", percentage(group.completed, group.total)),
                ]
            })
            .collect::<Vec<_>>();
        print_progress_table("Difficulty", &rows)?;
    }
    if !summary.by_topic.is_empty() {
        println!("\nBy topic");
        let rows = summary
            .by_topic
            .iter()
            .map(|group| {
                vec![
                    group.topic.clone(),
                    group.completed.to_string(),
                    group.total.to_string(),
                    format!("{:.1}%", percentage(group.completed, group.total)),
                ]
            })
            .collect::<Vec<_>>();
        print_progress_table("Topic", &rows)?;
    }
    Ok(0)
}

fn percentage(completed: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        completed as f64 / total as f64 * 100.0
    }
}

fn print_progress_table(group_heading: &str, rows: &[Vec<String>]) -> Result<(), String> {
    print_table(
        &[
            group_heading.to_string(),
            "Done".to_string(),
            "Total".to_string(),
            "Progress".to_string(),
        ],
        rows,
    )
}

struct ExecutionSignalHandlers {
    registrations: Vec<signal_hook::SigId>,
    received: Arc<AtomicUsize>,
}

impl ExecutionSignalHandlers {
    fn register(cancellation: &runner::CancellationToken) -> Result<Self, String> {
        let received = Arc::new(AtomicUsize::new(0));
        let mut registrations = Vec::with_capacity(2);
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            let received_for_handler = Arc::clone(&received);
            let cancellation_flag = cancellation.signal_flag();
            // SAFETY: the handler only performs lock-free atomic stores through owned Arcs.
            let registration = unsafe {
                signal_hook::low_level::register(signal, move || {
                    received_for_handler.store(signal as usize, Ordering::Release);
                    cancellation_flag.store(true, Ordering::Release);
                })
            }
            .map_err(|error| format!("cannot register execution signal handler: {error}"));
            match registration {
                Ok(registration) => registrations.push(registration),
                Err(error) => {
                    for registration in registrations {
                        signal_hook::low_level::unregister(registration);
                    }
                    return Err(error);
                }
            }
        }
        assert_eq!(registrations.len(), 2);
        Ok(Self {
            registrations,
            received,
        })
    }

    fn exit_code(&self) -> Option<i32> {
        match self.received.load(Ordering::Acquire) as i32 {
            signal_hook::consts::SIGINT => Some(130),
            signal_hook::consts::SIGTERM => Some(143),
            0 => None,
            signal => Some(128 + signal),
        }
    }

    fn unregister(&mut self) -> Result<(), String> {
        let mut failed = false;
        for registration in self.registrations.drain(..) {
            failed |= !signal_hook::low_level::unregister(registration);
        }
        if failed {
            Err("cannot unregister an execution signal handler".to_string())
        } else {
            Ok(())
        }
    }
}

impl Drop for ExecutionSignalHandlers {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            let _ = signal_hook::low_level::unregister(registration);
        }
    }
}

struct BlockedExecutionSignals {
    prior_mask: libc::sigset_t,
    active: bool,
}

impl BlockedExecutionSignals {
    fn block() -> Result<Self, String> {
        // SAFETY: all signal sets are initialized before use and pthread_sigmask only changes the
        // calling thread's mask.
        unsafe {
            let mut signals = std::mem::zeroed();
            if libc::sigemptyset(&mut signals) != 0 {
                return Err(format!(
                    "cannot initialize execution signal set: {}",
                    io::Error::last_os_error()
                ));
            }
            for signal in [libc::SIGINT, libc::SIGTERM] {
                if libc::sigaddset(&mut signals, signal) != 0 {
                    return Err(format!(
                        "cannot add execution signal to set: {}",
                        io::Error::last_os_error()
                    ));
                }
            }
            let mut prior_mask = std::mem::zeroed();
            let error = libc::pthread_sigmask(libc::SIG_BLOCK, &signals, &mut prior_mask);
            if error != 0 {
                return Err(format!(
                    "cannot block execution signals: {}",
                    io::Error::from_raw_os_error(error)
                ));
            }
            Ok(Self {
                prior_mask,
                active: true,
            })
        }
    }

    fn pending_exit_code(&self) -> Result<Option<i32>, String> {
        // SAFETY: pending is initialized by sigpending before sigismember reads it.
        unsafe {
            let mut pending = std::mem::zeroed();
            if libc::sigpending(&mut pending) != 0 {
                return Err(format!(
                    "cannot inspect pending execution signals: {}",
                    io::Error::last_os_error()
                ));
            }
            for (signal, exit_code) in [(libc::SIGINT, 130), (libc::SIGTERM, 143)] {
                match libc::sigismember(&pending, signal) {
                    1 => return Ok(Some(exit_code)),
                    0 => {}
                    _ => {
                        return Err(format!(
                            "cannot inspect pending execution signal: {}",
                            io::Error::last_os_error()
                        ));
                    }
                }
            }
            Ok(None)
        }
    }

    fn consume_pending(&self) -> Result<(), String> {
        // SAFETY: pending and wait_set are initialized before use. sigwait is called only for a
        // signal proven pending and blocked in this thread.
        unsafe {
            let mut pending = std::mem::zeroed();
            if libc::sigpending(&mut pending) != 0 {
                return Err(format!(
                    "cannot inspect pending execution signals for cleanup: {}",
                    io::Error::last_os_error()
                ));
            }
            for signal in [libc::SIGINT, libc::SIGTERM] {
                match libc::sigismember(&pending, signal) {
                    0 => continue,
                    1 => {}
                    _ => {
                        return Err(format!(
                            "cannot inspect pending execution signal for cleanup: {}",
                            io::Error::last_os_error()
                        ));
                    }
                }
                let mut wait_set = std::mem::zeroed();
                if libc::sigemptyset(&mut wait_set) != 0
                    || libc::sigaddset(&mut wait_set, signal) != 0
                {
                    return Err(format!(
                        "cannot initialize pending execution signal cleanup: {}",
                        io::Error::last_os_error()
                    ));
                }
                let mut received = 0;
                let error = libc::sigwait(&wait_set, &mut received);
                if error != 0 {
                    return Err(format!(
                        "cannot consume pending execution signal: {}",
                        io::Error::from_raw_os_error(error)
                    ));
                }
                if received != signal {
                    return Err(format!(
                        "consumed unexpected execution signal {received}; expected {signal}"
                    ));
                }
            }
            Ok(())
        }
    }

    fn restore(mut self) -> Result<(), String> {
        // SAFETY: prior_mask was initialized by the successful pthread_sigmask call in block.
        let error = unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.prior_mask, std::ptr::null_mut())
        };
        if error != 0 {
            return Err(format!(
                "cannot restore execution signal mask: {}",
                io::Error::from_raw_os_error(error)
            ));
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for BlockedExecutionSignals {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: prior_mask was initialized by the successful pthread_sigmask call in block.
            unsafe {
                libc::pthread_sigmask(libc::SIG_SETMASK, &self.prior_mask, std::ptr::null_mut());
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunOneResult {
    Completed(i32),
    Interrupted(i32),
}

impl RunOneResult {
    fn status_code(self) -> i32 {
        match self {
            Self::Completed(status) | Self::Interrupted(status) => status,
        }
    }
}

fn run_one(
    context: &Context,
    language: &str,
    reference: &str,
    set_slug: Option<&str>,
) -> Result<RunOneResult, String> {
    let plan = runner::plan_execution(
        &context.connection,
        &context.root,
        language,
        reference,
        set_slug,
    )?;
    let cancellation = runner::CancellationToken::new();
    let mut signal_handlers = ExecutionSignalHandlers::register(&cancellation)?;
    let execution = runner::execute(
        &plan,
        &context.database_path,
        &runner::ExecutionLimits::default(),
        &cancellation,
        None,
    );
    let mut result = execution?;
    if cancellation.is_cancelled() {
        result.termination = runner::Termination::Cancelled;
    }
    let blocked_signals = BlockedExecutionSignals::block()?;
    let finalization: Result<RunOneResult, String> = (|| {
        let attempt_id = runner::record_execution(&context.connection, &plan, &result)?;
        let pending_exit_code = blocked_signals.pending_exit_code()?;
        let signal_exit_code = signal_handlers.exit_code().or(pending_exit_code);
        let signal_cancelled = cancellation.is_cancelled() || signal_exit_code.is_some();
        signal_handlers.unregister()?;
        if signal_cancelled {
            let signal_exit_code = signal_exit_code.unwrap_or(130);
            database::finalize_attempt_cancelled(
                &context.connection,
                attempt_id,
                signal_exit_code,
            )?;
            result.termination = runner::Termination::Cancelled;
        }
        io::stderr()
            .write_all(result.display_output.as_bytes())
            .map_err(|error| format!("cannot write runner output: {error}"))?;
        Ok(if signal_cancelled {
            RunOneResult::Interrupted(signal_exit_code.unwrap_or(130))
        } else {
            RunOneResult::Completed(result.status_code())
        })
    })();
    let consume_result = blocked_signals.consume_pending();
    let restore_result = blocked_signals.restore();
    let mut errors = Vec::with_capacity(3);
    let run_result = match finalization {
        Ok(result) => Some(result),
        Err(error) => {
            errors.push(error);
            None
        }
    };
    if let Err(error) = consume_result {
        errors.push(error);
    }
    if let Err(error) = restore_result {
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(run_result.expect("successful finalization produces a run result"))
    } else {
        Err(errors.join("; "))
    }
}

fn command_run(context: &Context, args: &RunArgs) -> Result<i32, String> {
    match args.selectors.as_slice() {
        [reference] => {
            run_one(context, &args.language, reference, None).map(RunOneResult::status_code)
        }
        [set_slug, reference] => run_one(context, &args.language, reference, Some(set_slug))
            .map(RunOneResult::status_code),
        _ => Err("run expects LANGUAGE PROBLEM or LANGUAGE SET PROBLEM_OR_INDEX".to_string()),
    }
}

fn command_test(context: &Context, language: &str, problem: &str) -> Result<i32, String> {
    if problem != "all" {
        return run_one(context, language, problem, Some(&context.problem_set))
            .map(RunOneResult::status_code);
    }
    let mut status = 0;
    for member in database::list_set_members(&context.connection, &context.problem_set)? {
        match run_one(
            context,
            language,
            &member.problem.slug,
            Some(&context.problem_set),
        )? {
            RunOneResult::Completed(result) => {
                if result != 0 {
                    status = result;
                }
            }
            RunOneResult::Interrupted(result) => return Ok(result),
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
                            row.problem.difficulty.to_string(),
                            row.problem.topic,
                            if row.implementations.is_empty() {
                                "-".to_string()
                            } else {
                                row.implementations
                                    .iter()
                                    .map(|implementation| implementation.language.slug.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            },
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
            let problem = database::resolve_problem(&context.connection, problem, None)?.problem;
            print_problem(&problem, None);
            let implementations =
                database::list_enabled_implementations(&context.connection, problem.id)?;
            if !implementations.is_empty() {
                println!("\nAdapters");
                for implementation in implementations {
                    println!(
                        "{}: {}",
                        implementation.language.slug, implementation.solution_path
                    );
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
                    difficulty: args.difficulty,
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
                    difficulty: args.difficulty,
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
    let resolved_problem = database::resolve_problem(&context.connection, problem, None)?.problem;
    if resolved_problem.managed {
        return Err(format!("shipped problem is read-only: {problem}"));
    }
    let solution = context.root.join(solution_path);
    if !solution.is_file() {
        return Err(format!(
            "solution file does not exist: {}",
            solution.display()
        ));
    }
    let enabled_language = database::get_enabled_language(&context.connection, language)?;
    let runner_path = context.root.join(enabled_language.runner_path);
    let discovery_limits = runner::ExecutionLimits {
        wall_timeout: std::time::Duration::from_secs(5),
        display_output_bytes: 64 * 1024,
        ..runner::ExecutionLimits::default()
    };
    let adapters = runner::discover_adapters(
        &runner_path,
        &discovery_limits,
        &runner::CancellationToken::new(),
    )
    .map_err(|error| format!("language runner discovery failed: {language}: {error}"))?;
    if !adapters.iter().any(|slug| slug == problem) {
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
                        .map(|member| {
                            vec![
                                member.ordinal.get().to_string(),
                                member.problem.difficulty.to_string(),
                                member.problem.topic,
                                member.problem.slug,
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
    let root = config::resolve_root()?;
    let database_path = config::resolve_database_path(&root, cli.db.as_deref())?;
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
            let resolved = database::resolve_problem(
                &context.connection,
                problem,
                Some(&context.problem_set),
            )?;
            let membership = resolved
                .membership
                .as_ref()
                .map(|member| (context.problem_set.as_str(), member.ordinal));
            print_problem(&resolved.problem, membership);
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
                args.result,
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
