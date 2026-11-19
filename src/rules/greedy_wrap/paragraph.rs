//! Splitting comments into blocks, and deciding which consecutive lines of a block belong to the
//! same paragraph.

use crate::lexer::CommentLine;
use crate::markdown::Fence;
use crate::markdown::HtmlBlockCondition;
use crate::markdown::Preceding;
use crate::rules::greedy_wrap::line_kind::Body;
use crate::rules::greedy_wrap::line_kind::LineKind;

/// Two consecutive comment lines of the same paragraph, so the fit test applies to them.
///
/// # Type Parameters
///
/// * `'comment` - The lifetime of the comment lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinablePair<'comment> {
    pub line: &'comment CommentLine,
    pub next_line: &'comment CommentLine,

    /// How the paragraph precedes `next_line`: [`Preceding::Paragraph`], or
    /// [`Preceding::LazyParagraph`] if the paragraph is a lazy continuation of a list item's
    /// paragraph.
    pub preceding: Preceding,
}

/// Splits comment lines into blocks: maximal runs of own-line comments on consecutive lines with
/// the same marker text and marker column. A trailing comment is a block of its own.
///
/// # Returns
///
/// The blocks in source order.
pub fn blocks(comment_lines: &[CommentLine]) -> impl Iterator<Item = &[CommentLine]> {
    comment_lines.chunk_by(|previous, current| current.continues_block(previous))
}

/// Walks one block and finds the consecutive lines that belong to the same prose paragraph. The
/// lines of a paragraph that a setext heading underline turns into a heading are not joinable.
///
/// # Returns
///
/// The joinable pairs of the block, in source order.
pub fn joinable_pairs(block: &[CommentLine]) -> Vec<JoinablePair<'_>> {
    let Some(first_line) = block.first() else {
        return Vec::new();
    };
    let mut paragraph_machine = ParagraphMachine::new();
    paragraph_machine.step(Body::new(&first_line.text));
    let mut joinable_pairs = Vec::new();
    let mut paragraph_pairs = Vec::new();
    for (line, next_line) in block.iter().zip(block.iter().skip(1)) {
        match paragraph_machine.step(Body::new(&next_line.text)) {
            Step::ContinuesParagraph { preceding } => paragraph_pairs.push(JoinablePair {
                line,
                next_line,
                preceding,
            }),
            Step::UnderlinesHeading => paragraph_pairs.clear(),
            Step::EndsParagraph => joinable_pairs.append(&mut paragraph_pairs),
        }
    }
    joinable_pairs.append(&mut paragraph_pairs);
    joinable_pairs
}

/// What one line does to the paragraph of the line before it.
#[derive(Debug)]
enum Step {
    /// The line continues the previous line's prose paragraph, which precedes it as `preceding`
    /// describes.
    ContinuesParagraph { preceding: Preceding },

    /// The line is a setext heading underline, which turns the previous line's prose paragraph into
    /// a heading.
    UnderlinesHeading,

    /// The line ends the previous line's paragraph, if any, or belongs to no prose paragraph.
    EndsParagraph,
}

/// The paragraph that the latest line of a block belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Paragraph {
    /// Filled text, whose continuation lines are indented by `continuation_indent`.
    Prose {
        continuation_indent: usize,

        /// Whether the paragraph's first line is a lazy continuation line of a list item's
        /// paragraph: a text line, right after it, in an outer container.
        is_lazy: bool,
    },

    /// A link reference definition, whose continuation lines are indented deeper than `def_indent`.
    LinkDef { def_indent: usize },

    /// Indented text outside any list item, such as an aligned table or an indented code block.
    Preformatted,

    /// An HTML block that opened under `condition` and hasn't ended yet.
    Html { condition: HtmlBlockCondition },

    /// A fenced code block that opened with `fence` and hasn't been closed yet.
    FencedCode { fence: Fence },
}

/// The state of the walk over one block.
struct ParagraphMachine {
    paragraph: Option<Paragraph>,

    /// The content indents of the list items that are still open, in increasing order.
    list_indents: Vec<usize>,
}

