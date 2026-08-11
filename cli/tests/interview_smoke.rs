use std::path::PathBuf;
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
fn linux_pty_solve_edit_test_submit_and_quit() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI crate has repository parent")
        .to_path_buf();
    let status = Command::new("python3")
        .arg("tests/pty_solve_smoke.py")
        .arg(env!("CARGO_BIN_EXE_interview-tutor"))
        .arg(env!("CARGO_BIN_EXE_practice"))
        .arg(repository_root)
        .status()
        .expect("Python PTY smoke starts");
    assert!(status.success(), "PTY smoke failed with {status}");
}

#[test]
fn launcher_uses_locked_interview_binary() {
    let launcher = std::fs::read_to_string("../interview").expect("launcher is readable");
    assert!(launcher.contains("cargo run --locked --quiet"));
    assert!(launcher.contains("--bin interview-tutor"));
    assert!(launcher.contains("PRACTICE_ROOT"));
}
