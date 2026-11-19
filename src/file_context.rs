//! The per-file input of the lint rules: a file's path and source, its comments and suppressions,
//! its `syn` syntax tree, parsed on first use, and whether it is an out-of-line test module.

use std::cell::OnceCell;
use std::cell::RefCell;
use std::path::Path;

use crate::config::RuleSet;
use crate::diagnostic::Diagnostic;
use crate::diagnostic::DiagnosticCode;
use crate::diagnostic::Position;
use crate::lexer;
use crate::lexer::CommentLine;
use crate::lexer::LexError;
use crate::rules::RuleId;
use crate::suppression;
use crate::suppression::ParsedComments;
use crate::syntax::item_lines;
use crate::syntax::item_lines::ItemLines;
use crate::syntax::span;
use crate::syntax::test_code::TestModuleCache;
use crate::syntax::test_code::TestModuleVerdict;

/// What the rules see of one file, built once per file and shared by every enabled rule.
///
/// # Type Parameters
///
/// * `'source` - The lifetime of the file's path and text, and of the run's test-module cache.
pub struct FileContext<'source> {
    path: &'source Path,
    source_text: &'source str,
    lex_error: Option<LexError>,
    parsed_comments: ParsedComments,
    syntax_tree: OnceCell<syn::Result<syn::File>>,
    test_module_cache: &'source TestModuleCache,
    test_module_verdict: OnceCell<TestModuleVerdict>,

    /// The rules that asked for the syntax tree of a file that `syn` fails to parse.
    rules_skipped_by_parse_error: RefCell<Vec<RuleId>>,

    /// The rules that asked whether the file is a test module when the answer may be wrong.
    rules_unsure_of_test_module: RefCell<Vec<RuleId>>,
}

impl<'source> FileContext<'source> {
    /// Factory function.
    ///
    /// Lexes the file and splits its suppression comments from its other comments. A lex failure
    /// keeps the comments before it.
    ///
    /// # Returns
    ///
    /// The context of the file at `path` whose text is `source_text`, which decides whether the
    /// file is an out-of-line test module with the run's `test_module_cache`.
    pub fn new(
        path: &'source Path,
        source_text: &'source str,
        test_module_cache: &'source TestModuleCache,
    ) -> Self {
        let lexed = lexer::lex(source_text);
        Self {
            path,
            source_text,
            lex_error: lexed.error,
            parsed_comments: suppression::parse(lexed.comment_lines, lexed.header_end),
            syntax_tree: OnceCell::new(),
            test_module_cache,
            test_module_verdict: OnceCell::new(),
            rules_skipped_by_parse_error: RefCell::new(Vec::new()),
            rules_unsure_of_test_module: RefCell::new(Vec::new()),
        }
    }

    /// # Returns
    ///
    /// The comment lines that are not suppression comments, in source order.
    pub fn comment_lines(&self) -> &[CommentLine] {
        &self.parsed_comments.comment_lines
    }

    /// Gets the `syn` syntax tree for the rule `rule_id`, parsing the file on the first call.
    ///
    /// If `syn` fails to parse the file, `rule_id` is recorded as skipped, and [`Self::finish`]
    /// reports one `parse-error` diagnostic that names every skipped rule. A file that failed to
    /// lex has no tree either, since its lex failure is already a `parse-error` diagnostic and
    /// `syn` can't parse it.
    ///
    /// # Returns
    ///
    /// The syntax tree, or [`None`] if the file failed to lex or to parse.
    pub fn syntax_tree(&self, rule_id: RuleId) -> Option<&syn::File> {
        let parsed = self.parse()?;
        if parsed.is_err() {
            record(&self.rules_skipped_by_parse_error, rule_id);
        }
        parsed.as_ref().ok()
    }

    /// Gets the `syn` syntax tree for a rule that can do without it, parsing the file on the first
    /// call. Unlike [`Self::syntax_tree`], a parse failure is not reported, since the rule skips
    /// nothing without the tree.
    ///
    /// # Returns
    ///
    /// The syntax tree, or [`None`] if the file failed to lex or to parse.
    pub fn optional_syntax_tree(&self) -> Option<&syn::File> {
        self.parse()?.as_ref().ok()
    }

    /// Decides for the rule `rule_id` whether the file is an out-of-line test module, as
    /// [`TestModuleCache::verdict`] describes. The verdict is computed on the first call.
    ///
    /// If a candidate declaring file can't be read or parsed, `rule_id` is recorded, and
    /// [`Self::finish`] reports one `parse-error` diagnostic at the file's first line that says why
    /// and names every rule that asked.
    ///
    /// # Returns
    ///
    /// Whether the file is an out-of-line test module.
    pub fn is_test_module(&self, rule_id: RuleId) -> bool {
        let verdict = self
            .test_module_verdict
            .get_or_init(|| self.test_module_cache.verdict(self.path));
        if verdict.problem.is_some() {
            record(&self.rules_unsure_of_test_module, rule_id);
        }
        verdict.is_test_module
    }

