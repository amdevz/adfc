#![cfg(feature = "cli")]

//! CLI behaviour, driven against the real binary.
//!
//! Gated on the feature that builds the binary: without it `CARGO_BIN_EXE_adfc`
//! is undefined, and the tests ship in the crate, so a consumer building without
//! default features would see every case here fail.

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
    // Best-effort: plenty of runs never read stdin (clap exits on a usage
    // error, a FILE argument ignores it), so the child closes the pipe and this
    // gets EPIPE — which means the binary did its job, not that the test
    // failed. Anything larger than the 64KB pipe buffer loses that race.
    let mut stdin_handle = child.stdin.take().expect("stdin piped");
    if let Err(e) = stdin_handle.write_all(stdin.as_bytes()) {
        assert_eq!(
            e.kind(),
            std::io::ErrorKind::BrokenPipe,
            "unexpected error writing to child stdin: {e}"
        );
    }
    // Drop to signal EOF to a child that *is* reading stdin.
    drop(stdin_handle);

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
    // Must be rejected as a conflict rather than as an unrecognised flag;
    // both exit 2, so only the message distinguishes them.
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
    assert!(adfc::validate_document(&doc).is_ok());
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

// --- file input and output --------------------------------------------------

/// A unique scratch path for this test run; no tempfile dependency needed.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("adfc-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir.join(name)
}

#[test]
fn reads_named_input_file() {
    let r = run(&[&fixture("valid.md")], "");
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    assert_eq!(doc["content"][0]["type"], "heading");
}

#[test]
fn named_input_file_takes_precedence_over_stdin() {
    // Stdin carries something distinguishable; the file must win.
    let r = run(&[&fixture("valid.md")], "# FromStdin\n");
    assert_eq!(r.code, 0);
    assert!(
        !r.stdout.contains("FromStdin"),
        "stdin leaked into output: {}",
        r.stdout
    );
}

#[test]
fn writes_named_output_file() {
    let out = scratch("writes-named-output.json");
    let _ = std::fs::remove_file(&out);
    let r = run(&["-o", out.to_str().unwrap()], "# Title\n");
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "", "output went to the file, not stdout");

    let written = std::fs::read_to_string(&out).expect("output file exists");
    let doc: serde_json::Value = serde_json::from_str(&written).expect("file is JSON");
    assert!(adfc::validate_document(&doc).is_ok());
}

