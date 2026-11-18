//! The entry point of the `trait-params-doc` rule.

use syn::visit::Visit;

use crate::diagnostic::Diagnostic;
use crate::diagnostic::Position;
use crate::doc::doc_lines;
use crate::doc::param_section::PARAMETERS;
use crate::file_context::FileContext;
use crate::rules::RuleId;
use crate::syntax::doc_items;
use crate::syntax::doc_items::DocItem;
use crate::syntax::doc_items::DocItemKind;

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

    /// Checks every documented item of one file:
    ///
    /// * A trait method declaration with parameters other than its receiver must have a `#
    ///   Parameters` section with one bullet per parameter.
    /// * A trait method declaration without parameters must not have one.
    /// * Any other item must not have one.
    ///
    /// The file is skipped if it failed to lex or to parse, or if it is an out-of-line
    /// `#[cfg(test)]` module and tests aren't included.
    ///
    /// # Returns
    ///
    /// A `trait-params-doc` diagnostic for every mismatch between a trait method's parameters and
    /// its `# Parameters` section, and for every `# Parameters` section outside a trait method
    /// declaration. Each one's anchor span is the whole item, and its fn header is the lines from
    /// the item's first attribute or doc line through the item's own first line.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        let Some(syntax_tree) = context.syntax_tree(RuleId::TraitParamsDoc) else {
            return Vec::new();
        };
        if !self.include_tests && context.is_test_module(RuleId::TraitParamsDoc) {
            return Vec::new();
        }
        doc_items::collect(syntax_tree, self.include_tests)
            .iter()
            .flat_map(|doc_item| {
                item_findings(doc_item)
                    .into_iter()
                    .map(|(position, message)| {
                        Diagnostic::new(RuleId::TraitParamsDoc.into(), position, message)
                            .with_anchor(doc_item.lines.span)
                            .with_fn_header(doc_item.lines.header)
                    })
            })
            .collect()
    }
}

/// The parameter names of a trait method, without its receiver.
#[derive(Debug, Default)]
struct ParamNames {
    /// The names of the parameters bound by an identifier pattern, which must be documented.
    required: Vec<String>,

    /// The names bound inside destructuring patterns, which may be documented.
    accepted: Vec<String>,
}

impl ParamNames {
    /// Collects the parameter names of `sig`:
    ///
    /// * A parameter bound by an identifier pattern, such as `job_id` or `mut buffer`, is required.
    /// * The identifiers bound by any other pattern, such as `(a, b)`, are accepted. So a `_`
    ///   parameter has no name.
    ///
    /// # Returns
    ///
    /// The names, each list in declaration order.
    fn of(sig: &syn::Signature) -> Self {
        let mut param_names = Self::default();
        for input in &sig.inputs {
            let syn::FnArg::Typed(pat_type) = input else {
                continue;
            };
            if let syn::Pat::Ident(pat_ident) = &*pat_type.pat
                && pat_ident.subpat.is_none()
            {
                param_names.required.push(pat_ident.ident.to_string());
                continue;
            }
            let mut binding_collector = BindingCollector {
                names: &mut param_names.accepted,
            };
            binding_collector.visit_pat(&pat_type.pat);
        }
        param_names
    }

    const fn is_empty(&self) -> bool {
        self.required.is_empty() && self.accepted.is_empty()
    }
}

/// The `syn` visitor that collects the identifiers a pattern binds.
///
/// # Type Parameters
///
/// * `'names` - The lifetime of the list the names are added to.
struct BindingCollector<'names> {
    names: &'names mut Vec<String>,
}

impl Visit<'_> for BindingCollector<'_> {
    fn visit_pat_ident(&mut self, pat_ident: &syn::PatIdent) {
        self.names.push(pat_ident.ident.to_string());
        syn::visit::visit_pat_ident(self, pat_ident);
    }
}

