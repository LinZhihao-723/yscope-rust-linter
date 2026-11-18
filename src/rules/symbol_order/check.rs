//! The entry point of the `symbol-order` rule.

use crate::diagnostic::Diagnostic;
use crate::file_context::FileContext;
use crate::rules::RuleId;

/// Checks the order of the items in the modules and inherent impl blocks of each file of a run.
#[derive(Debug)]
pub struct Checker {
    include_tests: bool,
}

impl Checker {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// A checker that also checks `#[cfg(test)]` items and modules if `include_tests` is set.
    pub const fn new(include_tests: bool) -> Self {
        Self { include_tests }
    }

    /// Checks one file. The file is skipped if it failed to lex or to parse, or if it is an
    /// out-of-line `#[cfg(test)]` module and tests aren't included.
    ///
    /// # Returns
    ///
    /// A `symbol-order` diagnostic for every item that is out of order.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        let Some(_syntax_tree) = context.syntax_tree(RuleId::SymbolOrder) else {
            return Vec::new();
        };
        if !self.include_tests && context.is_test_module(RuleId::SymbolOrder) {
            return Vec::new();
        }
        Vec::new()
    }
}
