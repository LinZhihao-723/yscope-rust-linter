//! The entry point of the `type-params-doc` rule.

use crate::diagnostic::Diagnostic;
use crate::diagnostic::Position;
use crate::doc::doc_lines;
use crate::doc::param_section::TYPE_PARAMETERS;
use crate::file_context::FileContext;
use crate::rules::RuleId;
use crate::syntax::doc_items;
use crate::syntax::doc_items::DocItem;
use crate::syntax::doc_items::DocItemKind;

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

    /// Checks every documented item of one file but its trait impl methods:
    ///
    /// * An item with generic parameters must have a `# Type Parameters` section with one bullet
    ///   per parameter.
    /// * An item without generic parameters must not have one.
    ///
    /// The file is skipped if it failed to lex or to parse, or if it is an out-of-line
    /// `#[cfg(test)]` module and tests aren't included.
    ///
    /// # Returns
    ///
    /// A `type-params-doc` diagnostic for every mismatch between an item's generic parameters and
    /// its `# Type Parameters` section. Each one's anchor span is the whole item, and its fn header
    /// is the lines from the item's first attribute or doc line through the item's own first line.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        let Some(syntax_tree) = context.syntax_tree(RuleId::TypeParamsDoc) else {
            return Vec::new();
        };
        if !self.include_tests && context.is_test_module(RuleId::TypeParamsDoc) {
            return Vec::new();
        }
        doc_items::collect(syntax_tree, self.include_tests)
            .iter()
            .filter(|doc_item| !matches!(doc_item.kind, DocItemKind::TraitImplMethod(_)))
            .flat_map(|doc_item| {
                item_findings(doc_item)
                    .into_iter()
                    .map(|(position, message)| {
                        Diagnostic::new(RuleId::TypeParamsDoc.into(), position, message)
                            .with_anchor(doc_item.lines.span)
                            .with_fn_header(doc_item.lines.header)
                    })
            })
            .collect()
    }
}

/// Compares the `# Type Parameters` section of `doc_item` with its generic parameters.
///
/// # Returns
///
/// The position and message of each finding. An undocumented item has none.
fn item_findings(doc_item: &DocItem<'_>) -> Vec<(Position, String)> {
    let Some(doc_lines) = doc_lines::collect_documented(doc_item.attrs) else {
        return Vec::new();
    };
    let first_doc_line = &doc_lines[0];
    let names = generic_param_names(doc_item.generics());
    let description = doc_item.description();
    match TYPE_PARAMETERS.parse(&doc_lines) {
        None if names.is_empty() => Vec::new(),
        None => vec![(
            first_doc_line.position,
            TYPE_PARAMETERS.missing_section_message(&description, &names),
        )],
        Some(section) if names.is_empty() => vec![(
            section.header,
            TYPE_PARAMETERS.no_parameters_message(&description),
        )],
        Some(section) => section
            .compare(&names, &[])
            .into_iter()
            .map(|finding| {
                (
                    finding.position,
                    TYPE_PARAMETERS.finding_message(&finding.kind, &description),
                )
            })
            .collect(),
    }
}