/// Checks the `# Parameters` section of `doc_item`.
///
/// # Returns
///
/// The position and message of each finding. An undocumented item has none.
fn item_findings(doc_item: &DocItem<'_>) -> Vec<(Position, String)> {
    let Some(doc_lines) = doc_lines::collect_documented(doc_item.attrs) else {
        return Vec::new();
    };
    let first_doc_line = &doc_lines[0];
    let description = doc_item.description();
    let section = PARAMETERS.parse(&doc_lines);
    let DocItemKind::TraitMethod(sig) = doc_item.kind else {
        return section
            .map(|section| (section.header, misplaced_section_message(&description)))
            .into_iter()
            .collect();
    };
    let param_names = ParamNames::of(sig);
    match section {
        None if param_names.required.is_empty() => Vec::new(),
        None => vec![(
            first_doc_line.position,
            PARAMETERS.missing_section_message(&description, &param_names.required),
        )],
        Some(section) if param_names.is_empty() => vec![(
            section.header,
            PARAMETERS.no_parameters_message(&description),
        )],
        Some(section) => section
            .compare(&param_names.required, &param_names.accepted)
            .into_iter()
            .map(|finding| {
                (
                    finding.position,
                    PARAMETERS.finding_message(&finding.kind, &description),
                )
            })
            .collect(),
    }
}