#[test]
fn missing_input_file_exits_1_and_names_path() {
    let r = run(&["/nonexistent/input.md"], "");
    assert_eq!(r.code, 1, "stdout: {}", r.stdout);
    assert!(
        r.stderr.contains("/nonexistent/input.md"),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn unwritable_output_exits_1_and_names_path() {
    // A path whose parent does not exist: portable, unlike chmod tricks, and
    // still fails when the suite runs as root in a container.
    let bad = "/nonexistent-dir/out.json";
    let r = run(&["-o", bad], "# Title\n");
    assert_eq!(r.code, 1, "stdout: {}", r.stdout);
    assert!(r.stderr.contains(bad), "stderr: {}", r.stderr);
}

#[test]
fn failed_write_leaves_no_partial_file() {
    let bad = "/nonexistent-dir/out.json";
    let r = run(&["-o", bad], "# Title\n");
    assert_eq!(r.code, 1);
    assert!(
        !std::path::Path::new(bad).exists(),
        "a partial output file was left behind"
    );
}

#[test]
fn validation_failure_writes_no_output_file() {
    // The no-partial-output guarantee must hold for the file path too, not
    // just stdout.
    let out = scratch("validation-failure.json");
    let _ = std::fs::remove_file(&out);
    let r = run(
        &[
            "-o",
            out.to_str().unwrap(),
            "--schema",
            &fixture("reject-all-schema.json"),
        ],
        "# Title\n",
    );
    assert_eq!(r.code, 1);
    assert!(
        !out.exists(),
        "output file created despite validation failure"
    );
}

#[test]
fn non_utf8_input_file_exits_1() {
    let bad = scratch("non-utf8.md");
    std::fs::write(&bad, [0xff, 0xfe, 0x00, 0x80]).expect("write fixture");
    let r = run(&[bad.to_str().unwrap()], "");
    assert_eq!(r.code, 1, "stdout: {}", r.stdout);
    assert!(!r.stderr.is_empty(), "an error should be reported");
}

#[test]
fn broken_pipe_exits_zero() {
    // `adfc big.md | head` must be a quiet success, not a panic. Guards the
    // write path, where a simplification to println! would reintroduce it.
    use std::process::Command;
    let mut child = Command::new(env!("CARGO_BIN_EXE_adfc"))
        .arg(fixture("valid.md"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawns");
    drop(child.stdout.take()); // close the read end immediately
    let out = child.wait_with_output().expect("exits");
    assert_eq!(out.status.code(), Some(0), "broken pipe must exit 0");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "panicked: {stderr}");
}

// --- end-to-end across all four input/output combinations -------------------

#[test]
fn e2e_file_in_file_out() {
    let out = scratch("e2e-file-in-file-out.json");
    let _ = std::fs::remove_file(&out);
    let r = run(&[&fixture("valid.md"), "-o", out.to_str().unwrap()], "");
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("file exists"))
            .expect("file is JSON");
    assert!(adfc::validate_document(&doc).is_ok());
}

#[test]
fn e2e_file_in_stdout_out() {
    let r = run(&[&fixture("valid.md")], "");
    assert_eq!(r.code, 0);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    assert!(adfc::validate_document(&doc).is_ok());
}

#[test]
fn e2e_stdin_in_file_out() {
    let out = scratch("e2e-stdin-in-file-out.json");
    let _ = std::fs::remove_file(&out);
    let r = run(&["-o", out.to_str().unwrap()], "# Title\n");
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(out.exists());
}

#[test]
fn e2e_output_is_a_single_compact_json_line() {
    // Downstream consumers pipe this straight into jq; compact single-line
    // output is part of the contract.
    let r = run(&[&fixture("valid.md")], "");
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim_end().lines().count(), 1);
}

#[test]
fn usage_error_with_oversized_stdin_does_not_break_the_harness() {
    // Guards the harness, not the binary. The child rejects these arguments
    // and exits without reading stdin, so a payload larger than the pipe
    // buffer guarantees the write gets EPIPE.
    let big = "# Title\n\n".repeat(20_000);
    let r = run(
        &[
            "--no-validate",
            "--schema",
            &fixture("reject-all-schema.json"),
        ],
        &big,
    );
    assert_eq!(r.code, 2, "stderr: {}", r.stderr);
}

#[test]
fn file_argument_with_oversized_stdin_does_not_break_the_harness() {
    // Same race, different cause: given a FILE argument the binary never
    // reads stdin at all.
    let big = "# FromStdin\n\n".repeat(20_000);
    let r = run(&[&fixture("valid.md")], &big);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(!r.stdout.contains("FromStdin"), "stdin leaked into output");
}

// --- the validation depth bound ---------------------------------------------

/// Markdown nesting `depth` levels of bullet list. Kept under the 64KB stdin
/// pipe buffer noted on `run`, so the write cannot block.
fn nested_list_markdown(depth: usize) -> String {
    (0..depth)
        .map(|i| format!("{}- item", "  ".repeat(i)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn deeply_nested_input_is_refused_rather_than_exhausting_memory() {
    // Before the bound this aborted the process on an allocation failure after
    // ~2GB. It must now fail fast, explain itself, and name the escape hatch.
    let r = run(&[], &nested_list_markdown(200));
    assert_ne!(r.code, 0, "should refuse, stdout: {}", r.stdout);
    assert!(
        r.stderr.contains("nests") && r.stderr.contains("--no-validate"),
        "stderr should name the depth and the way past it: {}",
        r.stderr
    );
    // Same contract as a schema violation: nothing reaches stdout, so a
    // downstream consumer cannot ship a document this run never checked.
    assert!(
        r.stdout.is_empty(),
        "refused run must emit no document, got: {}",
        r.stdout
    );
}

#[test]
fn no_validate_converts_deeply_nested_input() {
    // The bound limits checking, not converting. Asserted as text rather than
    // parsed back: serde_json's default recursion limit is 128, the same depth
    // MAX_VALIDATION_DEPTH allows, so no default parser can read output this
    // deep.
    let r = run(&["--no-validate"], &nested_list_markdown(200));
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(
        r.stdout.starts_with(r#"{"content":["#) && r.stdout.contains(r#""type":"doc""#),
        "should still emit a document, got: {}",
        &r.stdout[..r.stdout.len().min(80)]
    );
}

// --- adf embeds end to end --------------------------------------------------

/// A fenced `adf` block carrying `body`.
fn adf_fence(body: &str) -> String {
    format!("```adf\n{body}\n```\n")
}

#[test]
fn cli_converts_a_block_embed() {
    let r = run(&[], &adf_fence(r#"{"type":"rule"}"#));
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    assert_eq!(doc["content"][0]["type"], "rule");
    assert!(adfc::validate_document(&doc).is_ok());
}

#[test]
fn cli_refuses_a_malformed_embed() {
    let r = run(&[], &adf_fence(r#"{"type":"status",}"#));
    assert_ne!(r.code, 0, "a malformed embed must fail the run");
    assert!(
        r.stderr.contains("adf embed"),
        "stderr should say an embed was the problem: {}",
        r.stderr
    );
    // Same contract as a schema violation: a document that was not honoured
    // must not reach a downstream consumer regardless of the exit code.
    assert!(
        r.stdout.is_empty(),
        "refused run must emit nothing, got: {}",
        r.stdout
    );
}

#[test]
fn cli_no_validate_converts_a_malformed_embed() {
    // The gate is validation, not conversion: skipping it still produces the
    // document, with the author's text preserved as visible code.
    let r = run(&["--no-validate"], &adf_fence(r#"{"type":"status",}"#));
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    assert_eq!(doc["content"][0]["type"], "codeBlock");
}

// --- located embed errors end to end ----------------------------------------

#[test]
fn cli_reports_the_field_and_line_for_a_bad_attribute() {
    let r = run(
        &[],
        "intro\n\n```adf\n{\"type\":\"status\",\"attrs\":{\"text\":\"Done\",\"colour\":\"green\"}}\n```\n",
    );
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("line 3"), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("'colour' was unexpected"),
        "stderr: {}",
        r.stderr
    );
    assert!(
        r.stderr.contains("\"color\" is a required property"),
        "stderr: {}",
        r.stderr
    );
    assert!(r.stdout.is_empty(), "nothing may reach stdout");
}

#[test]
fn cli_lists_allowed_values_for_a_bad_enum() {
    let r = run(
        &[],
        "```adf\n{\"type\":\"status\",\"attrs\":{\"text\":\"Done\",\"color\":\"orange\"}}\n```\n",
    );
    assert_ne!(r.code, 0);
    assert!(
        r.stderr.contains("\"orange\" is not one of"),
        "stderr: {}",
        r.stderr
    );
    assert!(r.stderr.contains("neutral"), "stderr: {}", r.stderr);
}

#[test]
fn cli_names_an_unknown_node_type() {
    let r = run(&[], "```adf\n{\"type\":\"statuz\"}\n```\n");
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("statuz"), "stderr: {}", r.stderr);
    assert!(
        !r.stderr.contains("is not valid under any of the schemas"),
        "must not fall back to the union message: {}",
        r.stderr
    );
}

// --- inline embeds end to end -----------------------------------------------

const STATUS_JSON: &str = r#"{"type":"status","attrs":{"text":"Done","color":"green"}}"#;

#[test]
fn cli_renders_a_status_badge_from_a_fence() {
    let r = run(&[], &format!("```adf\n{STATUS_JSON}\n```\n"));
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    assert_eq!(doc["content"][0]["type"], "paragraph");
    assert_eq!(doc["content"][0]["content"][0]["type"], "status");
}

#[test]
fn cli_renders_a_status_badge_inside_a_sentence() {
    let r = run(
        &[],
        &format!("The build is `adf:{STATUS_JSON}` and shipping.\n"),
    );
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    let blocks = doc["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 1, "the badge must not break the paragraph");
    let types: Vec<&str> = blocks[0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["text", "status", "text"]);
}

#[test]
fn cli_refuses_a_block_node_in_an_inline_span() {
    let r = run(&[], "text `adf:{\"type\":\"rule\"}` more\n");
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("rule"), "stderr: {}", r.stderr);
    assert!(r.stdout.is_empty(), "nothing may reach stdout");
}

// --- containment end to end -------------------------------------------------

const TABLE_JSON: &str = r#"{"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"x"}]}]}]}]}"#;

#[test]
fn cli_refuses_an_embedded_table_in_a_panel() {
    let md = format!("> [!NOTE]\n> see below\n>\n> ```adf\n> {TABLE_JSON}\n> ```\n");
    let r = run(&[], &md);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("table"), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("panel"), "stderr: {}", r.stderr);
    assert!(r.stdout.is_empty(), "nothing may reach stdout");
}

#[test]
fn cli_still_hoists_a_markdown_table_from_a_panel() {
    // The asymmetry, proven end to end: Markdown-derived content still moves,
    // because Markdown cannot express ADF's nesting rules.
    let r = run(
        &[],
        "> [!NOTE]\n> see below\n>\n> | a |\n> | - |\n> | 1 |\n",
    );
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["panel", "table"]);
}

#[test]
fn cli_converts_an_array_embed() {
    let r = run(
        &[],
        "```adf\n[{\"type\":\"rule\"},{\"type\":\"rule\"}]\n```\n",
    );
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let doc: serde_json::Value = serde_json::from_str(&r.stdout).expect("stdout is JSON");
    assert_eq!(doc["content"].as_array().unwrap().len(), 2);
}
