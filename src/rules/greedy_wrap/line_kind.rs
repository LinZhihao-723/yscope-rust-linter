//! Classifying the body of one comment line by the markdown role it plays in its paragraph.

use crate::markdown;
use crate::markdown::BlockStart;
use crate::markdown::Fence;
use crate::markdown::HtmlBlockCondition;
use crate::markdown::Preceding;

/// The markdown role of one comment line's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// An empty or whitespace-only body.
    Blank,

    /// A table row, starting with `|`.
    TableRow,

    /// A link reference definition, as [`markdown::link_definition`] recognizes it, or a footnote
    /// definition.
    LinkDef,

    /// The opening fence of a fenced code block.
    Fence { fence: Fence },

    /// An ATX header.
    Header,

    /// A block quote, starting with `>`.
    BlockQuote,

    /// A thematic break, such as `***` or `- - -`.
    ThematicBreak,

    /// A setext heading underline, such as `===` or `---`, right after a prose line.
    SetextUnderline,

    /// The first line of an HTML block, such as `<div>`, that opened under `condition`.
    Html { condition: HtmlBlockCondition },

    /// A list item.
    Bullet {
        /// The number of bytes before the item's text, counted from the start of the body.
        content_indent: usize,
    },

    /// A line starting with a tag such as `NOTE:` or `TODO:`.
    Tagged,

    /// A formula or pseudo-code line such as `deadline = now - 900ms` or `a.out -> b.in`: one
    /// code-like token, then an operator from [`FORMULA_OPERATORS`] between spaces.
    Formula,

    /// Anything else.
    Text,
}

impl LineKind {
    /// # Returns
    ///
    /// The kind of a line that opens `block_start` at `container_indent`, or [`None`] for indented
    /// code, which the paragraph machine tells apart by the line's indent.
    const fn of_block_start(block_start: BlockStart, container_indent: usize) -> Option<Self> {
        Some(match block_start {
            BlockStart::AtxHeading => Self::Header,
            BlockStart::FencedCode { fence } => Self::Fence { fence },
            BlockStart::BlockQuote => Self::BlockQuote,
            BlockStart::ThematicBreak => Self::ThematicBreak,
            BlockStart::SetextUnderline => Self::SetextUnderline,
            BlockStart::BulletItem { content_indent, .. }
            | BlockStart::OrderedItem { content_indent, .. } => Self::Bullet {
                content_indent: container_indent + content_indent,
            },
            BlockStart::HtmlBlock { condition } => Self::Html { condition },
            BlockStart::FootnoteDefinition | BlockStart::LinkReferenceDefinition => Self::LinkDef,
            BlockStart::IndentedCode => return None,
        })
    }
}

/// The text of a comment line after its marker, without the one space that usually follows it.
///
/// # Type Parameters
///
/// * `'text` - The lifetime of the comment line's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Body<'text> {
    text: &'text str,
}

impl<'text> Body<'text> {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The body of a comment line whose text after the marker is `comment_text`.
    pub fn new(comment_text: &'text str) -> Self {
        Self {
            text: comment_text.strip_prefix(' ').unwrap_or(comment_text),
        }
    }

    /// # Returns
    ///
    /// The number of spaces and tabs that start the body.
    pub fn indent(self) -> usize {
        markdown::indent(self.text)
    }

