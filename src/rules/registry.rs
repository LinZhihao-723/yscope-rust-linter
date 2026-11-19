//! The registry of the lint rules: their names, codes and deprecation states, the selectors that
//! name them, and the enabled rules that a run dispatches each file to.

use std::fmt;
use std::str::FromStr;

use crate::config::LintConfig;
use crate::diagnostic::Diagnostic;
use crate::diagnostic::DiagnosticKind;
use crate::file_context::FileContext;
use crate::rules::errors_doc;
use crate::rules::greedy_wrap;
use crate::rules::symbol_order;
use crate::rules::trait_params_doc;
use crate::rules::type_params_doc;

/// A lint rule that `--rule` can select and a suppression can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleId {
    CommentGreedyWrap,
    ErrorsDoc,
    SymbolOrder,
    TraitParamsDoc,
    TypeParamsDoc,
}

impl RuleId {
    pub const ALL: [Self; 5] = [
        Self::CommentGreedyWrap,
        Self::ErrorsDoc,
        Self::SymbolOrder,
        Self::TraitParamsDoc,
        Self::TypeParamsDoc,
    ];

    /// # Returns
    ///
    /// The rule's name as `--rule`, suppressions and diagnostics spell it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::CommentGreedyWrap => "comment-greedy-wrap",
            Self::ErrorsDoc => "errors-doc",
            Self::SymbolOrder => "symbol-order",
            Self::TraitParamsDoc => "trait-params-doc",
            Self::TypeParamsDoc => "type-params-doc",
        }
    }

    /// # Returns
    ///
    /// The rule's stable code. A code is never renumbered, and never reused by another rule.
    pub const fn code(self) -> Code {
        match self {
            Self::CommentGreedyWrap => Code::new(CodeSet::Doc, 1),
            Self::ErrorsDoc => Code::new(CodeSet::Doc, 2),
            Self::TypeParamsDoc => Code::new(CodeSet::Doc, 3),
            Self::TraitParamsDoc => Code::new(CodeSet::Doc, 4),
            Self::SymbolOrder => Code::new(CodeSet::Org, 1),
        }
    }

    /// A deprecated rule stays registered, so its name and code still parse and its code stays
    /// reserved, but it never runs.
    ///
    /// # Returns
    ///
    /// Whether the rule is deprecated.
    pub const fn is_deprecated(self) -> bool {
        match self {
            Self::CommentGreedyWrap
            | Self::ErrorsDoc
            | Self::SymbolOrder
            | Self::TraitParamsDoc
            | Self::TypeParamsDoc => false,
        }
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A set of diagnostic codes, named by the prefix its codes share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeSet {
    /// Comments and docstrings.
    Doc,

    /// Code organization: ordering, layout and placement.
    Org,

    /// The linter's own diagnostics, which no rule owns.
    Lnt,
}

impl CodeSet {
    pub const ALL: [Self; 3] = [Self::Doc, Self::Org, Self::Lnt];

    /// # Returns
    ///
    /// The three-letter uppercase prefix of the set's codes.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Doc => "DOC",
            Self::Org => "ORG",
            Self::Lnt => "LNT",
        }
    }
}

/// A stable diagnostic code: a set prefix and a 3-digit number, such as `DOC003`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code {
    pub set: CodeSet,
    pub number: u16,
}

impl Code {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The code numbered `number` in `set`.
    pub const fn new(set: CodeSet, number: u16) -> Self {
        Self { set, number }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}{:03}", self.set.prefix(), self.number)
    }
}

/// What `--rule` and a suppression name: one rule, by its name or its code, or the rules of a set,
/// by the set's prefix. Names are lowercase and codes and prefixes uppercase, as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSelector {
    Rule(RuleId),
    Set(CodeSet),
}

impl RuleSelector {
    /// # Returns
    ///
    /// The rules the selector names: its rule, or every rule of its set that isn't deprecated, in
    /// [`RuleId::ALL`] order.
    pub fn rule_ids(self) -> impl Iterator<Item = RuleId> {
        RuleId::ALL.into_iter().filter(move |rule_id| match self {
            Self::Rule(selected) => *rule_id == selected,
            Self::Set(set) => rule_id.code().set == set && !rule_id.is_deprecated(),
        })
    }
}

impl FromStr for RuleSelector {
    type Err = InvalidRuleSelector;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if let Some(rule_id) = RuleId::ALL
            .into_iter()
            .find(|rule_id| rule_id.name() == text || rule_id.code().to_string() == text)
        {
            return Ok(Self::Rule(rule_id));
        }
        if text == CodeSet::Lnt.prefix()
            || DiagnosticKind::META
                .iter()
                .any(|kind| kind.name() == text || kind.code().to_string() == text)
        {
            return Err(InvalidRuleSelector::Meta {
                text: text.to_owned(),
            });
        }
        CodeSet::ALL
            .into_iter()
            .find(|set| set.prefix() == text)
            .map(Self::Set)
            .ok_or_else(|| InvalidRuleSelector::Unknown {
                text: text.to_owned(),
            })
    }
}

/// Why a string is not a [`RuleSelector`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidRuleSelector {
    /// The text names a meta diagnostic, by its name or its code, or the set of meta codes, which
    /// no rule owns.
    Meta { text: String },

    /// The text is neither a rule name, a code nor a set prefix.
    Unknown { text: String },
}

