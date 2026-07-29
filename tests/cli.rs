//! CLI behaviour, driven against the real binary.
//!
//! Uses `CARGO_BIN_EXE_adfc` and `std::process::Command` rather than a test
//! harness crate: the binary is what ships, and this needs no dependency.

use std::io::Write;
use std::process::{Command, Stdio};

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Run the binary with `args`, feeding `stdin`, and capture everything.
fn run(args: &[&str], stdin: &str) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_adfc"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary spawns");
    child
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all(stdin.as_bytes())
        .expect("stdin writes");
    let out = child.wait_with_output().expect("child exits");
    Run {
        code: out.status.code().expect("exited via code, not signal"),
        stdout: String::from_utf8(out.stdout).expect("stdout is utf-8"),
        stderr: String::from_utf8(out.stderr).expect("stderr is utf-8"),
    }
}

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

// --- validation is on by default -------------------------------------------

#[test]
fn default_validates_and_succeeds_on_good_input() {
    let r = run(&[], "# Title\n");
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    assert_eq!(doc["type"], "doc");
}

#[test]
fn default_validation_needs_no_schema_file_on_disk() {
    // The whole point of embedding: run from a directory with no checkout.
    let mut child = Command::new(env!("CARGO_BIN_EXE_adfc"))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary spawns");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"# Title\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn validation_failure_exits_nonzero_and_reports_violations() {
    let r = run(
        &["--schema", &fixture("reject-all-schema.json")],
        "# Title\n",
    );
    assert_eq!(r.code, 1, "stdout: {}", r.stdout);
    assert!(
        r.stderr.contains("definitely-absent-key"),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn validation_failure_writes_nothing_to_stdout() {
    // A half-written invalid document is worse than none: downstream would
    // ship it. The write must happen strictly after validation succeeds.
    let r = run(
        &["--schema", &fixture("reject-all-schema.json")],
        "# Title\n",
    );
    assert_eq!(r.code, 1);
    assert_eq!(r.stdout, "", "stdout must be empty on validation failure");
}

// --- flags ------------------------------------------------------------------

#[test]
fn no_validate_skips_validation() {
    let r = run(&["--no-validate"], "# Title\n");
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("\"type\":\"doc\""));
}

#[test]
fn schema_flag_overrides_embedded_schema() {
    // The vendored schema would accept this; the override rejects it, which is
    // only observable if the override actually replaced the embedded one.
    let r = run(
        &["--schema", &fixture("reject-all-schema.json")],
        "# Title\n",
    );
    assert_eq!(r.code, 1);
}

#[test]
fn missing_schema_file_exits_nonzero_and_names_path() {
    let r = run(&["--schema", "/nonexistent/schema.json"], "# Title\n");
    assert_eq!(r.code, 1);
    assert!(
        r.stderr.contains("/nonexistent/schema.json"),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn malformed_schema_file_exits_nonzero_and_names_path() {
    let path = fixture("malformed-schema.json");
    let r = run(&["--schema", &path], "# Title\n");
    assert_eq!(r.code, 1);
    assert!(
        r.stderr.contains("malformed-schema.json"),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn no_validate_with_schema_is_a_usage_error() {
    // Supplying a schema and then ignoring it is self-contradictory. Rejected
    // outright rather than given a silent precedence rule.
    let r = run(
        &[
            "--no-validate",
            "--schema",
            &fixture("reject-all-schema.json"),
        ],
        "# Title\n",
    );
    assert_eq!(r.code, 2, "stderr: {}", r.stderr);
    // Must be rejected as a conflict, not as an unrecognised flag: before
    // --no-validate existed this exited 2 for the wrong reason.
    assert!(
        !r.stderr.contains("unknown argument"),
        "rejected as unknown rather than conflicting: {}",
        r.stderr
    );
    assert!(
        r.stderr.contains("--no-validate") && r.stderr.contains("--schema"),
        "conflict message should name both flags: {}",
        r.stderr
    );
}

#[test]
fn unknown_flag_is_a_usage_error() {
    let r = run(&["--definitely-not-a-flag"], "");
    assert_eq!(r.code, 2);
}

#[test]
fn help_and_version_succeed() {
    let h = run(&["--help"], "");
    assert_eq!(h.code, 0);
    assert!(h.stdout.contains("adfc"));

    let v = run(&["--version"], "");
    assert_eq!(v.code, 0);
    assert!(v.stdout.contains(env!("CARGO_PKG_VERSION")));
}

// --- end-to-end proof of the slice -----------------------------------------

#[test]
fn e2e_stdin_to_stdout_validated() {
    let md = std::fs::read_to_string(fixture("valid.md")).expect("fixture readable");
    let r = run(&[], &md);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);

    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    // Validated twice over: once by the binary, once here against the library.
    assert!(adfc::validate(&doc).is_ok());
    assert_eq!(doc["content"][0]["type"], "heading");
}

#[test]
fn e2e_empty_input_produces_valid_empty_doc() {
    let r = run(&[], "");
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["type"], "doc");
    assert_eq!(doc["content"], serde_json::json!([]));
}
