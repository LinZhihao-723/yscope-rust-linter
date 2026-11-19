//! The entry point of the `comment-greedy-wrap` rule: the fit test and its diagnostics.

use crate::config::Width;
use crate::diagnostic::Diagnostic;
use crate::diagnostic::Position;
use crate::diagnostic::counted;
use crate::file_context::FileContext;
use crate::line_ranges::LineSpan;
use crate::markdown::BlockStart;
use crate::rules::RuleId;
use crate::rules::greedy_wrap::line_kind::Body;
use crate::rules::greedy_wrap::paragraph;
use crate::rules::greedy_wrap::paragraph::JoinablePair;
use crate::rules::greedy_wrap::rustfmt_lists;

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
    /// A move that would fill a line to exactly the width gets a hint when the line is in a field
    /// or parameter list, where rustfmt wraps comments one column early. A file that fails to parse
    /// gets no hint.
    ///
    /// # Returns
    ///
    /// A `comment-greedy-wrap` diagnostic for every line that leaves room for the leading words of
    /// the next line.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        let underfilled_lines: Vec<_> = paragraph::blocks(context.comment_lines())
            .flat_map(paragraph::joinable_pairs)
            .filter_map(|joinable_pair| UnderfilledLine::find(joinable_pair, self.width))
            .collect();
        let early_wrap_lines = underfilled_lines
            .iter()
            .any(|underfilled_line| underfilled_line.fills_width(self.width))
            .then(|| context.optional_syntax_tree())
            .flatten()
            .map(rustfmt_lists::early_wrap_lines);
        underfilled_lines
            .into_iter()
            .map(|underfilled_line| {
                let is_wrapped_early = underfilled_line.fills_width(self.width)
                    && early_wrap_lines.as_ref().is_some_and(|line_ranges| {
                        line_ranges
                            .intersects(LineSpan::single(underfilled_line.joinable_pair.line.line))
                    });
                underfilled_line.diagnostic(self.width, is_wrapped_early)
            })
            .collect()
    }
}

/// A line that leaves room for the leading words of the next line of its paragraph.
///
/// # Type Parameters
///
/// * `'comment` - The lifetime of the comment lines.
struct UnderfilledLine<'comment> {
    joinable_pair: JoinablePair<'comment>,

    /// The number of the next line's leading words in [`Self::moved`].
    num_words: usize,

    /// The fewest leading words of the next line that make a valid move, as the next line's text
    /// holds them.
    moved: &'comment str,
}

impl<'comment> UnderfilledLine<'comment> {
    /// Applies the fit test to one joinable pair: the line is under-filled if the next line's first
    /// `k` words, for some `k` of at least 1, would still end within `width` columns after one
    /// space, and what is left of the next line is a valid continuation. It is valid when it is
    /// empty or opens no Markdown block that interrupts a paragraph, so the move can't turn it into
    /// a list item, a heading or another block.
    ///
    /// # Returns
    ///
    /// The under-filled line with the fewest words that make a valid move, or [`None`] if the line
    /// isn't under-filled.
    fn find(joinable_pair: JoinablePair<'comment>, width: Width) -> Option<Self> {
        let next_text = Body::new(&joinable_pair.next_line.text).trimmed();
        let room = width.columns().checked_sub(joinable_pair.line.width + 1)?;
        word_ends(next_text)
            .map(|word_end| next_text.split_at(word_end))
            .take_while(|(moved, _)| moved.chars().count() <= room)
            .enumerate()
            .find(|(_, (_, rest))| {
                let rest = rest.trim_start();
                rest.is_empty() || BlockStart::parse(rest, joinable_pair.preceding).is_none()
            })
            .map(|(index, (moved, _))| Self {
                joinable_pair,
                num_words: index + 1,
                moved,
            })
    }

    /// # Returns
    ///
    /// Whether the move would make the line exactly `width` columns wide.
    fn fills_width(&self, width: Width) -> bool {
        self.joinable_pair.line.width + 1 + self.moved.chars().count() == width.columns()
    }

