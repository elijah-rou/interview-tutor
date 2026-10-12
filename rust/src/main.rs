use std::env;
use std::process::ExitCode;

use local_judge::{find, run, PROBLEMS};

fn usage(program: &str) {
    eprintln!("Usage: {program} <slug>");
    eprintln!("       {program} --list");
    eprintln!("       {program} --cases <slug>");
    eprintln!("       {program} --case <slug> <case>");
}

fn main() -> ExitCode {
    let mut arguments = env::args();
    let program = arguments
        .next()
        .unwrap_or_else(|| "local-judge-rust".into());
    let arguments: Vec<String> = arguments.collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();

    let result = match arguments.as_slice() {
        ["--list"] => {
            for problem in PROBLEMS {
                println!("{}", problem.slug);
            }
            return ExitCode::SUCCESS;
        }
        ["--cases", slug] => find(slug).map(|problem| {
            for name in problem.case_names() {
                println!("{name}");
            }
        }),
        ["--case", slug, name] => find(slug)
            .and_then(|problem| problem.run_case(name))
            .map(|()| println!("PASS {slug}")),
        [slug] if !slug.starts_with('-') => run(slug).map(|()| println!("PASS {slug}")),
        _ => {
            usage(&program);
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            eprintln!("Run `{program} --list` to see valid slugs.");
            ExitCode::from(2)
        }
    }
}
