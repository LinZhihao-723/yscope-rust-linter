//! The entry point of the `symbol-order` rule.

use crate::diagnostic::Diagnostic;
use crate::file_context::FileContext;
use crate::line_ranges::LineSpan;
use crate::rules::RuleId;
use crate::rules::symbol_order::keys::SortKey;
use crate::rules::symbol_order::moves;
use crate::rules::symbol_order::scope::Entry;
use crate::rules::symbol_order::scope::EntryCollector;
use crate::syntax::crate_names::CrateNames;

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
    /// The file root and every inline module are checked, and so is every inherent impl block in
    /// them. In each scope, the items outside a longest in-order subsequence are out of order.
    ///
    /// # Returns
    ///
    /// A `symbol-order` diagnostic for every item that is out of order. Each one's anchor span runs
    /// from the item to the item it should come next to, and its fn header is the item's header.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        let Some(syntax_tree) = context.syntax_tree(RuleId::SymbolOrder) else {
            return Vec::new();
        };
        if !self.include_tests && context.is_test_module(RuleId::SymbolOrder) {
            return Vec::new();
        }
        let crate_names = CrateNames::collect(syntax_tree);
        let mut file_check = FileCheck {
            entry_collector: EntryCollector {
                crate_names: &crate_names,
                include_tests: self.include_tests,
            },
            diagnostics: Vec::new(),
        };
        file_check.record_module_diagnostics(&syntax_tree.items);
        file_check.diagnostics
    }
}

/// The check of one file's scopes.
///
/// # Type Parameters
///
/// * `'file` - The lifetime of the file's crate-local names.
struct FileCheck<'file> {
    entry_collector: EntryCollector<'file>,
    diagnostics: Vec<Diagnostic>,
}

impl FileCheck<'_> {
    /// Checks the module whose items are `items`, its inherent impl blocks and its inline modules,
    /// recursively, and records the findings.
    fn record_module_diagnostics(&mut self, items: &[syn::Item]) {
        self.diagnostics.extend(scope_diagnostics(
            &self.entry_collector.module_entries(items),
        ));
        for item in items {
            match item {
                syn::Item::Mod(item_mod)
                    if !self.entry_collector.is_skipped(&item_mod.attrs, false) =>
                {
                    if let Some((_, content)) = &item_mod.content {
                        self.record_module_diagnostics(content);
                    }
                }
                syn::Item::Impl(item_impl)
                    if item_impl.trait_.is_none()
                        && !self.entry_collector.is_skipped(&item_impl.attrs, false) =>
                {
                    self.diagnostics.extend(scope_diagnostics(
                        &self.entry_collector.impl_entries(item_impl),
                    ));
                }
                _ => {}
            }
        }
    }
}