    /// Builds the line's diagnostic, which names the words to move. With `is_wrapped_early`, it
    /// adds that rustfmt wraps the comment one column early, so the paragraph needs rewording.
    ///
    /// The diagnostic points at the line's marker. Its anchor span covers both lines, since a
    /// change to either can create the violation.
    ///
    /// # Returns
    ///
    /// The diagnostic.
    ///
    /// # Panics
    ///
    /// Panics if the pair's next line comes before its line, which [`paragraph::joinable_pairs`]
    /// never produces.
    fn diagnostic(self, width: Width, is_wrapped_early: bool) -> Diagnostic {
        let Self {
            joinable_pair: JoinablePair {
                line, next_line, ..
            },
            num_words,
            moved,
        } = self;
        let moved_width = counted(moved.chars().count(), "column");
        let mut message = if num_words == 1 {
            format!(
                "line is {} wide and the next line's first word `{moved}` ({moved_width}) fits \
                 within {}; move it up",
                counted(line.width, "column"),
                width.columns()
            )
        } else {
            format!(
                "line is {} wide and the next line's first {num_words} words `{moved}` \
                 ({moved_width}) fit within {}; move them up",
                counted(line.width, "column"),
                width.columns()
            )
        };
        if is_wrapped_early {
            message.push_str(
                ", but rustfmt wraps this comment one column early (rust-lang/rustfmt#6180), so \
                 reword the paragraph instead",
            );
        }
        let position = Position {
            line: line.line,
            column: line.marker_column,
        };
        let anchor = LineSpan::new(line.line, next_line.line)
            .expect("a joinable pair's next line follows its line");
        Diagnostic::new(RuleId::CommentGreedyWrap.into(), position, message).with_anchor(anchor)
    }
}

