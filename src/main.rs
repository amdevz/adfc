//! adfc - Markdown on stdin, ADF JSON on stdout.
//!
//! Output is validated against the embedded ADF JSON Schema before printing;
//! violations go to stderr and exit non-zero, and nothing is written. Pass
//! --no-validate to skip, or --schema <file> to check against a different
//! schema revision than the embedded one.

use std::io::{Read, Write};
use std::process::ExitCode;

const USAGE: &str = "\
usage: adfc [--no-validate | --schema <adf-schema.json>] < input.md > output.json

Converts Markdown on stdin to Atlassian Document Format JSON on stdout.
The emitted document is validated against the embedded ADF schema by default.

Options:
      --no-validate    skip schema validation
      --schema <FILE>  validate against this schema instead of the embedded one
  -h, --help
  -V, --version";

/// Runtime failure: I/O, an unusable schema, or a document that violates one.
const FAILURE: ExitCode = ExitCode::FAILURE;
/// Usage error: an argument the CLI cannot act on.
const USAGE_ERROR: u8 = 2;

fn main() -> ExitCode {
    let mut schema_path: Option<String> = None;
    let mut no_validate = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--schema" => {
                if let Some(path) = args.next() {
                    schema_path = Some(path);
                } else {
                    eprintln!("adfc: --schema requires a path\n{USAGE}");
                    return ExitCode::from(USAGE_ERROR);
                }
            }
            "--no-validate" => no_validate = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("adfc {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("adfc: unknown argument: {other}\n{USAGE}");
                return ExitCode::from(USAGE_ERROR);
            }
        }
    }

    // Supplying a schema and then declining to use it is self-contradictory.
    // Rejected outright rather than given a silent precedence rule, so a
    // scripted invocation cannot quietly skip the check it asked for.
    if no_validate && schema_path.is_some() {
        eprintln!("adfc: --no-validate conflicts with --schema\n{USAGE}");
        return ExitCode::from(USAGE_ERROR);
    }

    let mut markdown = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut markdown) {
        eprintln!("adfc: failed to read stdin: {e}");
        return FAILURE;
    }

    let doc = adfc::markdown_to_adf(&markdown);

    // Validate before writing a single byte: a document that fails the schema
    // must not reach stdout, or a downstream consumer would ship it anyway.
    if !no_validate {
        let result: Result<(), String> = match schema_path {
            Some(path) => match load_schema(&path) {
                Ok(schema) => adfc::validate_against(&schema, &doc).map_err(|e| format!("{e}")),
                Err(e) => {
                    eprintln!("{e}");
                    return FAILURE;
                }
            },
            None => adfc::validate(&doc).map_err(|e| format!("{e}")),
        };
        if let Err(report) = result {
            eprintln!("adfc: output failed ADF schema validation:\n{report}");
            return FAILURE;
        }
    }

    // Write directly (not println!) so a downstream consumer closing the
    // pipe early — `adfc | head` — is a quiet success, not a panic.
    match writeln!(std::io::stdout(), "{doc}") {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("adfc: failed to write output: {e}");
            FAILURE
        }
    }
}

/// Read the schema named by `--schema`.
///
/// Errors name the path, since the whole point of the flag is pointing at a
/// file the user chose and a bare parse error would not say which.
fn load_schema(path: &str) -> Result<serde_json::Value, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("adfc: cannot read schema {path}: {e}"))?;
    serde_json::from_str(&source).map_err(|e| format!("adfc: cannot parse schema {path}: {e}"))
}