impl ParagraphMachine {
    const fn new() -> Self {
        Self {
            paragraph: None,
            list_indents: Vec::new(),
        }
    }

    /// Moves the walk to the next line of the block.
    ///
    /// A line indented by the continuation indent of the current prose paragraph is classified
    /// after that paragraph. Any other line is classified where no paragraph precedes it, inside
    /// the deepest open list item whose content indent is at most the line's indent.
    ///
    /// # Returns
    ///
    /// What the line does to the previous line's paragraph.
    fn step(&mut self, body: Body<'_>) -> Step {
        if let Some(Paragraph::FencedCode { fence }) = self.paragraph {
            if fence.is_closed_by(body.text()) {
                self.paragraph = None;
            }
            return Step::EndsParagraph;
        }
        if let Some(Paragraph::Html { condition }) = self.paragraph {
            if condition.is_ended_by(body.text()) {
                self.paragraph = None;
            }
            return Step::EndsParagraph;
        }
        let indent = body.indent();
        let kind = match self.paragraph {
            Some(Paragraph::Prose {
                continuation_indent,
                is_lazy,
            }) if indent == continuation_indent => {
                body.kind(continuation_indent, Preceding::paragraph(is_lazy))
            }
            _ => body.kind(self.container_indent(indent), Preceding::NoParagraph),
        };
        let is_lazy = self.is_lazy_continuation(indent);
        match kind {
            LineKind::Fence { fence } => {
                self.paragraph = Some(Paragraph::FencedCode { fence });
            }
            LineKind::Blank | LineKind::TableRow | LineKind::BlockQuote => {
                self.paragraph = None;
            }
            LineKind::ThematicBreak => {
                self.paragraph = None;
                self.close_list_items_deeper_than(indent);
            }
            LineKind::Html { condition } => {
                self.paragraph =
                    (!condition.is_ended_by(body.text())).then_some(Paragraph::Html { condition });
            }
            LineKind::Formula => {
                self.paragraph = Some(Paragraph::Preformatted);
            }
            LineKind::Header => {
                self.paragraph = None;
                self.list_indents.clear();
            }
            LineKind::SetextUnderline => {
                self.paragraph = None;
                self.list_indents.clear();
                return Step::UnderlinesHeading;
            }
            LineKind::LinkDef => {
                self.paragraph = Some(Paragraph::LinkDef { def_indent: indent });
            }
            LineKind::Bullet { content_indent } => {
                self.close_list_items_deeper_than(indent);
                self.list_indents.push(content_indent);
                self.paragraph = Some(Paragraph::Prose {
                    continuation_indent: content_indent,
                    is_lazy: false,
                });
            }
            LineKind::Tagged => {
                self.paragraph = Some(Paragraph::Prose {
                    continuation_indent: indent,
                    is_lazy,
                });
            }
            LineKind::Text => return self.step_text(indent, is_lazy),
        }
        Step::EndsParagraph
    }

    /// Moves the walk to a [`LineKind::Text`] line indented by `indent`. `is_lazy` tells whether
    /// the line is a lazy continuation line of the current paragraph, if it starts a prose
    /// paragraph.
    ///
    /// The line continues the current paragraph in these cases:
    ///
    /// * The paragraph is prose, and the line has its continuation indent.
    /// * The paragraph is a link reference definition, and the line is indented deeper than it.
    ///
    /// Otherwise, the line starts a new paragraph.
    ///
    /// # Returns
    ///
    /// What the line does to the previous line's paragraph.
    fn step_text(&mut self, indent: usize, is_lazy: bool) -> Step {
        match self.paragraph {
            Some(Paragraph::Prose {
                continuation_indent,
                is_lazy,
            }) if indent == continuation_indent => {
                return Step::ContinuesParagraph {
                    preceding: Preceding::paragraph(is_lazy),
                };
            }
            Some(Paragraph::LinkDef { def_indent }) if indent > def_indent => {
                return Step::EndsParagraph;
            }
            _ => {}
        }
        self.paragraph = Some(if indent == 0 {
            self.list_indents.clear();
            Paragraph::Prose {
                continuation_indent: 0,
                is_lazy,
            }
        } else if self.list_indents.contains(&indent) {
            self.close_list_items_deeper_than(indent);
            Paragraph::Prose {
                continuation_indent: indent,
                is_lazy,
            }
        } else {
            Paragraph::Preformatted
        });
        Step::EndsParagraph
    }