/// # Returns
///
/// The byte offset of the end of each whitespace-separated word of `text`, in order.
fn word_ends(text: &str) -> impl Iterator<Item = usize> + '_ {
    text.char_indices()
        .zip(text.chars().skip(1).map(Some).chain([None]))
        .filter(|((_, current), next)| {
            !current.is_whitespace() && next.is_none_or(char::is_whitespace)
        })
        .map(|((index, current), _)| index + current.len_utf8())
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::path::Path;

    use super::*;
    use crate::lexer;
    use crate::syntax::test_code::TestModuleCache;

    fn diagnostics(source_text: &str, columns: usize) -> Vec<Diagnostic> {
        assert_eq!(lexer::lex(source_text).error, None, "{source_text:?}");
        let width = Width::new(NonZeroUsize::new(columns).expect("test widths are non-zero"));
        let test_module_cache = TestModuleCache::default();
        Checker::new(width).check(&FileContext::new(
            Path::new("a.rs"),
            source_text,
            &test_module_cache,
        ))
    }

    #[test]
    fn fit_test_is_exact_at_the_width() {
        // `// aaaa` is 7 columns wide, and ` bbb` brings it to 11.
        let source_text = "// aaaa\n// bbb\n";
        assert_eq!(diagnostics(source_text, 10), []);
        let flagged = diagnostics(source_text, 11);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].position(), Position { line: 1, column: 1 });
        assert_eq!(flagged[0].kind(), RuleId::CommentGreedyWrap.into());
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
    fn move_that_would_start_a_list_item_is_not_flagged() {
        let source_text = "/// The prepared children of [`create_test_prepared_child`] for the \
                           input bytes `0` to\n/// `num_children - 1`, in order.\n";
        assert_eq!(diagnostics(source_text, 100), []);
    }

    #[test]
    fn fewest_words_that_leave_a_valid_continuation_are_named() {
        // `// aaaa` is 7 columns wide. Moving `x` would leave `- y` as a list item, so `x -` (3
        // columns) must move together, which brings the line to 11.
        let source_text = "// aaaa\n// x - y z\n";
        assert_eq!(diagnostics(source_text, 10), []);
        let messages: Vec<_> = diagnostics(source_text, 11)
            .iter()
            .map(|diagnostic| diagnostic.message().to_owned())
            .collect();
        assert_eq!(
            messages,
            [
                "line is 7 columns wide and the next line's first 2 words `x -` (3 columns) fit \
                 within 11; move them up"
            ]
        );
    }

    #[test]
    fn continuation_of_a_lazy_paragraph_must_not_open_any_list_item() {
        let source_text = "// 1. An item\n// lazy aaaa\n// x 2. y\n";
        let messages: Vec<_> = diagnostics(source_text, 100)
            .iter()
            .map(|diagnostic| diagnostic.message().to_owned())
            .collect();
        assert_eq!(
            messages,
            [
                "line is 12 columns wide and the next line's first 2 words `x 2.` (4 columns) fit \
                 within 100; move them up"
            ]
        );
    }

    #[test]
    fn continuation_that_opens_no_interrupting_block_is_flagged() {
        for next_line in [
            "// x 2. y",
            "// x * ",
            "// x <span>",
            "// x [a]: b",
            "// x -1",
            "// x",
        ] {
            let source_text = format!("// aaaa\n{next_line}\n");
            let messages: Vec<_> = diagnostics(&source_text, 100)
                .iter()
                .map(|diagnostic| diagnostic.message().to_owned())
                .collect();
            assert_eq!(
                messages,
                [
                    "line is 7 columns wide and the next line's first word `x` (1 column) fits \
                     within 100; move it up"
                ],
                "{next_line:?}"
            );
        }
    }

    #[test]
    fn continuation_that_opens_an_interrupting_block_needs_more_words_moved() {
        for (next_line, moved) in [
            ("// x - y", "x -"),
            ("// x 1. y", "x 1."),
            ("// x # y", "x #"),
            ("// x > y", "x >"),
            ("// x ```", "x ```"),
            ("// x ***", "x ***"),
            ("// x ---", "x ---"),
            ("// x ==", "x =="),
            ("// x <div>", "x <div>"),
            ("// x [^1]: y", "x [^1]:"),
        ] {
            let source_text = format!("// aaaa\n{next_line}\n");
            let messages: Vec<_> = diagnostics(&source_text, 100)
                .iter()
                .map(|diagnostic| diagnostic.message().to_owned())
                .collect();
            assert_eq!(
                messages,
                [format!(
                    "line is 7 columns wide and the next line's first 2 words `{moved}` ({} \
                     columns) fit within 100; move them up",
                    moved.chars().count()
                )],
                "{next_line:?}"
            );
            assert_eq!(diagnostics(&source_text, 9), [], "{next_line:?}");
        }
    }

    /// # Returns
    ///
    /// Whether each diagnostic of `source_text` at `columns` has the rustfmt hint.
    fn rustfmt_hints(source_text: &str, columns: usize) -> Vec<bool> {
        diagnostics(source_text, columns)
            .iter()
            .map(|diagnostic| diagnostic.message().contains("rust-lang/rustfmt#6180"))
            .collect()
    }

    #[test]
    fn move_that_fills_a_field_or_parameter_comment_to_the_width_gets_the_rustfmt_hint() {
        // Each comment line is 17 columns wide, and ` cc` brings it to 20.
        let sources = [
            "struct S {\n    /// aaaa bbbb\n    /// cc dd\n    a: u8,\n}\n",
            "struct S(\n    /// aaaa bbbb\n    /// cc dd\n    u8,\n);\n",
            "union U {\n    // aaaa bbbbb\n    // cc dd\n    a: u8,\n}\n",
            "enum E {\n    V {\n        /// aaaaa\n        /// cc dd\n        a: u8,\n    },\n}\n",
            "enum E {\n    V(\n        /// aaaaa\n        /// cc dd\n        u8,\n    ),\n}\n",
            "fn f(\n    /// aaaa bbbb\n    /// cc dd\n    a: u8,\n) {}\n",
            "trait T {\n    fn f(\n        // aaaaaa\n        // cc dd\n        a: u8,\n    \
             );\n}\n",
        ];
        for source_text in sources {
            assert_eq!(rustfmt_hints(source_text, 20), [true], "{source_text:?}");
            assert_eq!(rustfmt_hints(source_text, 21), [false], "{source_text:?}");
        }
        let messages: Vec<_> = diagnostics(sources[0], 20)
            .iter()
            .map(|diagnostic| diagnostic.message().to_owned())
            .collect();
        assert_eq!(
            messages,
            [
                "line is 17 columns wide and the next line's first word `cc` (2 columns) fits \
                 within 20; move it up, but rustfmt wraps this comment one column early \
                 (rust-lang/rustfmt#6180), so reword the paragraph instead"
            ]
        );
    }

    #[test]
    fn move_that_fills_another_comment_to_the_width_gets_no_rustfmt_hint() {
        let sources = [
            "enum E {\n    /// aaaa bbbb\n    /// cc dd\n    V { a: u8 },\n}\n",
            "/// aaaa bbbb ccc\n/// cc dd\nstruct S {\n    a: u8,\n}\n",
            "fn f() {\n    // aaaa bbbbb\n    // cc dd\n    let s = S {\n        a: 1,\n    \
             };\n}\n",
            "struct S {\n    /// aaaa bbbb\n    /// cc dd\n    a: u8,\n\n",
        ];
        for source_text in sources {
            assert_eq!(rustfmt_hints(source_text, 20), [false], "{source_text:?}");
        }
    }

    #[test]
    fn slashes_in_literals_and_block_comments_are_not_checked() {
        let source_text = "let a = \"\n// a\n// b\n\";\n/*\n// c\n// d\n*/\n";
        assert_eq!(diagnostics(source_text, 100), []);
    }
}
