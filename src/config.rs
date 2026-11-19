//! The validated settings of one lint run.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use crate::rules::RuleId;
use crate::source::TargetSpec;

/// Everything a lint run needs, validated from the command line.
#[derive(Debug, Clone)]
pub struct LintConfig {
    pub rule_set: RuleSet,
    pub width: Width,
    pub include_tests: bool,
    pub format: OutputFormat,
    pub targets: Vec<TargetSpec>,
    pub diff_input: Option<DiffInput>,
    pub excludes: Vec<PathBuf>,
}

/// The rules enabled in a run. A deprecated rule is never enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSet {
    rule_ids: HashSet<RuleId>,
}

impl RuleSet {
    /// # Returns
    ///
    /// The set of every rule that isn't deprecated.
    pub fn all() -> Self {
        RuleId::ALL.into_iter().collect()
    }

    /// # Returns
    ///
    /// Whether `rule_id` is enabled.
    pub fn contains(&self, rule_id: RuleId) -> bool {
        self.rule_ids.contains(&rule_id)
    }
}

impl FromIterator<RuleId> for RuleSet {
    fn from_iter<IteratorType: IntoIterator<Item = RuleId>>(rule_ids: IteratorType) -> Self {
        Self {
            rule_ids: rule_ids
                .into_iter()
                .filter(|rule_id| !rule_id.is_deprecated())
                .collect(),
        }
    }
}

/// The column limit that `comment-greedy-wrap` fills comment lines to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Width {
    columns: NonZeroUsize,
}

impl Width {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The width of `columns` columns.
    pub const fn new(columns: NonZeroUsize) -> Self {
        Self { columns }
    }

    pub const fn columns(self) -> usize {
        self.columns.get()
    }
}

/// How diagnostics are printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// `<path>:<line>:<column>: <code> <name>: <message>`.
    Text,

    /// GitHub Actions `::error` workflow commands.
    Github,
}

/// Where the unified diff of `--diff` is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffInput {
    Stdin,
    File { path: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_set_from_rule_ids_contains_only_those_rules() {
        let rule_set: RuleSet = std::iter::once(RuleId::ErrorsDoc).collect();
        assert!(rule_set.contains(RuleId::ErrorsDoc));
        assert!(!rule_set.contains(RuleId::CommentGreedyWrap));
        assert!(
            RuleId::ALL
                .into_iter()
                .all(|rule_id| RuleSet::all().contains(rule_id))
        );
    }
}