impl fmt::Display for InvalidRuleSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Meta { text } => {
                write!(formatter, "`{text}` is not a rule and can't be selected")
            }
            Self::Unknown { text } => {
                let uppercase = text.to_ascii_uppercase();
                if uppercase != *text && uppercase.parse::<RuleSelector>().is_ok() {
                    return write!(
                        formatter,
                        "unknown rule `{text}`; codes and sets are uppercase, so write \
                         `{uppercase}`"
                    );
                }
                let rules: Vec<_> = RuleId::ALL
                    .into_iter()
                    .filter(|rule_id| !rule_id.is_deprecated())
                    .map(|rule_id| format!("{rule_id} ({})", rule_id.code()))
                    .collect();
                let sets: Vec<_> = CodeSet::ALL
                    .into_iter()
                    .filter(|set| *set != CodeSet::Lnt)
                    .map(CodeSet::prefix)
                    .collect();
                write!(
                    formatter,
                    "unknown rule `{text}`; expected a rule name or code, one of {}, or a set, \
                     one of {}",
                    rules.join(", "),
                    sets.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for InvalidRuleSelector {}

/// An enabled rule, with its settings and the state it keeps across the files of a run.
#[derive(Debug)]
pub enum Rule {
    CommentGreedyWrap(greedy_wrap::Checker),
    ErrorsDoc(errors_doc::Checker),
    SymbolOrder(symbol_order::Checker),
    TraitParamsDoc(trait_params_doc::Checker),
    TypeParamsDoc(type_params_doc::Checker),
}

impl Rule {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The rule `rule_id`, with the settings of `config`.
    pub const fn new(rule_id: RuleId, config: &LintConfig) -> Self {
        match rule_id {
            RuleId::CommentGreedyWrap => {
                Self::CommentGreedyWrap(greedy_wrap::Checker::new(config.width))
            }
            RuleId::ErrorsDoc => Self::ErrorsDoc(errors_doc::Checker::new(config.include_tests)),
            RuleId::SymbolOrder => {
                Self::SymbolOrder(symbol_order::Checker::new(config.include_tests))
            }
            RuleId::TraitParamsDoc => {
                Self::TraitParamsDoc(trait_params_doc::Checker::new(config.include_tests))
            }
            RuleId::TypeParamsDoc => {
                Self::TypeParamsDoc(type_params_doc::Checker::new(config.include_tests))
            }
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
            Self::SymbolOrder(checker) => checker.check(context),
            Self::TraitParamsDoc(checker) => checker.check(context),
            Self::TypeParamsDoc(checker) => checker.check(context),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected(text: &str) -> Result<Vec<RuleId>, InvalidRuleSelector> {
        text.parse::<RuleSelector>()
            .map(|selector| selector.rule_ids().collect())
    }

    #[test]
    fn rules_are_selected_by_their_names_and_their_codes() {
        for rule_id in RuleId::ALL {
            assert_eq!(rule_id.name().parse(), Ok(RuleSelector::Rule(rule_id)));
            assert_eq!(
                rule_id.code().to_string().parse(),
                Ok(RuleSelector::Rule(rule_id))
            );
        }
    }

    #[test]
    fn set_prefix_selects_every_rule_of_the_set() {
        assert_eq!(
            selected("DOC"),
            Ok(vec![
                RuleId::CommentGreedyWrap,
                RuleId::ErrorsDoc,
                RuleId::TraitParamsDoc,
                RuleId::TypeParamsDoc,
            ])
        );
        assert_eq!(selected("ORG"), Ok(vec![RuleId::SymbolOrder]));
    }

    #[test]
    fn meta_codes_and_their_set_are_not_selectable() {
        for text in [
            "parse-error",
            "invalid-suppression",
            "LNT001",
            "LNT003",
            "LNT",
        ] {
            assert_eq!(
                selected(text),
                Err(InvalidRuleSelector::Meta {
                    text: text.to_owned()
                }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn selectors_are_case_sensitive_and_lowercase_codes_get_a_hint() {
        for text in [
            "doc002",
            "Doc",
            "org",
            "ERRORS-DOC",
            "errors_doc",
            "DOC005",
            "DOC02",
        ] {
            assert_eq!(
                selected(text),
                Err(InvalidRuleSelector::Unknown {
                    text: text.to_owned()
                }),
                "{text:?}"
            );
        }
        let unknown = |text: &str| {
            InvalidRuleSelector::Unknown {
                text: text.to_owned(),
            }
            .to_string()
        };
        assert_eq!(
            unknown("doc002"),
            "unknown rule `doc002`; codes and sets are uppercase, so write `DOC002`"
        );
        assert_eq!(
            unknown("org"),
            "unknown rule `org`; codes and sets are uppercase, so write `ORG`"
        );
        assert_eq!(
            unknown("lnt001"),
            "unknown rule `lnt001`; expected a rule name or code, one of comment-greedy-wrap \
             (DOC001), errors-doc (DOC002), symbol-order (ORG001), trait-params-doc (DOC004), \
             type-params-doc (DOC003), or a set, one of DOC, ORG"
        );
    }
}
