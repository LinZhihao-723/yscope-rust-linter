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
use crate::rules::RuleSelector;
use crate::source::TargetSpec;

/// Lints the y-scope Rust comment and docstring rules that rustfmt and clippy can't check.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// A file, a directory, or `FILE:RANGES` (e.g. `src/a.rs:10-20,30,40-45`).
    #[arg(value_name = "TARGET", required_unless_present = "diff")]
    targets: Vec<String>,

    /// Enable only this rule: a name, a code (DOC003) or a set (DOC). Repeatable. Default: all
    /// rules.
    #[arg(long = "rule", value_name = "ID")]
    rules: Vec<RuleSelector>,

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
    /// # Returns
    ///
    /// A warning for each deprecated rule that `--rule` names, since it never runs.
    pub fn warnings(&self) -> Vec<String> {
        let mut deprecated_rule_ids: Vec<RuleId> = Vec::new();
        for rule_id in self.rules.iter().flat_map(|selector| selector.rule_ids()) {
            if rule_id.is_deprecated() && !deprecated_rule_ids.contains(&rule_id) {
                deprecated_rule_ids.push(rule_id);
            }
        }
        deprecated_rule_ids
            .into_iter()
            .map(|rule_id| {
                format!(
                    "`{rule_id}` ({}) is deprecated and never runs; remove it from `--rule`",
                    rule_id.code()
                )
            })
            .collect()
    }

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
            self.rules
                .into_iter()
                .flat_map(RuleSelector::rule_ids)
                .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("yscope-rust-linter").chain(args.iter().copied()))
            .expect("valid arguments")
    }

    fn enabled_rules(cli: Cli) -> Vec<RuleId> {
        let rule_set = cli.into_config().expect("valid config").rule_set;
        RuleId::ALL
            .into_iter()
            .filter(|rule_id| rule_set.contains(*rule_id))
            .collect()
    }

    #[test]
    fn rule_flag_selects_rules_by_name_code_and_set() {
        assert_eq!(
            enabled_rules(parse(&["--rule", "errors-doc", "--rule", "ORG", "a.rs"])),
            [RuleId::ErrorsDoc, RuleId::SymbolOrder]
        );
        assert_eq!(
            enabled_rules(parse(&[
                "--rule",
                "DOC003",
                "--rule",
                "type-params-doc",
                "a.rs"
            ])),
            [RuleId::TypeParamsDoc]
        );
        assert_eq!(enabled_rules(parse(&["a.rs"])), RuleId::ALL);
    }

    #[test]
    fn rule_flag_rejects_unknown_values_and_meta_codes() {
        for value in ["nope", "doc003", "LNT001", "parse-error", "LNT"] {
            assert!(
                Cli::try_parse_from(["yscope-rust-linter", "--rule", value, "a.rs"]).is_err(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn deprecated_rule_flag_warns_once_and_enables_nothing() {
        RuleId::ErrorsDoc.with_deprecated(|| {
            let cli = parse(&["--rule", "errors-doc", "--rule", "DOC002", "a.rs"]);
            assert_eq!(
                cli.warnings(),
                ["`errors-doc` (DOC002) is deprecated and never runs; remove it from `--rule`"]
            );
            assert_eq!(enabled_rules(cli), []);

            let cli = parse(&["--rule", "DOC", "a.rs"]);
            assert_eq!(cli.warnings(), Vec::<String>::new());
            assert_eq!(
                enabled_rules(cli),
                [
                    RuleId::CommentGreedyWrap,
                    RuleId::TraitParamsDoc,
                    RuleId::TypeParamsDoc,
                ]
            );
        });
    }
}
