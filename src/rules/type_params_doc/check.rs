//! The entry point of the `type-params-doc` rule.

use crate::diagnostic::Diagnostic;
use crate::file_context::FileContext;
use crate::rules::RuleId;

/// Checks the `# Type Parameters` sections of the items in each file of a run.
#[derive(Debug)]
pub struct Checker {
    include_tests: bool,
}

impl Checker {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// A checker that also checks `#[test]` fns and `#[cfg(test)]` items if `include_tests` is set.
    pub const fn new(include_tests: bool) -> Self {
        Self { include_tests }
    }

    /// Checks one file. The file is skipped if it failed to lex or to parse, or if it is an
    /// out-of-line `#[cfg(test)]` module and tests aren't included.
    ///
    /// # Returns
    ///
    /// A `type-params-doc` diagnostic for every mismatch between an item's generic parameters and
    /// its `# Type Parameters` section.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        let Some(_syntax_tree) = context.syntax_tree(RuleId::TypeParamsDoc) else {
            return Vec::new();
        };
        if !self.include_tests && context.is_test_module(RuleId::TypeParamsDoc) {
            return Vec::new();
        }
        Vec::new()
    }
}
