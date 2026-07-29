//! jira-md2adf - Markdown on stdin, ADF JSON on stdout.
//!
//! With --schema <file>, the output is validated against that ADF JSON
//! Schema before printing; violations go to stderr and exit non-zero.

use std::io::{Read, Write};
use std::process::ExitCode;

const USAGE: &str = "usage: jira-md2adf [--schema <adf-schema.json>] < input.md > output.json";

fn main() -> ExitCode {
    let mut schema_path: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--schema" => {
                if let Some(path) = args.next() {
                    schema_path = Some(path);
                } else {
                    eprintln!("jira-md2adf: --schema requires a path\n{USAGE}");
                    return ExitCode::from(2);
                }
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("jira-md2adf {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("jira-md2adf: unknown argument: {other}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    let mut markdown = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut markdown) {
        eprintln!("jira-md2adf: failed to read stdin: {e}");
        return ExitCode::FAILURE;
    }

    let doc = jira_md2adf::markdown_to_adf(&markdown);

    if let Some(path) = schema_path {
        let schema: serde_json::Value = match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        {
            Ok(v) => v,
            Err(e) => {
                eprintln!("jira-md2adf: cannot load schema {path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let validator = match jsonschema::validator_for(&schema) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("jira-md2adf: invalid schema {path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let errors: Vec<String> = validator
            .iter_errors(&doc)
            .map(|e| format!("  {} at {}", e, e.instance_path))
            .collect();
        if !errors.is_empty() {
            eprintln!(
                "jira-md2adf: output failed ADF schema validation:\n{}",
                errors.join("\n")
            );
            return ExitCode::FAILURE;
        }
    }

    // Write directly (not println!) so a downstream consumer closing the
    // pipe early — `jira-md2adf | head` — is a quiet success, not a panic.
    match writeln!(std::io::stdout(), "{doc}") {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("jira-md2adf: failed to write output: {e}");
            ExitCode::FAILURE
        }
    }
}
