use std::process::Command;

#[test]
fn binary_help_documents_startup_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_interview-tutor"))
        .arg("--help")
        .output()
        .expect("interview binary executes");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("--db"));
    assert!(stdout.contains("--set"));
    assert!(stdout.contains("--language"));
}

#[test]
fn launcher_uses_locked_interview_binary() {
    let launcher = std::fs::read_to_string("../interview").expect("launcher is readable");
    assert!(launcher.contains("cargo run --locked --quiet"));
    assert!(launcher.contains("--bin interview-tutor"));
    assert!(launcher.contains("PRACTICE_ROOT"));
}