    /// Completes the rules' diagnostics with the ones the file's lexing, parsing and suppressions
    /// produce:
    ///
    /// * A lex failure becomes a `parse-error` diagnostic.
    /// * A `syn` failure that skipped some rule becomes a `parse-error` diagnostic that names the
    ///   skipped rules.
    /// * A test-module verdict that may be wrong becomes a `parse-error` diagnostic at the first
    ///   line, which names the rules that asked for it.
    /// * The items of `allow-item` suppressions are located in the syntax tree, which is parsed
    ///   only if the file has such a suppression. If the file fails to lex or parse, each of them
    ///   is an `invalid-suppression` diagnostic.
    /// * Suppressions drop the diagnostics they cover, and unused suppressions are reported, as
    ///   [`suppression::Suppressions::apply`] decides with `rule_set`.
    /// * Each malformed or misplaced suppression is an `invalid-suppression` diagnostic.
    ///
    /// # Returns
    ///
    /// The file's diagnostics, unsorted.
    pub fn finish(self, rule_diagnostics: Vec<Diagnostic>, rule_set: &RuleSet) -> Vec<Diagnostic> {
        let item_lines = self
            .parsed_comments
            .suppressions
            .has_unlocated_items()
            .then(|| self.item_lines());
        let meta_diagnostics: Vec<Diagnostic> = self
            .parse_error_diagnostic()
            .into_iter()
            .chain(self.test_module_diagnostic())
            .collect();
        let Self {
            lex_error,
            mut parsed_comments,
            ..
        } = self;
        let mut invalid_suppressions = parsed_comments.invalid_suppressions;
        match item_lines {
            Some(Ok(item_lines)) => {
                invalid_suppressions.extend(parsed_comments.suppressions.locate_items(&item_lines));
            }
            Some(Err(reason)) => {
                invalid_suppressions.extend(parsed_comments.suppressions.reject_items(&reason));
            }
            None => {}
        }
        let lex_diagnostic = lex_error.map(|lex_error| {
            Diagnostic::new(
                DiagnosticCode::ParseError,
                Position {
                    line: lex_error.line,
                    column: lex_error.column,
                },
                format!(
                    "unterminated {}; only the comments before it are checked",
                    lex_error.kind
                ),
            )
        });
        let mut diagnostics = parsed_comments.suppressions.apply(
            lex_diagnostic
                .into_iter()
                .chain(meta_diagnostics)
                .chain(rule_diagnostics)
                .collect(),
            rule_set,
        );
        diagnostics.extend(invalid_suppressions);
        diagnostics
    }

    /// Parses the file with `syn` on the first call.
    ///
    /// # Returns
    ///
    /// The result of parsing the file, or [`None`] if the file failed to lex.
    fn parse(&self) -> Option<&syn::Result<syn::File>> {
        self.lex_error.is_none().then(|| {
            self.syntax_tree
                .get_or_init(|| syn::parse_file(self.source_text))
        })
    }

    /// # Returns
    ///
    /// The `parse-error` diagnostic of a `syn` failure, if some rule was skipped because of it.
    fn parse_error_diagnostic(&self) -> Option<Diagnostic> {
        let skipped_rules = self.rules_skipped_by_parse_error.borrow();
        if skipped_rules.is_empty() {
            return None;
        }
        let Some(Err(parse_error)) = self.syntax_tree.get() else {
            return None;
        };
        let verb = if skipped_rules.len() == 1 {
            "is"
        } else {
            "are"
        };
        Some(Diagnostic::new(
            DiagnosticCode::ParseError,
            span::start_position(parse_error.span()),
            format!(
                "syn failed to parse the file ({parse_error}); {} {verb} skipped",
                joined(&skipped_rules)
            ),
        ))
    }

    /// # Returns
    ///
    /// The `parse-error` diagnostic of a test-module verdict that may be wrong, if some rule asked
    /// for it.
    fn test_module_diagnostic(&self) -> Option<Diagnostic> {
        let unsure_rules = self.rules_unsure_of_test_module.borrow();
        if unsure_rules.is_empty() {
            return None;
        }
        let problem = self.test_module_verdict.get()?.problem.as_ref()?;
        let verb = if unsure_rules.len() == 1 {
            "checks"
        } else {
            "check"
        };
        Some(Diagnostic::new(
            DiagnosticCode::ParseError,
            Position { line: 1, column: 1 },
            format!(
                "can't tell whether this file is a `#[cfg(test)]` module: {problem}; {} {verb} it \
                 as non-test code",
                joined(&unsure_rules)
            ),
        ))
    }

