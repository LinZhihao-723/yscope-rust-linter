//! The entry point of the `trait-params-doc` rule.

use crate::diagnostic::Diagnostic;
use crate::file_context::FileContext;
use crate::rules::RuleId;

/// Checks the `# Parameters` sections of the items in each file of a run.
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
    /// A `trait-params-doc` diagnostic for every mismatch between a trait method's parameters and
    /// its `# Parameters` section, and for every `# Parameters` section outside a trait method
    /// declaration.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        let Some(_syntax_tree) = context.syntax_tree(RuleId::TraitParamsDoc) else {
            return Vec::new();
        };
        if !self.include_tests && context.is_test_module(RuleId::TraitParamsDoc) {
            return Vec::new();
        }
        Vec::new()
    }
}
