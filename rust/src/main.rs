use std::env;
use std::process::ExitCode;

use local_judge::{run, PROBLEMS};

fn usage(program: &str) {
    eprintln!("Usage: {program} <slug>");
    eprintln!("       {program} --list");
}

fn main() -> ExitCode {
    let mut arguments = env::args();
    let program = arguments
        .next()
        .unwrap_or_else(|| "local-judge-rust".into());
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
            println!("{}", problem.slug);
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