    /// Locates the file's items in its syntax tree.
    ///
    /// # Returns
    ///
    /// The lines of the file's items on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * A description of the failure, if the file failed to lex or `syn` fails to parse it.
    fn item_lines(&self) -> Result<Vec<ItemLines>, String> {
        match self.parse() {
            Some(Ok(syntax_tree)) => Ok(item_lines::collect(syntax_tree)),
            Some(Err(parse_error)) => Err(format!("syn failed to parse the file ({parse_error})")),
            None => Err("the file failed to lex".to_owned()),
        }
    }
}

/// Adds `rule_id` to `rule_ids` unless it is already there.
fn record(rule_ids: &RefCell<Vec<RuleId>>, rule_id: RuleId) {
    let mut rule_ids = rule_ids.borrow_mut();
    if !rule_ids.contains(&rule_id) {
        rule_ids.push(rule_id);
    }
}

/// # Returns
///
/// The IDs of `rule_ids`, in [`RuleId::ALL`] order, separated by `, `.
fn joined(rule_ids: &[RuleId]) -> String {
    RuleId::ALL
        .into_iter()
        .filter(|rule_id| rule_ids.contains(rule_id))
        .map(RuleId::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(source_text: &str) -> Vec<(DiagnosticCode, usize, String)> {
        let test_module_cache = TestModuleCache::default();
        FileContext::new(Path::new("a.rs"), source_text, &test_module_cache)
            .finish(Vec::new(), &RuleSet::all())
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.code(),
                    diagnostic.position().line,
                    diagnostic.message().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn allow_item_is_invalid_when_the_file_fails_to_parse_or_lex() {
        let allow_item = "// yscope-rust-linter: allow-item(errors-doc): reason\n";
        assert_eq!(
            finished(&format!("{allow_item}fn f( {{}}\n")),
            [(
                DiagnosticCode::InvalidSuppression,
                1,
                "can't locate the item of this `allow-item` suppression: syn failed to parse the \
                 file (cannot parse string into token stream)"
                    .to_owned()
            )]
        );
        let codes: Vec<_> = finished(&format!("{allow_item}fn f() {{\n    \"open\n"))
            .into_iter()
            .map(|(code, line, message)| (code, line, message.contains("failed to lex")))
            .collect();
        assert_eq!(
            codes,
            [
                (DiagnosticCode::ParseError, 3, false),
                (DiagnosticCode::InvalidSuppression, 1, true),
            ]
        );
    }

    #[test]
    fn syn_failure_is_one_parse_error_that_names_every_skipped_rule() {
        let test_module_cache = TestModuleCache::default();
        let context = FileContext::new(Path::new("a.rs"), "fn f( {}\n", &test_module_cache);
        for rule_id in [
            RuleId::TypeParamsDoc,
            RuleId::ErrorsDoc,
            RuleId::TypeParamsDoc,
        ] {
            assert!(context.syntax_tree(rule_id).is_none());
        }
        let messages: Vec<String> = context
            .finish(Vec::new(), &RuleSet::all())
            .iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.code(), diagnostic.message()))
            .collect();
        assert_eq!(
            messages,
            [
                "parse-error: syn failed to parse the file (cannot parse string into token \
                 stream); errors-doc, type-params-doc are skipped"
            ]
        );

        let context = FileContext::new(Path::new("a.rs"), "fn f( {}\n", &test_module_cache);
        assert!(context.syntax_tree(RuleId::SymbolOrder).is_none());
        assert_eq!(
            context.finish(Vec::new(), &RuleSet::all())[0].message(),
            "syn failed to parse the file (cannot parse string into token stream); symbol-order \
             is skipped"
        );
        assert_eq!(finished("fn f( {}\n"), []);
    }

    #[test]
    fn allow_item_locates_its_item_in_the_lazily_parsed_tree() {
        let source_text = "// yscope-rust-linter: allow-item(errors-doc): reason\nfn f() {}\n";
        let test_module_cache = TestModuleCache::default();
        let context = FileContext::new(Path::new("a.rs"), source_text, &test_module_cache);
        let finding = Diagnostic::new(
            RuleId::ErrorsDoc.into(),
            Position { line: 2, column: 1 },
            "finding".to_owned(),
        );
        assert_eq!(context.finish(vec![finding], &RuleSet::all()), []);
    }
}
