//! adfc - convert Markdown to Atlassian Document Format JSON.
//!
//! Output is validated against the embedded ADF JSON Schema before being
//! written; violations go to stderr and exit non-zero, and nothing is written.

use anyhow::{Context, Result};
use clap::Parser;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "adfc", version, about, long_about = None)]
struct Cli {
    /// Markdown input; reads stdin when omitted
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Write ADF JSON here; writes stdout when omitted
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Skip ADF schema validation
    //
    // Conflicts with --schema: supplying a schema and then declining to use it
    // is contradictory, and a silent precedence rule would let a scripted
    // invocation skip the check it asked for.
    #[arg(long, conflicts_with = "schema")]
    no_validate: bool,

    /// Validate against this schema instead of the embedded one
    #[arg(long, value_name = "FILE")]
    schema: Option<PathBuf>,
}

fn main() -> ExitCode {
    // clap exits 2 itself on usage errors, keeping those distinct from the
    // runtime failures below, which are 1.
    let cli = Cli::parse();

    match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("adfc: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode> {
    let markdown = read_input(cli.file.as_deref())?;
    let doc = adfc::markdown_to_adf(&markdown);

    // Validate before producing any output at all: a document that fails the
    // schema must not reach stdout or land in the output file, or a downstream
    // consumer would ship it regardless of the exit code.
    if !cli.no_validate {
        match &cli.schema {
            Some(path) => adfc::validate_against(&load_schema(path)?, &doc)
                .with_context(|| format!("validating against {}", path.display()))?,
            // A refusal and a violation need different messages: the first says
            // the document was never checked, the second that it was and failed.
            // Reporting a refusal as a schema failure would send the reader
            // hunting for a defect that may not exist.
            None => adfc::validate(&doc).map_err(|e| match e {
                adfc::ValidationError::TooDeep { .. } => {
                    anyhow::anyhow!("{e}; pass --no-validate to convert it unchecked")
                }
                adfc::ValidationError::Violations(_) => {
                    anyhow::anyhow!("output failed ADF schema validation:\n{e}")
                }
            })?,
        }
    }

    write_output(cli.output.as_deref(), &doc)
}

fn read_input(file: Option<&Path>) -> Result<String> {
    if let Some(path) = file {
        return std::fs::read_to_string(path)
            .with_context(|| format!("cannot read input {}", path.display()));
    }

    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("cannot read stdin")?;
    Ok(buf)
}

/// Write the document, or report why it could not be written.
///
/// The document is rendered to a string first and written in one call, so a
/// failure cannot leave a truncated file behind.
fn write_output(output: Option<&Path>, doc: &serde_json::Value) -> Result<ExitCode> {
    let rendered = format!("{doc}\n");

    match output {
        Some(path) => {
            std::fs::write(path, &rendered)
                .with_context(|| format!("cannot write output {}", path.display()))?;
            Ok(ExitCode::SUCCESS)
        }
        // Write directly rather than with println! so a downstream consumer
        // closing the pipe early — `adfc t.md | head` — is a quiet success
        // rather than a panic.
        None => match std::io::stdout().write_all(rendered.as_bytes()) {
            Ok(()) => Ok(ExitCode::SUCCESS),
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(ExitCode::SUCCESS),
            Err(e) => Err(e).context("cannot write output"),
        },
    }
}

/// Read the schema named by `--schema`.
///
/// Errors name the path: the flag exists to point at a file the user chose, so
/// a bare parse error would not say which one failed.
fn load_schema(path: &Path) -> Result<serde_json::Value> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read schema {}", path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("cannot parse schema {}", path.display()))
}
