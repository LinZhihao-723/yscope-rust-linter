//! The per-file input of the lint rules: a file's path and source, its comments and suppressions,
//! and its `syn` syntax tree, parsed on first use.

use std::cell::OnceCell;
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

/// What the rules see of one file, built once per file and shared by every enabled rule.
pub struct FileContext<'source> {
    path: &'source Path,
    source_text: &'source str,
    lex_error: Option<LexError>,
    parsed_comments: ParsedComments,
    syntax_tree: OnceCell<syn::Result<syn::File>>,
}

impl<'source> FileContext<'source> {
    /// Factory function.
    ///
    /// Lexes the file and splits its suppression comments from its other comments. A lex failure
    /// keeps the comments before it.
    ///
    /// # Returns
    ///
    /// The context of the file at `path` whose text is `source_text`.
    pub fn new(path: &'source Path, source_text: &'source str) -> Self {
        let lexed = lexer::lex(source_text);
        Self {
            path,
            source_text,
            lex_error: lexed.error,
            parsed_comments: suppression::parse(lexed.comment_lines, lexed.header_end),
            syntax_tree: OnceCell::new(),
        }
    }

    pub const fn path(&self) -> &Path {
        self.path
    }

    /// # Returns
    ///
    /// The comment lines that are not suppression comments, in source order.
    pub fn comment_lines(&self) -> &[CommentLine] {
        &self.parsed_comments.comment_lines
    }

    /// Gets the `syn` syntax tree, parsing the file on the first call.
    ///
    /// # Returns
    ///
    /// The syntax tree on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`None`] if the file failed to lex, since its lex failure is already a `parse-error`
    ///   diagnostic and `syn` can't parse it either.
    /// * A `parse-error` diagnostic that says `rule_id` is skipped, if `syn` fails to parse the
    ///   file.
    pub fn syntax_tree(&self, rule_id: RuleId) -> Result<&syn::File, Option<Diagnostic>> {
        let Some(parsed) = self.parse() else {
            return Err(None);
        };
        parsed.as_ref().map_err(|parse_error| {
            Some(Diagnostic::new(
                DiagnosticCode::ParseError,
                span::start_position(parse_error.span()),
                format!("syn failed to parse the file ({parse_error}); {rule_id} is skipped"),
            ))
        })
    }

    /// Completes the rules' diagnostics with the ones the file's lexing and suppressions produce:
    ///
    /// * A lex failure becomes a `parse-error` diagnostic.
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
            lex_diagnostic.into_iter().chain(rule_diagnostics).collect(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(source_text: &str) -> Vec<(DiagnosticCode, usize, String)> {
        FileContext::new(Path::new("a.rs"), source_text)
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
    fn allow_item_locates_its_item_in_the_lazily_parsed_tree() {
        let source_text = "// yscope-rust-linter: allow-item(errors-doc): reason\nfn f() {}\n";
        let context = FileContext::new(Path::new("a.rs"), source_text);
        let finding = Diagnostic::new(
            RuleId::ErrorsDoc.into(),
            Position { line: 2, column: 1 },
            "finding".to_owned(),
        );
        assert_eq!(context.finish(vec![finding], &RuleSet::all()), []);
    }
}
