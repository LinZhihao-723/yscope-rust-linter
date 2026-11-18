//! `yscope-rust-linter`: lints the y-scope Rust comment and docstring rules that rustfmt and clippy
//! can't check.
//!
//! Exit codes:
//!
//! * 0: no diagnostics.
//! * 1: at least one diagnostic.
//! * 2: a usage error, an invalid target or diff, or an I/O failure.

mod cli;
mod config;
mod diagnostic;
mod diff;
mod doc;
mod driver;
mod error;
mod file_context;
mod lexer;
mod line_ranges;
mod markdown;
mod rules;
mod source;
mod suppression;
mod syntax;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    let config = match cli.into_config() {
        Ok(config) => config,
        Err(error) => return report_fatal_error(&error),
    };
    let format = config.format;
    let report = match driver::run(config) {
        Ok(report) => report,
        Err(error) => return report_fatal_error(&error),
    };

    let mut stdout = std::io::stdout().lock();
    for file_diagnostic in &report.diagnostics {
        if writeln!(stdout, "{}", file_diagnostic.render(format)).is_err() {
            return ExitCode::from(2);
        }
    }
    eprintln!(
        "yscope-rust-linter: {} in {} checked, {} skipped",
        diagnostic::counted(report.diagnostics.len(), "diagnostic"),
        diagnostic::counted(report.num_files_checked, "file"),
        diagnostic::counted(report.num_generated_files, "generated file")
    );
    if report.diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Prints a fatal error to stderr.
///
/// # Returns
///
/// The exit code of a fatal error.
fn report_fatal_error(error: &error::Error) -> ExitCode {
    eprintln!("yscope-rust-linter: error: {error}");
    ExitCode::from(2)
}
