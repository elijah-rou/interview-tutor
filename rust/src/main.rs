use std::env;
use std::process::ExitCode;

use blind_75::{run, PROBLEMS};

fn usage(program: &str) {
    eprintln!("Usage: {program} <slug>");
    eprintln!("       {program} --list");
}

fn main() -> ExitCode {
    let mut arguments = env::args();
    let program = arguments.next().unwrap_or_else(|| "blind-75".into());
    let Some(argument) = arguments.next() else {
        usage(&program);
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        usage(&program);
        return ExitCode::from(2);
    }

    if argument == "--list" {
        for problem in PROBLEMS {
            println!(
                "{:<6}  {:<62}  {}",
                problem.difficulty, problem.slug, problem.title
            );
        }
        return ExitCode::SUCCESS;
    }

    match run(&argument) {
        Ok(()) => {
            println!("PASS {argument}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            eprintln!("Run `{program} --list` to see valid slugs.");
            ExitCode::from(2)
        }
    }
}