/// # Returns
///
/// The message of a `# Parameters` section in the docstring of `item`, which isn't a trait method
/// declaration.
fn misplaced_section_message(item: &str) -> String {
    format!("`# Parameters` belongs only in trait method declarations; remove it from {item}")
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
            "trait Store {\n",
            "    /// Submits a job.\n",
            "    ///\n",
            "    /// # Parameters\n",
            "    ///\n",
            "    /// * `job_id` - The job.\n",
            "    /// * `inputs` - Its inputs.\n",
            "    fn submit(&self, job_id: JobId, mut inputs: Vec<u8>);\n",
            "\n",
            "    /// Closes the store.\n",
            "    fn close(self: Box<Self>);\n",
            "}\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
    }

    #[test]
    fn trait_methods_without_a_section_are_reported_at_the_first_doc_line() {
        let source_text = concat!(
            "trait Store {\n",
            "    /// Submits a job.\n",
            "    fn submit(&mut self, job_id: JobId, _context: Context, _: u8);\n",
            "\n",
            "    /// Loads a job.\n",
            "    fn load(job_id: JobId) -> Job {\n",
            "        todo!()\n",
            "    }\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "2:5: trait method `submit` has parameters but no `# Parameters` section; add one \
                 with a bullet for each of `job_id`, `_context`",
                "5:5: trait method `load` has parameters but no `# Parameters` section; add one \
                 with a bullet for each of `job_id`",
            ]
        );
    }

    #[test]
    fn names_bound_by_destructuring_patterns_are_accepted_but_not_required() {
        let source_text = concat!(
            "trait Geometry {\n",
            "    /// Moves to a point.\n",
            "    fn move_to(&self, (x, y): (u8, u8)) {}\n",
            "\n",
            "    /// Moves by an offset.\n",
            "    ///\n",
            "    /// # Parameters\n",
            "    ///\n",
            "    /// * `dx` - The horizontal offset.\n",
            "    /// * `name` - The point's name.\n",
            "    /// * `stale` - Not a parameter.\n",
            "    fn move_by(&self, Offset { dx, dy }: Offset, name @ _: &str) {}\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "11:5: `# Parameters` documents `stale`, which isn't a parameter of trait method \
                 `move_by`; remove the bullet"
            ]
        );
    }

    #[test]
    fn a_section_on_a_trait_method_without_parameters_is_reported_at_its_header() {
        let source_text = concat!(
            "trait Store {\n",
            "    /// Closes the store.\n",
            "    ///\n",
            "    /// # Parameters\n",
            "    ///\n",
            "    /// * `self` - The store.\n",
            "    fn close(&self, _: u8);\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            ["4:5: trait method `close` has no parameters; remove the `# Parameters` section"]
        );
    }

    #[test]
    fn mismatched_bullets_are_reported() {
        let source_text = concat!(
            "trait Store {\n",
            "    /// Submits a job.\n",
            "    ///\n",
            "    /// # Parameters\n",
            "    ///\n",
            "    /// * [`job_id`]: The job.\n",
            "    /// * `job_id` - The job again.\n",
            "    /// * The inputs.\n",
            "    fn submit(&self, job_id: JobId, inputs: Vec<u8>);\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "4:5: `# Parameters` doesn't document `inputs`; add \"* `inputs` - ...\"",
                "6:5: write this `# Parameters` bullet as \"* `job_id` - description\"",
                "7:5: `job_id` is already documented on line 6; remove this bullet",
                "8:5: this `# Parameters` bullet doesn't start with the parameter it documents; \
                 write \"* `name` - description\"",
            ]
        );
    }

    #[test]
    fn a_section_outside_a_trait_method_declaration_is_reported_at_its_header() {
        let source_text = concat!(
            "/// Runs.\n",
            "///\n",
            "/// # Parameters\n",
            "///\n",
            "/// * `stale` - The bullets aren't checked.\n",
            "fn run(job_id: JobId) {}\n",
            "impl Store for S {\n",
            "    /// Submits.\n",
            "    ///\n",
            "    /// # Parameters\n",
            "    ///\n",
            "    /// * `job_id` - The job.\n",
            "    fn submit(&self, job_id: JobId) {}\n",
            "}\n",
            "impl S {\n",
            "    /// Creates.\n",
            "    ///\n",
            "    /// # Parameters\n",
            "    fn new(job_id: JobId) -> Self { todo!() }\n",
            "}\n",
            "/// A store.\n",
            "///\n",
            "/// # Parameters\n",
            "trait Store {}\n",
            "/// A job.\n",
            "///\n",
            "/// # Parameters\n",
            "struct Job;\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "3:1: `# Parameters` belongs only in trait method declarations; remove it from fn \
                 `run`",
                "10:5: `# Parameters` belongs only in trait method declarations; remove it from \
                 trait impl method `submit`",
                "18:5: `# Parameters` belongs only in trait method declarations; remove it from \
                 method `new`",
                "23:1: `# Parameters` belongs only in trait method declarations; remove it from \
                 trait `Store`",
                "27:1: `# Parameters` belongs only in trait method declarations; remove it from \
                 struct `Job`",
            ]
        );
    }

    #[test]
    fn undocumented_trait_methods_are_not_checked() {
        let source_text = concat!(
            "trait Store {\n",
            "    fn submit(&self, job_id: JobId);\n",
            "    #[doc = \"\"]\n",
            "    fn load(&self, job_id: JobId);\n",
            "}\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
    }

    #[test]
    fn test_code_is_checked_only_with_include_tests() {
        let source_text = concat!(
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    trait Fixture {\n",
            "        /// Sets up.\n",
            "        fn set_up(&self, seed: u64);\n",
            "    }\n",
            "    /// Runs.\n",
            "    ///\n",
            "    /// # Parameters\n",
            "    #[test]\n",
            "    fn runs() {}\n",
            "}\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
        assert_eq!(
            checked(source_text, true),
            [
                "4:9: trait method `set_up` has parameters but no `# Parameters` section; add one \
                 with a bullet for each of `seed`",
                "9:5: `# Parameters` belongs only in trait method declarations; remove it from fn \
                 `runs`",
            ]
        );
    }

    #[test]
    fn diagnostics_are_anchored_to_the_item_with_its_header() {
        let source_text = concat!(
            "trait Store {\n",
            "    /// Submits.\n",
            "    #[must_use]\n",
            "    fn submit(&self, job_id: JobId) -> bool {\n",
            "        true\n",
            "    }\n",
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
            [("2-6".to_owned(), Some("2-4".to_owned()))]
        );
    }
}
