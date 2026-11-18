//! The command-line interface and its conversion into a [`LintConfig`].

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;

use crate::config::DiffInput;
use crate::config::LintConfig;
use crate::config::OutputFormat;
use crate::config::RuleSet;
use crate::config::Width;
use crate::error::Error;
use crate::rules::RuleId;
use crate::source::TargetSpec;

/// Lints the y-scope Rust comment and docstring rules that rustfmt and clippy can't check.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// A file, a directory, or `FILE:RANGES` (e.g. `src/a.rs:10-20,30,40-45`).
    #[arg(value_name = "TARGET", required_unless_present = "diff")]
    targets: Vec<String>,

    /// Enable only this rule. Repeatable. Default: all rules.
    #[arg(long = "rule", value_name = "ID")]
    rules: Vec<RuleId>,

    /// Fill width for comment-greedy-wrap.
    #[arg(long, value_name = "N", default_value = "100")]
    width: NonZeroUsize,

    /// Also check #[test] fns and #[cfg(test)] items, which the syntax-tree rules skip by default.
    #[arg(long)]
    include_tests: bool,

    /// Skip this file or directory (path-prefix match by whole components). Repeatable.
    #[arg(long = "exclude", value_name = "PATH")]
    excludes: Vec<PathBuf>,

    /// Lint the changed lines of a unified diff (`git diff -U0`); `-` reads stdin.
    #[arg(long, value_name = "FILE")]
    diff: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

impl Cli {
    /// Validates the parsed arguments.
    ///
    /// # Returns
    ///
    /// The run's configuration on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * Forwards [`TargetSpec::from_str`]'s return values on failure.
    pub fn into_config(self) -> Result<LintConfig, Error> {
        let targets = self
            .targets
            .iter()
            .map(|argument| TargetSpec::from_str(argument))
            .collect::<Result<Vec<_>, _>>()?;
        let rule_set = if self.rules.is_empty() {
            RuleSet::all()
        } else {
            self.rules.into_iter().collect()
        };
        let diff_input = self.diff.map(|path| {
            if path.as_os_str() == "-" {
                DiffInput::Stdin
            } else {
                DiffInput::File { path }
            }
        });
        Ok(LintConfig {
            rule_set,
            width: Width::new(self.width),
            include_tests: self.include_tests,
            format: self.format,
            targets,
            diff_input,
            excludes: self.excludes,
        })
    }
}
