// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! The binary, end to end: the M0 minimal-evidence chain driven the
//! way a user drives it — `check`, `run` against a real child
//! process, `simulate` against the scripted lab.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/echo.rkg")
}

fn rastok() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rastok"))
}

#[test]
fn check_accepts_the_example_and_prints_its_hash() {
    let output = rastok()
        .arg("check")
        .arg(example())
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("semantic hash"), "{stdout}");
}

#[test]
fn check_renders_a_semantic_refusal_in_the_rustc_shape() {
    let path = std::env::temp_dir().join("rastok-cli-evidence-bad.rkg");
    // A real semantic rule, not a parser accident: an emission
    // leaving by an input port.
    std::fs::write(
        &path,
        "graph g : App {\n  operator o {\n    on in { in = . }\n  }\n  . -> o -> .\n}\n",
    )
    .expect("a writable temp dir");
    let output = rastok()
        .arg("check")
        .arg(&path)
        .output()
        .expect("the binary runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E-PORT-DIRECTION]"), "{stderr}");
    assert!(stderr.contains(":3:"), "line and column: {stderr}");
    assert!(stderr.contains("^^"), "the underline: {stderr}");
}

#[test]
fn run_echoes_a_line_through_a_real_child_process() {
    let mut child = rastok()
        .arg("run")
        .arg(example())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary runs");
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(b"salut\n")
        .expect("stdin accepts");
    let output = child.wait_with_output().expect("the run completes");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "out: salut\n");
}

#[test]
fn simulate_runs_the_lab_and_reports_every_scenario() {
    let output = rastok()
        .arg("simulate")
        .arg(example())
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS smoke.roundtrip"), "{stdout}");
    assert!(stdout.contains("PASS smoke.silence_expires"), "{stdout}");
    assert!(stdout.contains("3 scenario(s), 0 failed"), "{stdout}");
}

#[test]
fn simulate_fails_loudly_on_a_broken_expectation() {
    let source = std::fs::read_to_string(example()).expect("the example reads");
    let path = std::env::temp_dir().join("rastok-cli-evidence-broken.rkg");
    std::fs::write(
        &path,
        source.replace("data == \"salut\"", "data == \"autre\""),
    )
    .expect("a writable temp dir");
    let output = rastok()
        .arg("simulate")
        .arg(&path)
        .output()
        .expect("the binary runs");
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FAIL smoke.roundtrip"), "{stdout}");
    assert!(stdout.contains("the echo"), "the label names it: {stdout}");
}

#[test]
fn simulate_blesses_then_verifies_a_golden_directory() {
    let dir = std::env::temp_dir().join("rastok-cli-golden");
    let _ = std::fs::remove_dir_all(&dir);
    // Bless…
    let output = rastok()
        .args(["simulate"])
        .arg(example())
        .args(["--golden"])
        .arg(&dir)
        .args(["--bless"])
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{output:?}");
    assert!(dir.join("smoke.roundtrip.trace").exists());
    // …verify…
    let output = rastok()
        .args(["simulate"])
        .arg(example())
        .args(["--golden"])
        .arg(&dir)
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{output:?}");
    // …and a staled golden is named, not hex-dumped.
    let path = dir.join("smoke.roundtrip.trace");
    let stored = std::fs::read_to_string(&path).expect("the blessed trace");
    std::fs::write(&path, stored.replace("73616c7574", "6175747265")).expect("the alteration");
    let output = rastok()
        .args(["simulate"])
        .arg(example())
        .args(["--golden"])
        .arg(&dir)
        .output()
        .expect("the binary runs");
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FAIL smoke.roundtrip"), "{stdout}");
    assert!(stdout.contains("golden divergence"), "{stdout}");
    assert!(
        stdout.contains("data(8)"),
        "the diff names the key: {stdout}"
    );
}

#[test]
fn bless_without_golden_is_a_usage_error() {
    let output = rastok()
        .args(["simulate"])
        .arg(example())
        .args(["--bless"])
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--bless requires --golden"), "{stderr}");
}
