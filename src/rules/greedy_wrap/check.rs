//! The entry point of the `comment-greedy-wrap` rule: the fit test and its diagnostics.

use crate::config::Width;
use crate::diagnostic::Diagnostic;
use crate::diagnostic::Position;
use crate::diagnostic::counted;
use crate::file_context::FileContext;
use crate::line_ranges::LineSpan;
use crate::rules::RuleId;
use crate::rules::greedy_wrap::line_kind::Body;
use crate::rules::greedy_wrap::paragraph;
use crate::rules::greedy_wrap::paragraph::JoinablePair;

/// The `comment-greedy-wrap` rule, with the width of a run.
#[derive(Debug)]
pub struct Checker {
    width: Width,
}

impl Checker {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// A checker that fills comment paragraphs to `width` columns.
    pub const fn new(width: Width) -> Self {
        Self { width }
    }

    /// Checks that every comment paragraph of one file is filled greedily to the width. Suppression
    /// comments are not part of any paragraph.
    ///
    /// # Returns
    ///
    /// A `comment-greedy-wrap` diagnostic for every line that leaves room for the next line's first
    /// word.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        paragraph::blocks(context.comment_lines())
            .flat_map(paragraph::joinable_pairs)
            .filter_map(|pair| underfilled_line_diagnostic(pair, self.width))
            .collect()
    }
}

/// Applies the fit test to one joinable pair: the line is under-filled if the next line's first
/// word, after one space, would still end within `width` columns.
///
/// The diagnostic points at the line's marker. Its anchor span covers both lines, since a change to
/// either can create the violation.
///
/// # Returns
///
/// A diagnostic if the line is under-filled, or [`None`] otherwise.
///
/// # Panics
///
/// Panics if the pair's next line comes before its line, which [`paragraph::joinable_pairs`] never
/// produces.
fn underfilled_line_diagnostic(pair: JoinablePair<'_>, width: Width) -> Option<Diagnostic> {
    let JoinablePair { line, next_line } = pair;
    let first_word = Body::new(&next_line.text).first_word();
    let word_width = first_word.chars().count();
    if line.width + 1 + word_width > width.columns() {
        return None;
    }
    let message = format!(
        "line is {} wide and the next line's first word `{first_word}` ({}) fits within {}; move \
         it up",
        counted(line.width, "column"),
        counted(word_width, "column"),
        width.columns()
    );
    let position = Position {
        line: line.line,
        column: line.marker_column,
    };
    let anchor = LineSpan::new(line.line, next_line.line)
        .expect("a joinable pair's next line follows its line");
    Some(Diagnostic::new(RuleId::CommentGreedyWrap.into(), position, message).with_anchor(anchor))
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::path::Path;

    use super::*;
    use crate::lexer;

    fn diagnostics(source_text: &str, columns: usize) -> Vec<Diagnostic> {
        assert_eq!(lexer::lex(source_text).error, None, "{source_text:?}");
        let width = Width::new(NonZeroUsize::new(columns).expect("test widths are non-zero"));
        Checker::new(width).check(&FileContext::new(Path::new("a.rs"), source_text))
    }

    #[test]
    fn fit_test_is_exact_at_the_width() {
        // `// aaaa` is 7 columns wide, and ` bbb` brings it to 11.
        let source_text = "// aaaa\n// bbb\n";
        assert_eq!(diagnostics(source_text, 10), []);
        let flagged = diagnostics(source_text, 11);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].position(), Position { line: 1, column: 1 });
        assert_eq!(flagged[0].code(), RuleId::CommentGreedyWrap.into());
        assert_eq!(
            flagged[0].message(),
            "line is 7 columns wide and the next line's first word `bbb` (3 columns) fits within \
             11; move it up"
        );
    }

    #[test]
    fn one_column_first_word_is_singular_in_the_message() {
        let flagged = diagnostics("// a\n// b\n", 100);
        let messages: Vec<_> = flagged.iter().map(Diagnostic::message).collect();
        assert_eq!(
            messages,
            [
                "line is 4 columns wide and the next line's first word `b` (1 column) fits within \
                 100; move it up"
            ]
        );
    }

    #[test]
    fn width_counts_the_whole_source_line_in_scalars() {
        // `    /// éé` is 10 columns wide, and ` x` brings it to 12.
        let source_text = "    /// éé\n    /// x\n";
        assert_eq!(diagnostics(source_text, 11), []);
        let flagged = diagnostics(source_text, 12);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].position(), Position { line: 1, column: 5 });
    }

    #[test]
    fn anchor_covers_the_line_and_the_next_line() {
        let flagged = diagnostics("/// a\n/// b\n", 100);
        let anchors: Vec<_> = flagged.iter().map(Diagnostic::anchor).collect();
        assert_eq!(anchors, [LineSpan::new(1, 2).expect("valid span")]);
    }

    #[test]
    fn slashes_in_literals_and_block_comments_are_not_checked() {
        let source_text = "let a = \"\n// a\n// b\n\";\n/*\n// c\n// d\n*/\n";
        assert_eq!(diagnostics(source_text, 100), []);
    }
}