/// # Returns
///
/// The names of the parameters `generics` declares, in declaration order. A lifetime keeps its
/// tick, such as `'a`.
fn generic_param_names(generics: &syn::Generics) -> Vec<String> {
    generics
        .params
        .iter()
        .map(|param| match param {
            syn::GenericParam::Lifetime(lifetime_param) => lifetime_param.lifetime.to_string(),
            syn::GenericParam::Type(type_param) => type_param.ident.to_string(),
            syn::GenericParam::Const(const_param) => const_param.ident.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::syntax::test_code::TestModuleCache;

    const NO_FINDINGS: [String; 0] = [];

    /// Checks `source_text` as a file that no other file declares.
    ///
    /// # Returns
    ///
    /// Each finding as `<line>:<column>: <message>`, sorted by position.
    ///
    /// # Panics
    ///
    /// Panics if `source_text` doesn't parse.
    fn checked(source_text: &str, include_tests: bool) -> Vec<String> {
        diagnostics(source_text, include_tests)
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

    fn diagnostics(source_text: &str, include_tests: bool) -> Vec<Diagnostic> {
        syn::parse_file(source_text).expect("valid file");
        let test_module_cache = TestModuleCache::default();
        let context = FileContext::new(
            Path::new("nonexistent-fixture-directory/subject.rs"),
            source_text,
            &test_module_cache,
        );
        let mut diagnostics = Checker::new(include_tests).check(&context);
        diagnostics.sort_by_key(Diagnostic::position);
        diagnostics
    }

    #[test]
    fn sections_that_document_every_parameter_are_clean() {
        let source_text = concat!(
            "/// Loads.\n",
            "///\n",
            "/// # Type Parameters\n",
            "///\n",
            "/// * `'cache_lifetime` - The cache's lifetime.\n",
            "/// * `StorageType` - The storage.\n",
            "/// * `LIMIT` - The limit.\n",
            "fn load<'cache_lifetime, StorageType: Storage, const LIMIT: usize>() {}\n",
            "\n",
            "/// A job.\n",
            "///\n",
            "/// # Type Parameters\n",
            "///\n",
            "/// * `Payload` - The payload.\n",
            "struct Job<Payload> {\n",
            "    payload: Payload,\n",
            "}\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
    }

    #[test]
    fn items_of_every_checked_kind_without_a_section_are_reported_at_the_first_doc_line() {
        let source_text = concat!(
            "/// A.\n",
            "struct S<'a, T, const N: usize>(&'a [T; N]);\n",
            "/// A.\n",
            "enum E<T> { A(T) }\n",
            "/// A.\n",
            "union U<T: Copy> { a: T }\n",
            "/// A.\n",
            "trait Tr<T> {\n",
            "    /// A.\n",
            "    fn declared<D>();\n",
            "    /// A.\n",
            "    fn provided<P>() {}\n",
            "}\n",
            "/// A.\n",
            "type Alias<T> = Vec<T>;\n",
            "impl S<'_, u8, 1> {\n",
            "    /// A.\n",
            "    fn inherent<I>() {\n",
            "        /// A.\n",
            "        fn nested<N>() {}\n",
            "    }\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "1:1: struct `S` has generic parameters but no `# Type Parameters` section; add \
                 one with a bullet for each of `'a`, `T`, `N`",
                "3:1: enum `E` has generic parameters but no `# Type Parameters` section; add one \
                 with a bullet for each of `T`",
                "5:1: union `U` has generic parameters but no `# Type Parameters` section; add \
                 one with a bullet for each of `T`",
                "7:1: trait `Tr` has generic parameters but no `# Type Parameters` section; add \
                 one with a bullet for each of `T`",
                "9:5: trait method `declared` has generic parameters but no `# Type Parameters` \
                 section; add one with a bullet for each of `D`",
                "11:5: trait method `provided` has generic parameters but no `# Type Parameters` \
                 section; add one with a bullet for each of `P`",
                "14:1: type alias `Alias` has generic parameters but no `# Type Parameters` \
                 section; add one with a bullet for each of `T`",
                "17:5: method `inherent` has generic parameters but no `# Type Parameters` \
                 section; add one with a bullet for each of `I`",
                "19:9: fn `nested` has generic parameters but no `# Type Parameters` section; add \
                 one with a bullet for each of `N`",
            ]
        );
    }

    #[test]
    fn a_section_on_an_item_without_generic_parameters_is_reported_at_its_header() {
        let source_text = concat!(
            "/// Runs.\n",
            "///\n",
            "/// # Type Parameters\n",
            "///\n",
            "/// * `T` - Stale.\n",
            "fn run() {}\n",
        );
        assert_eq!(
            checked(source_text, false),
            ["3:1: fn `run` has no generic parameters; remove the `# Type Parameters` section"]
        );
    }

    #[test]
    fn mismatched_bullets_are_reported() {
        let source_text = concat!(
            "/// Runs.\n",
            "///\n",
            "/// # Type Parameters\n",
            "///\n",
            "/// * `a` - The lifetime without its tick.\n",
            "/// * `T` - The item.\n",
            "/// * `T` - The item again.\n",
            "/// * [`Sink`]: The sink.\n",
            "/// * The limit.\n",
            "///\n",
            "/// # Type Parameters\n",
            "fn run<'a, T, Sink, const LIMIT: usize>() {}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "3:1: `# Type Parameters` doesn't document `'a`; add \"* `'a` - ...\"",
                "3:1: `# Type Parameters` doesn't document `LIMIT`; add \"* `LIMIT` - ...\"",
                "5:1: `# Type Parameters` documents `a`, which isn't a generic parameter of fn \
                 `run`; remove the bullet",
                "7:1: `T` is already documented on line 6; remove this bullet",
                "8:1: write this `# Type Parameters` bullet as \"* `Sink` - description\"",
                "9:1: this `# Type Parameters` bullet doesn't start with the generic parameter it \
                 documents; write \"* `name` - description\"",
                "11:1: the docstring has a second `# Type Parameters` section at the same level; \
                 merge them",
            ]
        );
    }

    #[test]
    fn a_prose_only_section_reports_only_the_missing_parameters() {
        let source_text = concat!(
            "/// A handle.\n",
            "///\n",
            "/// # Type Parameters\n",
            "///\n",
            "/// The parameters are forwarded to [`State`]. Short names are used to:\n",
            "///\n",
            "/// * Reduce verbosity.\n",
            "/// * Avoid formatting issues.\n",
            "struct Handle<R, T> {\n",
            "    state: State<R, T>,\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "3:1: `# Type Parameters` doesn't document `R`; add \"* `R` - ...\"",
                "3:1: `# Type Parameters` doesn't document `T`; add \"* `T` - ...\"",
            ]
        );
    }

    #[test]
    fn trait_impl_methods_are_not_checked() {
        let source_text = concat!(
            "impl<T> Tr<T> for S {\n",
            "    /// A.\n",
            "    fn declared<D>() {}\n",
            "    /// A.\n",
            "    ///\n",
            "    /// # Type Parameters\n",
            "    ///\n",
            "    /// * `T` - The impl's parameter.\n",
            "    fn provided() {}\n",
            "}\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
    }

    #[test]
    fn undocumented_items_are_not_checked() {
        let source_text = concat!(
            "fn run<T>() {}\n",
            "///\n",
            "#[doc = \"  \"]\n",
            "struct S<T>(T);\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
    }

    #[test]
    fn only_an_items_own_named_parameters_are_its_generic_parameters() {
        let source_text = concat!(
            "impl<Outer> S<Outer> {\n",
            "    /// A method that only uses the impl's parameter.\n",
            "    fn get(&self, _: &'_ Outer, _: impl Fn(&u8)) -> &Outer { todo!() }\n",
            "}\n",
            "trait Tr<T> {\n",
            "    /// A method that only uses the trait's parameter.\n",
            "    fn get(&self) -> T;\n",
            "}\n",
            "/// A fn with only a higher-ranked lifetime.\n",
            "fn call(_: &dyn for<'x> Fn(&'x u8)) {}\n",
            "impl<Outer> S<Outer> {\n",
            "    /// A method that documents the impl's parameter.\n",
            "    ///\n",
            "    /// # Type Parameters\n",
            "    ///\n",
            "    /// * `Inner` - The inner value.\n",
            "    /// * `Outer` - The impl's parameter.\n",
            "    fn map<Inner>(&self) {}\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "17:5: `# Type Parameters` documents `Outer`, which isn't a generic parameter of \
                 method `map`; remove the bullet"
            ]
        );
    }

    #[test]
    fn test_code_is_checked_only_with_include_tests() {
        let source_text = concat!(
            "/// A.\n",
            "#[test]\n",
            "fn unit_test<T>() {}\n",
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    /// A.\n",
            "    struct Fixture<T>(T);\n",
            "}\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
        assert_eq!(
            checked(source_text, true),
            [
                "1:1: fn `unit_test` has generic parameters but no `# Type Parameters` section; \
                 add one with a bullet for each of `T`",
                "6:5: struct `Fixture` has generic parameters but no `# Type Parameters` section; \
                 add one with a bullet for each of `T`",
            ]
        );
    }

    #[test]
    fn diagnostics_are_anchored_to_the_item_with_its_header() {
        let source_text = concat!(
            "/// A.\n",
            "#[derive(Debug)]\n",
            "struct S<T> {\n",
            "    value: T,\n",
            "}\n",
        );
        let diagnostics = diagnostics(source_text, false);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (
                    diagnostic.anchor().to_string(),
                    diagnostic.fn_header().map(|header| header.to_string())
                ))
                .collect::<Vec<_>>(),
            [("1-5".to_owned(), Some("1-3".to_owned()))]
        );
    }
}