    /// # Returns
    ///
    /// The body's text without leading and trailing whitespace.
    pub fn trimmed(self) -> &'text str {
        self.text.trim()
    }

    /// # Returns
    ///
    /// The body's text, which the [`crate::markdown`] classifiers take.
    pub const fn text(self) -> &'text str {
        self.text
    }

    /// Classifies the body. The rule's own kinds come first: [`LineKind::Blank`],
    /// [`LineKind::TableRow`] and [`LineKind::LinkDef`]. Then [`BlockStart::parse`] recognizes the
    /// Markdown block of the body's text after `container_indent`, by the line that `preceding`
    /// describes. [`LineKind::Tagged`], [`LineKind::Formula`] and [`LineKind::Text`] follow, in
    /// this order, and the first kind that matches wins.
    ///
    /// `container_indent` is the content indent of the open list item the line is in, or 0 outside
    /// list items. For a line that may continue a prose paragraph, it is the paragraph's
    /// continuation indent.
    ///
    /// # Returns
    ///
    /// The body's kind.
    ///
    /// # Panics
    ///
    /// Panics if `container_indent` is deeper than the body's indent.
    pub fn kind(self, container_indent: usize, preceding: Preceding) -> LineKind {
        assert!(
            container_indent <= self.indent(),
            "a line's container indent is at most its indent"
        );
        let trimmed = self.trimmed();
        if trimmed.is_empty() {
            LineKind::Blank
        } else if trimmed.starts_with('|') {
            LineKind::TableRow
        } else if markdown::link_definition(self.text).is_some() {
            LineKind::LinkDef
        } else if let Some(kind) = BlockStart::parse(&self.text[container_indent..], preceding)
            .and_then(|block_start| LineKind::of_block_start(block_start, container_indent))
        {
            kind
        } else if is_tagged(trimmed) {
            LineKind::Tagged
        } else if is_formula(trimmed) {
            LineKind::Formula
        } else {
            LineKind::Text
        }
    }
}

/// The tags that start a [`LineKind::Tagged`] line when followed by `:`.
const TAGS: [&str; 8] = [
    "NOTE",
    "TODO",
    "FIXME",
    "SAFETY",
    "XXX",
    "HACK",
    "WARNING",
    "IMPORTANT",
];

/// The operators that make a line a [`LineKind::Formula`] when they follow its first token.
const FORMULA_OPERATORS: [&str; 12] = [
    "=", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", ":=", "->", "=>",
];

/// # Returns
///
/// Whether `trimmed` starts with one of [`TAGS`] followed by `:`.
fn is_tagged(trimmed: &str) -> bool {
    TAGS.iter().any(|tag| {
        trimmed
            .strip_prefix(tag)
            .is_some_and(|rest| rest.starts_with(':'))
    })
}

