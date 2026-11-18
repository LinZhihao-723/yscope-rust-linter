//! The registry of the lint rules: their IDs and names, and the enabled rules that a run dispatches
//! each file to.

use std::fmt;
use std::str::FromStr;

use crate::config::LintConfig;
use crate::diagnostic::Diagnostic;
use crate::file_context::FileContext;
use crate::rules::errors_doc;
use crate::rules::greedy_wrap;

/// A lint rule that `--rule` can select and a suppression can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleId {
    CommentGreedyWrap,
    ErrorsDoc,
}

impl RuleId {
    pub const ALL: [Self; 2] = [Self::CommentGreedyWrap, Self::ErrorsDoc];

    /// # Returns
    ///
    /// The rule's ID as `--rule`, suppressions and diagnostics spell it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommentGreedyWrap => "comment-greedy-wrap",
            Self::ErrorsDoc => "errors-doc",
        }
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl clap::ValueEnum for RuleId {
    fn value_variants<'variant>() -> &'variant [Self] {
        &Self::ALL
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.as_str()))
    }
}

impl FromStr for RuleId {
    type Err = UnknownRuleId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|rule_id| rule_id.as_str() == text)
            .ok_or(UnknownRuleId)
    }
}

/// An enabled rule, with its settings and the state it keeps across the files of a run.
#[derive(Debug)]
pub enum Rule {
    CommentGreedyWrap(greedy_wrap::Checker),
    ErrorsDoc(errors_doc::Checker),
}

impl Rule {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The rule `rule_id`, with the settings of `config`.
    pub fn new(rule_id: RuleId, config: &LintConfig) -> Self {
        match rule_id {
            RuleId::CommentGreedyWrap => {
                Self::CommentGreedyWrap(greedy_wrap::Checker::new(config.width))
            }
            RuleId::ErrorsDoc => Self::ErrorsDoc(errors_doc::Checker::new(config.include_tests)),
        }
    }

    /// Runs the rule on one file.
    ///
    /// # Returns
    ///
    /// The rule's diagnostics, before suppressions and line filters apply.
    pub fn check(&mut self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        match self {
            Self::CommentGreedyWrap(checker) => checker.check(context),
            Self::ErrorsDoc(checker) => checker.check(context),
        }
    }
}

/// The error of parsing a string that is not a known rule ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownRuleId;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCode;

    #[test]
    fn rule_ids_round_trip_through_their_names() {
        for rule_id in RuleId::ALL {
            assert_eq!(rule_id.as_str().parse(), Ok(rule_id));
        }
        for code in DiagnosticCode::META {
            assert_eq!(code.as_str().parse::<RuleId>(), Err(UnknownRuleId));
        }
    }
}