    /// # Returns
    ///
    /// Whether a line indented by `indent` that starts a prose paragraph is a lazy continuation
    /// line of the current paragraph: the current paragraph is prose in a list item that the line's
    /// container is outside of.
    fn is_lazy_continuation(&self, indent: usize) -> bool {
        matches!(
            self.paragraph,
            Some(Paragraph::Prose { continuation_indent, .. })
                if self.container_indent(continuation_indent) > self.container_indent(indent)
        )
    }

    /// # Returns
    ///
    /// The content indent of the deepest open list item that a line indented by `indent` is in, or
    /// 0 if it is in none.
    fn container_indent(&self, indent: usize) -> usize {
        self.list_indents
            .iter()
            .rev()
            .find(|list_indent| **list_indent <= indent)
            .copied()
            .unwrap_or_default()
    }

    /// Closes the open list items whose content indent is deeper than `indent`.
    fn close_list_items_deeper_than(&mut self, indent: usize) {
        while self.list_indents.last().is_some_and(|last| *last > indent) {
            self.list_indents.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer;

    /// # Returns
    ///
    /// The line numbers of the first line of each joinable pair in `source_text`.
    fn joinable_lines(source_text: &str) -> Vec<usize> {
        let lexed = lexer::lex(source_text);
        assert_eq!(lexed.error, None, "{source_text:?}");
        blocks(&lexed.comment_lines)
            .flat_map(joinable_pairs)
            .map(|pair| pair.line.line)
            .collect()
    }

    #[test]
    fn consecutive_prose_lines_are_joinable() {
        assert_eq!(joinable_lines("/// One\n/// two\n/// three.\n"), [1, 2]);
        assert_eq!(joinable_lines("// One\n// two.\n"), [1]);
    }

    #[test]
    fn code_fence_contents_are_not_joinable() {
        let source_text =
            "/// Text\n/// ```\n/// let a = 1;\n/// let b = 2;\n/// ```\n/// More\n/// text.\n";
        assert_eq!(joinable_lines(source_text), [6]);
    }

    #[test]
    fn code_fence_closes_only_on_its_own_marker() {
        let source_text = "/// ```\n/// code\n/// ~~~\n/// not prose\n/// either\n/// ```\n/// \
                           Prose\n/// wraps.\n";
        assert_eq!(joinable_lines(source_text), [7]);
    }

    #[test]
    fn blank_line_ends_the_paragraph() {
        assert_eq!(joinable_lines("/// First.\n///\n/// Second.\n"), []);
        assert_eq!(
            joinable_lines("/// First.\n///\n/// Second\n/// wraps.\n"),
            [3]
        );
    }

    #[test]
    fn header_is_never_joined_with_its_neighbors() {
        assert_eq!(joinable_lines("/// Summary.\n/// # Errors\n"), []);
        assert_eq!(joinable_lines("/// # Returns\n/// A new sender.\n"), []);
        assert_eq!(
            joinable_lines("/// # Returns\n///\n/// A new\n/// sender.\n"),
            [3]
        );
    }

    #[test]
    fn following_bullet_starts_a_new_paragraph() {
        assert_eq!(joinable_lines("/// * First.\n/// * Second.\n"), []);
        assert_eq!(joinable_lines("/// Intro\n/// * First.\n"), []);
    }

    #[test]
    fn bullet_continuation_is_joinable_but_a_zero_indent_line_is_not() {
        assert_eq!(
            joinable_lines("/// * A bullet that ends\n///   early.\n"),
            [1]
        );
        assert_eq!(
            joinable_lines("/// * A bullet.\n/// A new paragraph.\n"),
            []
        );
        assert_eq!(joinable_lines("/// 1. Item\n///    wraps.\n"), [1]);
        assert_eq!(
            joinable_lines("/// * Top\n///   * Nested\n///     wraps.\n"),
            [2]
        );
    }

    #[test]
    fn line_at_another_indent_than_the_paragraph_continuation_is_not_joinable() {
        assert_eq!(joinable_lines("// Text\n//   indented\n"), []);
        assert_eq!(joinable_lines("/// * Item\n///     deeper\n"), []);
        assert_eq!(joinable_lines("/// * Item\n///   wraps\n/// out.\n"), [1]);
    }

    #[test]
    fn table_rows_and_block_quotes_are_not_joinable() {
        assert_eq!(joinable_lines("/// | a | b |\n/// | - | - |\n"), []);
        assert_eq!(joinable_lines("/// | a | b |\n/// Text.\n"), []);
        assert_eq!(joinable_lines("/// > quote\n/// > more\n"), []);
        assert_eq!(joinable_lines("/// a | b |\n/// c | d |\n"), [1]);
    }

    #[test]
    fn change_of_marker_or_marker_column_ends_the_paragraph() {
        assert_eq!(joinable_lines("// A plain\n/// doc.\n"), []);
        assert_eq!(joinable_lines("    // Four\n        // eight.\n"), []);
        assert_eq!(joinable_lines("    // Four\n    // four.\n"), [1]);
    }

    #[test]
    fn trailing_comments_belong_to_no_block() {
        assert_eq!(joinable_lines("let a = 1; // short\n// A comment.\n"), []);
        assert_eq!(joinable_lines("// A comment\nlet a = 1; // short\n"), []);
        assert_eq!(joinable_lines("// short\n// A comment.\n"), [1]);
    }

    #[test]
    fn link_reference_definition_and_its_target_are_not_joinable() {
        let source_text = "/// See [`Foo::bar`].\n/// [`Foo::bar`]:\n///   crate::foo::Foo::bar\n";
        assert_eq!(joinable_lines(source_text), []);
        let source_text =
            "/// [`Foo::bar`]:\n///   crate::foo::Foo::bar\n/// Next\n/// paragraph.\n";
        assert_eq!(joinable_lines(source_text), [3]);
        let in_list_item = "/// * Item.\n/// [`a`]:\n///   https://example.com\n///   \"Title\"\n";
        assert_eq!(joinable_lines(in_list_item), []);
    }

    #[test]
    fn indented_paragraph_is_prose_only_inside_a_list_item() {
        let loose_item = "/// * Item.\n///\n///   Second paragraph\n///   of the item.\n";
        assert_eq!(joinable_lines(loose_item), [3]);
        let preformatted = "// Cases:\n//\n//   index 0: alive\n//   index 1: dead\n";
        assert_eq!(joinable_lines(preformatted), []);
        let after_header = "/// * Item.\n/// # Header\n///\n///   indented\n///   text\n";
        assert_eq!(joinable_lines(after_header), []);
        let after_preformatted = "//   aligned\n// Then\n// prose.\n";
        assert_eq!(joinable_lines(after_preformatted), [2]);
    }

    #[test]
    fn trailing_colon_does_not_end_the_paragraph() {
        assert_eq!(joinable_lines("// There are 3:\n// The first.\n"), [1]);
        assert_eq!(joinable_lines("/// Values:\n/// `A`, `B`.\n"), [1]);
    }

    #[test]
    fn formula_lines_are_never_joined_with_their_neighbors() {
        let formulas = "// soft_timeout_ddl = registered_at + 100ms\n// deadline = now - 900ms\n";
        assert_eq!(joinable_lines(formulas), []);
        let after_colon = "// Validate fan-out case:\n// task_5.output_1 -> task_6.input_0\n";
        assert_eq!(joinable_lines(after_colon), []);
        assert_eq!(joinable_lines("// x = 1\n// Then\n// prose.\n"), [2]);
        assert_eq!(joinable_lines("// total number = a +\n// b\n"), [1]);
    }

    #[test]
    fn tagged_line_starts_a_new_paragraph() {
        assert_eq!(joinable_lines("// Write outputs\n// NOTE: one type.\n"), []);
        assert_eq!(joinable_lines("// NOTE: a note\n// that wraps.\n"), [1]);
    }

    #[test]
    fn line_that_cannot_interrupt_a_paragraph_continues_it() {
        assert_eq!(joinable_lines("/// In\n/// 2022. Then\n"), [1]);
        assert_eq!(joinable_lines("/// One\n/// *\n"), [1]);
        assert_eq!(joinable_lines("/// One\n/// <span>\n"), [1]);
        assert_eq!(joinable_lines("/// One\n/// 1. Item\n"), []);
        assert_eq!(joinable_lines("/// One\n/// - Item\n"), []);
    }

    #[test]
    fn setext_underline_turns_its_paragraph_into_a_heading() {
        let heading = "/// A heading\n/// that wraps\n/// ===\n/// Text\n/// wraps.\n";
        assert_eq!(joinable_lines(heading), [4]);
        assert_eq!(joinable_lines("/// A heading\n/// ---\n"), []);
        assert_eq!(joinable_lines("/// ---\n/// Text\n/// wraps.\n"), [2]);
        let in_list_item =
            "/// * Item\n///\n///   Heading\n///   ===\n///\n///   indented\n///   text\n";
        assert_eq!(joinable_lines(in_list_item), []);
    }

    #[test]
    fn thematic_break_ends_the_paragraph() {
        assert_eq!(
            joinable_lines("/// Text\n/// * * *\n/// More\n/// text.\n"),
            [3]
        );
        assert_eq!(joinable_lines("/// Text\n/// ***\n"), []);
        let after_list_item = "/// * Item\n/// ***\n///   indented\n///   text\n";
        assert_eq!(joinable_lines(after_list_item), []);
    }

    #[test]
    fn html_block_lines_are_not_joinable_until_the_block_ends() {
        let block_tag = "/// <div>\n/// Raw\n/// html.\n///\n/// Text\n/// wraps.\n";
        assert_eq!(joinable_lines(block_tag), [5]);
        let raw_text = "/// <pre>\n/// Raw\n///\n/// text.\n/// </pre>\n/// Text\n/// wraps.\n";
        assert_eq!(joinable_lines(raw_text), [6]);
        let one_line_comment = "/// <!-- comment -->\n/// Text\n/// wraps.\n";
        assert_eq!(joinable_lines(one_line_comment), [2]);
    }

    #[test]
    fn any_list_item_ends_a_lazy_continuation_of_a_list_item() {
        let lazy = "/// 1. An item\n/// lazy line\n/// 2. Next\n/// item.\n";
        assert_eq!(joinable_lines(lazy), []);
        let not_lazy = "/// 1. An item\n///\n/// Paragraph\n/// 2. Not an item.\n";
        assert_eq!(joinable_lines(not_lazy), [3]);
        let lazy_tagged = "/// 1. An item\n/// NOTE: lazy\n/// 2. Next\n/// item.\n";
        assert_eq!(joinable_lines(lazy_tagged), []);
    }

    #[test]
    fn block_starts_are_recognized_relative_to_the_open_list_item() {
        let nested = "/// * Top\n///      * Nested\n///        wraps.\n";
        assert_eq!(joinable_lines(nested), [2]);
        let too_deep = "/// * Top\n///       * Nested\n///         wraps.\n";
        assert_eq!(joinable_lines(too_deep), []);
        let indented_code = "// Intro:\n//\n//     - not a list\n//       item.\n";
        assert_eq!(joinable_lines(indented_code), []);
    }

    #[test]
    fn fence_state_resets_at_a_block_boundary() {
        let source_text = "/// ```\n/// unclosed\nfn f() {}\n/// Text\n/// wraps.\n";
        assert_eq!(joinable_lines(source_text), [4]);
    }
}
