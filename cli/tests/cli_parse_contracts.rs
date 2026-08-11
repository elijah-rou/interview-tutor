use std::process::{Command, Output};

fn practice(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_practice"))
        .args(arguments)
        .output()
        .expect("practice binary executes")
}

fn output_text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("CLI output is UTF-8")
}

#[test]
fn typed_values_keep_public_possible_values_in_help() {
    let add_help = practice(&["problems", "add", "custom", "--help"]);
    assert!(add_help.status.success());
    let add_stdout = output_text(&add_help.stdout);
    assert!(add_stdout.contains("--difficulty <DIFFICULTY>"));
    assert!(add_stdout.contains("[possible values: Easy, Medium, Hard]"));

    let update_help = practice(&["problems", "update", "custom", "--help"]);
    assert!(update_help.status.success());
    assert!(output_text(&update_help.stdout).contains("[possible values: Easy, Medium, Hard]"));

    let record_help = practice(&["_record", "--help"]);
    assert!(record_help.status.success());
    let record_stdout = output_text(&record_help.stdout);
    assert!(record_stdout.contains("<RESULT>"));
    assert!(record_stdout.contains("[possible values: pass, fail, error, cancelled]"));
}

#[test]
fn typed_values_keep_public_invalid_value_diagnostics() {
    let invalid_difficulty = practice(&[
        "problems",
        "add",
        "custom",
        "--title",
        "Custom",
        "--difficulty",
        "Extreme",
        "--topic",
        "Arrays",
    ]);
    assert_eq!(invalid_difficulty.status.code(), Some(2));
    let difficulty_stderr = output_text(&invalid_difficulty.stderr);
    assert!(difficulty_stderr.contains(
        "invalid value 'Extreme' for '--difficulty <DIFFICULTY>'\n  [possible values: Easy, Medium, Hard]"
    ));

    let invalid_outcome = practice(&["_record", "python", "custom", "timeout", "1"]);
    assert_eq!(invalid_outcome.status.code(), Some(2));
    let outcome_stderr = output_text(&invalid_outcome.stderr);
    assert!(outcome_stderr.contains(
        "invalid value 'timeout' for '<RESULT>'\n  [possible values: pass, fail, error, cancelled]"
    ));
}