/// Finds the misplaced items of one scope whose counted items are `entries`.
///
/// # Type Parameters
///
/// * `KeyType` - The type of the scope's sort keys.
///
/// # Returns
///
/// A diagnostic for each item outside the kept subsequence, at the item's first token after its
/// attributes.
///
/// # Panics
///
/// Panics if an item's span ends before an earlier item's span starts.
fn scope_diagnostics<KeyType: SortKey>(entries: &[Entry<KeyType>]) -> Vec<Diagnostic> {
    let keys: Vec<KeyType> = entries.iter().map(|entry| entry.key).collect();
    let is_kept = moves::kept(&keys);
    entries
        .iter()
        .enumerate()
        .filter(|&(index, _)| !is_kept[index])
        .map(|(index, flagged)| {
            let placement = moves::place(&keys, &is_kept, index);
            let reference = &entries[placement.reference];
            let conflict = &entries[placement.conflict];
            let message = format!(
                "{} should come {} {}: {}",
                flagged.description,
                placement.direction.as_str(),
                reference.description,
                KeyType::reason(flagged.key.min(conflict.key), flagged.key.max(conflict.key)),
            );
            let (earlier, later) = if index < placement.reference {
                (flagged, reference)
            } else {
                (reference, flagged)
            };
            let anchor = LineSpan::new(earlier.lines.span.first(), later.lines.span.last())
                .expect("a later item ends after an earlier item starts");
            Diagnostic::new(RuleId::SymbolOrder.into(), flagged.position, message)
                .with_anchor(anchor)
                .with_fn_header(flagged.lines.header)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::config::RuleSet;
    use crate::syntax::test_code::TestModuleCache;

    const FIXTURE_PATH: &str = "nonexistent-fixture-directory/subject.rs";

    /// Checks `source_text` as a file that no other file declares.
    ///
    /// # Returns
    ///
    /// The diagnostics, sorted by position.
    ///
    /// # Panics
    ///
    /// Panics if `source_text` doesn't parse.
    fn diagnostics(source_text: &str, include_tests: bool) -> Vec<Diagnostic> {
        syn::parse_file(source_text).expect("valid file");
        let test_module_cache = TestModuleCache::default();
        let context = FileContext::new(Path::new(FIXTURE_PATH), source_text, &test_module_cache);
        let mut diagnostics = Checker::new(include_tests).check(&context);
        diagnostics.sort_by_key(Diagnostic::position);
        diagnostics
    }

    /// # Returns
    ///
    /// Each diagnostic of `source_text`, without tests, as `<line>:<column>: <message>`.
    fn findings(source_text: &str) -> Vec<String> {
        format_findings(&diagnostics(source_text, false))
    }

    /// # Returns
    ///
    /// Each of `diagnostics` as `<line>:<column>: <message>`.
    fn format_findings(diagnostics: &[Diagnostic]) -> Vec<String> {
        diagnostics
            .iter()
            .map(|diagnostic| {
                let position = diagnostic.position();
                format!(
                    "{}:{}: {}",
                    position.line,
                    position.column,
                    diagnostic.message()
                )
            })
            .collect()
    }

    #[test]
    fn ordered_modules_are_clean() {
        let source_text = concat!(
            "pub const A: u8 = 0;\n",
            "pub static B: u8 = 0;\n",
            "pub trait Trait {}\n",
            "pub struct S;\n",
            "impl S {}\n",
            "impl Trait for S {}\n",
            "impl Clone for S { fn clone(&self) -> Self { Self } }\n",
            "pub type Alias = u8;\n",
            "pub fn f() {}\n",
            "pub(crate) fn g() {}\n",
            "pub(super) const C: u8 = 0;\n",
            "pub(in crate::a) fn h() {}\n",
            "const D: u8 = 0;\n",
            "enum E {}\n",
            "impl E {}\n",
            "fn i() {}\n",
        );
        assert_eq!(findings(source_text), Vec::<String>::new());
    }

    #[test]
    fn visibility_groups_are_ordered_pub_first_and_private_last() {
        assert_eq!(
            findings("fn a() {}\npub fn b() {}\npub fn c() {}\n"),
            [
                "1:1: private fn `a` should come after pub fn `c`: pub items come before private \
                 items"
            ]
        );
        assert_eq!(
            findings(concat!(
                "pub(crate) fn a() {}\n",
                "pub fn b() {}\n",
                "pub(in crate::x) fn c() {}\n",
                "pub(super) fn d() {}\n",
                "pub(self) fn e() {}\n",
                "pub(in crate) fn f() {}\n",
            )),
            [
                "2:1: pub fn `b` should come before pub(crate) fn `a`: pub items come before \
                 pub(crate) items",
                "4:1: pub(super) fn `d` should come after pub(crate) fn `a`: pub(super) items \
                 come before pub(in path) items",
                "6:1: pub(crate) fn `f` should come after pub(crate) fn `a`: pub(crate) items \
                 come before pub(in path) items",
            ]
        );
    }

    #[test]
    fn categories_are_ordered_consts_statics_types_fns_within_a_group() {
        assert_eq!(
            findings(concat!(
                "pub fn a() {}\n",
                "pub struct S;\n",
                "pub static B: u8 = 0;\n",
                "pub const C: u8 = 0;\n",
                "pub fn d() {}\n",
            )),
            [
                "2:1: pub struct `S` should come before pub fn `a`: types and their impls come \
                 before fns",
                "3:1: pub static `B` should come before pub fn `a`: statics come before fns",
                "4:1: pub const `C` should come before pub fn `a`: consts come before fns",
            ]
        );
        assert_eq!(
            findings("pub static A: u8 = 0;\npub const B: u8 = 0;\npub static C: u8 = 0;\n"),
            ["2:1: pub const `B` should come before pub static `A`: consts come before statics"]
        );
    }

    #[test]
    fn types_of_any_kind_keep_their_relative_order() {
        let source_text = concat!(
            "pub type A = u8;\n",
            "pub trait B {}\n",
            "pub union C { a: u8 }\n",
            "pub enum D {}\n",
            "pub struct E;\n",
            "pub trait F = B;\n",
        );
        assert_eq!(findings(source_text), Vec::<String>::new());
    }

    #[test]
    fn impl_blocks_directly_follow_their_type() {
        assert_eq!(
            findings("pub struct A;\npub struct B;\nimpl A {}\nimpl B {}\n"),
            [
                "3:1: impl `A` should come after pub struct `A`: a type's impl blocks directly \
                 follow it"
            ]
        );
        assert_eq!(
            findings("impl A {}\npub struct A;\n"),
            [
                "2:1: pub struct `A` should come before impl `A`: a type's impl blocks directly \
                 follow it"
            ]
        );
        assert_eq!(
            findings("pub struct A;\npub fn f() {}\nimpl Clone for A {}\n"),
            [
                "3:1: impl `Clone for A` should come after pub struct `A`: types and their impls \
                 come before fns"
            ]
        );
        assert_eq!(
            findings("pub struct A<T>(T);\npub struct B;\nimpl<T> !Send for A<T> {}\n"),
            [
                "3:1: impl `!Send for A` should come after pub struct `A`: a type's impl blocks \
                 directly follow it"
            ]
        );
    }

    #[test]
    fn impl_blocks_are_ordered_inherent_then_crate_traits_then_external_traits() {
        let source_text = concat!(
            "use crate::store::Store;\n",
            "use std::fmt;\n",
            "pub trait Local {}\n",
            "pub struct S;\n",
            "impl fmt::Display for S {}\n",
            "impl Local for S {}\n",
            "impl S {}\n",
            "impl Store for S {}\n",
            "impl crate::Other for S {}\n",
        );
        assert_eq!(
            findings(source_text),
            [
                "5:1: impl `fmt::Display for S` should come after impl `crate::Other for S`: \
                 impls of crate traits come before impls of external traits",
                "7:1: impl `S` should come after pub struct `S`: inherent impls come before impls \
                 of crate traits",
            ]
        );
    }

    #[test]
    fn impl_blocks_take_the_visibility_group_of_their_type() {
        assert_eq!(
            findings("struct Private;\nimpl Private {}\npub fn f() {}\n"),
            [
                "3:1: pub fn `f` should come before private struct `Private`: pub items come \
                 before private items"
            ]
        );
        assert_eq!(
            findings("pub fn f() {}\nstruct Private;\nimpl Private {}\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn impl_blocks_of_types_from_elsewhere_are_exempt() {
        let source_text = concat!(
            "pub struct S<T>(T);\n",
            "pub fn f() {}\n",
            "impl Foreign {}\n",
            "impl crate::a::S<u8> {}\n",
            "impl<T> Trait for T {}\n",
            "impl Trait for &S<u8> {}\n",
            "impl dyn Trait {}\n",
            "impl<T> (S<T>) {}\n",
        );
        assert_eq!(
            findings(source_text),
            [
                "8:1: impl `S` should come after pub struct `S`: types and their impls come \
                 before fns"
            ]
        );
    }

    #[test]
    fn impl_blocks_belong_to_the_nearest_earlier_definition_of_their_type() {
        let source_text = concat!(
            "#[cfg(unix)]\n",
            "pub struct S;\n",
            "#[cfg(unix)]\n",
            "impl S {}\n",
            "#[cfg(not(unix))]\n",
            "pub struct S;\n",
            "#[cfg(not(unix))]\n",
            "impl S {}\n",
        );
        assert_eq!(findings(source_text), Vec::<String>::new());
    }

    #[test]
    fn inherent_impls_order_consts_first_then_fns_by_visibility() {
        let source_text = concat!(
            "pub struct S;\n",
            "impl S {\n",
            "    const A: u8 = 0;\n",
            "    pub fn b() {}\n",
            "    fn c() {}\n",
            "    pub const D: u8 = 0;\n",
            "    pub(crate) fn e() {}\n",
            "    type F = u8;\n",
            "    m!();\n",
            "}\n",
        );
        assert_eq!(
            findings(source_text),
            [
                "6:5: pub const `D` should come after private const `A`: associated consts come \
                 before fns",
                "7:5: pub(crate) fn `e` should come after pub fn `b`: pub(crate) fns come before \
                 private fns",
            ]
        );
    }

    #[test]
    fn associated_consts_are_not_ordered_by_visibility() {
        let source_text = concat!(
            "pub struct S;\n",
            "impl S {\n",
            "    const A: u8 = 0;\n",
            "    pub const B: u8 = 0;\n",
            "    pub fn c() {}\n",
            "}\n",
        );
        assert_eq!(findings(source_text), Vec::<String>::new());
    }

    #[test]
    fn trait_impls_and_trait_declarations_are_not_checked_inside() {
        let source_text = concat!(
            "pub trait Trait { fn a(); const B: u8; }\n",
            "pub struct S;\n",
            "impl Trait for S { fn a() {} const B: u8 = 0; }\n",
        );
        assert_eq!(findings(source_text), Vec::<String>::new());
    }

    #[test]
    fn exempt_items_are_neither_checked_nor_counted() {
        let source_text = concat!(
            "pub fn a() {}\n",
            "use std::fmt;\n",
            "extern crate alloc;\n",
            "mod declared;\n",
            "mod inline {}\n",
            "macro_rules! m { () => {} }\n",
            "m!();\n",
            "extern \"C\" { fn external(); }\n",
            "pub fn b() {}\n",
        );
        assert_eq!(findings(source_text), Vec::<String>::new());
    }

    #[test]
    fn inline_module_bodies_are_checked_as_their_own_scope() {
        let source_text = concat!(
            "pub fn a() {}\n",
            "mod inner {\n",
            "    fn b() {}\n",
            "    pub fn c() {}\n",
            "    mod nested {\n",
            "        fn d() {}\n",
            "        pub const E: u8 = 0;\n",
            "    }\n",
            "}\n",
            "pub fn f() {}\n",
        );
        assert_eq!(
            findings(source_text),
            [
                "4:5: pub fn `c` should come before private fn `b`: pub items come before private \
                 items",
                "7:9: pub const `E` should come before private fn `d`: pub items come before \
                 private items",
            ]
        );
    }

    #[test]
    fn items_in_fn_bodies_are_not_checked() {
        let source_text = concat!(
            "pub fn a() {\n",
            "    fn b() {}\n",
            "    pub struct C;\n",
            "    impl C { fn d() {} pub fn e() {} }\n",
            "}\n",
        );
        assert_eq!(findings(source_text), Vec::<String>::new());
    }

    #[test]
    fn test_code_is_exempt_unless_tests_are_included() {
        let source_text = concat!(
            "fn a() {}\n",
            "#[cfg(test)]\n",
            "pub fn b() {}\n",
            "#[test]\n",
            "pub fn c() {}\n",
            "pub struct S;\n",
            "impl S {\n",
            "    fn d() {}\n",
            "    #[cfg(test)]\n",
            "    pub fn e() {}\n",
            "}\n",
            "#[cfg(test)]\n",
            "impl S { fn f() {} pub fn g() {} }\n",
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    fn h() {}\n",
            "    pub fn i() {}\n",
            "}\n",
        );
        assert_eq!(
            findings(source_text),
            [
                "1:1: private fn `a` should come after impl `S`: pub items come before private \
                 items"
            ]
        );
        assert_eq!(
            format_findings(&diagnostics(source_text, true)),
            [
                "1:1: private fn `a` should come after impl `S`: pub items come before private \
                 items",
                "3:1: pub fn `b` should come after impl `S`: types and their impls come before fns",
                "5:1: pub fn `c` should come after impl `S`: types and their impls come before fns",
                "10:5: pub fn `e` should come before private fn `d`: pub fns come before private \
                 fns",
                "13:20: pub fn `g` should come before private fn `f`: pub fns come before private \
                 fns",
                "17:5: pub fn `i` should come before private fn `h`: pub items come before \
                 private items",
            ]
        );
    }

    #[test]
    fn one_misplaced_item_is_one_diagnostic() {
        let source_text = concat!(
            "fn helper() {}\n",
            "pub struct A;\n",
            "impl A {}\n",
            "pub struct B;\n",
            "pub fn c() {}\n",
            "pub fn d() {}\n",
        );
        assert_eq!(
            findings(source_text),
            [
                "1:1: private fn `helper` should come after pub fn `d`: pub items come before \
                 private items"
            ]
        );
    }

    #[test]
    fn diagnostics_are_at_the_item_after_its_attributes_and_anchored_to_its_reference() {
        let source_text = concat!(
            "pub fn a() {}\n",
            "/// Doc.\n",
            "#[inline]\n",
            "fn b() {\n",
            "}\n",
            "pub fn c() {}\n",
            "pub fn d() {}\n",
            "#[derive(Debug)] pub struct S;\n",
        );
        let diagnostics = diagnostics(source_text, false);
        assert_eq!(
            format_findings(&diagnostics),
            [
                "4:1: private fn `b` should come after pub fn `d`: pub items come before private \
                 items",
                "8:18: pub struct `S` should come before pub fn `a`: types and their impls come \
                 before fns",
            ]
        );
        let spans: Vec<(String, Option<String>)> = diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.anchor().to_string(),
                    diagnostic.fn_header().map(|header| header.to_string()),
                )
            })
            .collect();
        assert_eq!(
            spans,
            [
                ("2-7".to_owned(), Some("2-4".to_owned())),
                ("1-8".to_owned(), Some("8".to_owned())),
            ]
        );
    }

    #[test]
    fn suppressions_above_an_item_cover_its_finding() {
        let source_text = concat!(
            "pub fn a() {}\n",
            "// yscope-rust-linter: allow(symbol-order): kept next to its caller\n",
            "/// Doc.\n",
            "fn b() {}\n",
            "pub fn c() {}\n",
            "pub fn d() {}\n",
        );
        let test_module_cache = TestModuleCache::default();
        let context = FileContext::new(Path::new(FIXTURE_PATH), source_text, &test_module_cache);
        let rule_diagnostics = Checker::new(false).check(&context);
        assert_eq!(
            format_findings(&rule_diagnostics),
            [
                "4:1: private fn `b` should come after pub fn `d`: pub items come before private \
                 items"
            ]
        );
        let rule_set: RuleSet = std::iter::once(RuleId::SymbolOrder).collect();
        assert_eq!(context.finish(rule_diagnostics, &rule_set), []);
    }
}