/// # Returns
///
/// Whether `trimmed` is a formula: a first token made of identifier characters, `.`, `:`, `[`, `]`,
/// `(` and `)`, then whitespace, then one of [`FORMULA_OPERATORS`], then whitespace.
fn is_formula(trimmed: &str) -> bool {
    let mut words = trimmed.split_whitespace();
    let (Some(first_token), Some(operator), Some(_)) = (words.next(), words.next(), words.next())
    else {
        return false;
    };
    first_token
        .chars()
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && first_token.chars().all(|character| {
            character.is_alphanumeric()
                || matches!(character, '_' | '.' | ':' | '[' | ']' | '(' | ')')
        })
        && FORMULA_OPERATORS.contains(&operator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_are_classified_by_their_markdown_role() {
        let cases = [
            ("", LineKind::Blank),
            ("   ", LineKind::Blank),
            ("``not a fence", LineKind::Text),
            ("# Errors", LineKind::Header),
            ("###### Six", LineKind::Header),
            ("#", LineKind::Header),
            ("####### Seven", LineKind::Text),
            ("#544).", LineKind::Text),
            ("#[derive(Debug)]", LineKind::Text),
            ("| a | b |", LineKind::TableRow),
            ("> quoted", LineKind::BlockQuote),
            ("[`Foo::bar`]:", LineKind::LinkDef),
            ("[label]: https://example.com", LineKind::LinkDef),
            ("   [label]:", LineKind::LinkDef),
            ("     [label]:", LineKind::Text),
            ("[]:", LineKind::Text),
            ("[`Foo`]:bar", LineKind::Text),
            ("     # Indented", LineKind::Text),
            ("     - Indented", LineKind::Text),
            ("***", LineKind::ThematicBreak),
            ("- - -", LineKind::ThematicBreak),
            ("---", LineKind::ThematicBreak),
            ("===", LineKind::Text),
            (
                "<div>",
                LineKind::Html {
                    condition: HtmlBlockCondition::BlockTag,
                },
            ),
            (
                "<span>",
                LineKind::Html {
                    condition: HtmlBlockCondition::CompleteTag,
                },
            ),
            ("[^1]: A footnote.", LineKind::LinkDef),
            ("[^1]:A footnote.", LineKind::LinkDef),
            ("[`Foo`] is a type.", LineKind::Text),
            ("* item", LineKind::Bullet { content_indent: 2 }),
            ("   - item", LineKind::Bullet { content_indent: 4 }),
            ("+   item", LineKind::Bullet { content_indent: 4 }),
            ("12. item", LineKind::Bullet { content_indent: 4 }),
            ("1) item", LineKind::Bullet { content_indent: 3 }),
            ("2. item", LineKind::Bullet { content_indent: 3 }),
            ("-", LineKind::Bullet { content_indent: 2 }),
            ("1.5 seconds", LineKind::Text),
            ("*emphasis*", LineKind::Text),
            ("--flag", LineKind::Text),
            ("NOTE: a note", LineKind::Tagged),
            ("TODO: later", LineKind::Tagged),
            ("NOTES: not a tag", LineKind::Text),
            ("Note: not upper case", LineKind::Text),
            ("<https://example.com>", LineKind::Text),
            ("deadline = now - 900ms", LineKind::Formula),
            ("task_5.output_1 -> task_6.input_0", LineKind::Formula),
            ("a[i] += 1", LineKind::Formula),
            ("x =", LineKind::Text),
            ("total number = a + b", LineKind::Text),
            ("Plain prose.", LineKind::Text),
        ];
        for (text, kind) in cases {
            assert_eq!(
                Body::new(text).kind(0, Preceding::NoParagraph),
                kind,
                "{text:?}"
            );
        }
        for text in ["```rust", "  ~~~"] {
            assert!(
                matches!(
                    Body::new(text).kind(0, Preceding::NoParagraph),
                    LineKind::Fence { .. }
                ),
                "{text:?}"
            );
        }
    }

    #[test]
    fn only_blocks_that_interrupt_a_paragraph_follow_a_prose_line() {
        let cases = [
            ("2. item", LineKind::Text),
            ("1. item", LineKind::Bullet { content_indent: 3 }),
            ("- 1`, in order.", LineKind::Bullet { content_indent: 2 }),
            ("*", LineKind::Text),
            ("-", LineKind::SetextUnderline),
            ("---", LineKind::SetextUnderline),
            ("===", LineKind::SetextUnderline),
            ("- - -", LineKind::ThematicBreak),
            ("<span>", LineKind::Text),
            (
                "<div>",
                LineKind::Html {
                    condition: HtmlBlockCondition::BlockTag,
                },
            ),
            ("# Errors", LineKind::Header),
            ("> quoted", LineKind::BlockQuote),
            ("[label]: https://example.com", LineKind::LinkDef),
            ("| a | b |", LineKind::TableRow),
            ("NOTE: a note", LineKind::Tagged),
            ("deadline = now - 900ms", LineKind::Formula),
            ("Plain prose.", LineKind::Text),
        ];
        for (text, kind) in cases {
            assert_eq!(
                Body::new(text).kind(0, Preceding::Paragraph),
                kind,
                "{text:?}"
            );
        }
        assert_eq!(
            Body::new("   2. item").kind(2, Preceding::Paragraph),
            LineKind::Text
        );
        assert_eq!(
            Body::new("2. item").kind(0, Preceding::LazyParagraph),
            LineKind::Bullet { content_indent: 3 }
        );
        assert_eq!(
            Body::new("===").kind(0, Preceding::LazyParagraph),
            LineKind::Text
        );
    }

    #[test]
    fn block_starts_are_recognized_inside_the_container() {
        assert_eq!(
            Body::new("   - nested").kind(2, Preceding::NoParagraph),
            LineKind::Bullet { content_indent: 4 }
        );
        assert_eq!(
            Body::new("      - nested").kind(2, Preceding::NoParagraph),
            LineKind::Bullet { content_indent: 7 }
        );
        assert_eq!(
            Body::new("       - too deep").kind(2, Preceding::NoParagraph),
            LineKind::Text
        );
        assert_eq!(
            Body::new("       # too deep").kind(2, Preceding::NoParagraph),
            LineKind::Text
        );
        assert_eq!(
            Body::new("     # Header").kind(2, Preceding::NoParagraph),
            LineKind::Header
        );
    }

    #[test]
    fn body_drops_only_one_leading_space() {
        assert_eq!(Body::new("    indented  word").indent(), 3);
        assert_eq!(Body::new("no-space").indent(), 0);
    }
}
