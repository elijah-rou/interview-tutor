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

    let show_help = practice(&["attempts", "show", "--help"]);
    assert!(show_help.status.success());
    let show_stdout = output_text(&show_help.stdout);
    assert!(show_stdout.contains("--stream <STREAM>"));
    assert!(show_stdout.contains("[possible values: stdout, stderr]"));
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

    let invalid_stream = practice(&["attempts", "show", "1", "--case", "1", "--stream", "both"]);
    assert_eq!(invalid_stream.status.code(), Some(2));
    let stream_stderr = output_text(&invalid_stream.stderr);
    assert!(stream_stderr.contains(
        "invalid value 'both' for '--stream <STREAM>'\n  [possible values: stdout, stderr]"
    ));

    let invalid_limit = practice(&["attempts", "list", "two-sum", "--limit", "0"]);
    assert_eq!(invalid_limit.status.code(), Some(2));
    assert!(output_text(&invalid_limit.stderr).contains("invalid value '0' for '--limit <LIMIT>'"));
}

#[test]
fn removed_record_command_is_rejected() {
    let record = practice(&["_record", "python", "two-sum", "pass", "1"]);
    assert_eq!(record.status.code(), Some(2));
    assert!(output_text(&record.stderr).contains("unrecognized subcommand '_record'"));
}
