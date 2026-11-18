//! Classifying the body of one comment line by the markdown role it plays in its paragraph.

use crate::markdown;
use crate::markdown::Fence;

/// The markdown role of one comment line's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// An empty or whitespace-only body.
    Blank,

    /// A code fence: three or more backticks or tildes, as [`Fence::parse`] recognizes it.
    Fence,

    /// An ATX header, as [`markdown::atx_header`] recognizes it.
    Header,

    /// A table row, starting with `|`.
    TableRow,

    /// A block quote, starting with `>`.
    BlockQuote,

    /// A link reference definition, as [`markdown::link_definition`] recognizes it.
    LinkDef,

    /// A list item, as [`markdown::bullet_marker`] recognizes it.
    Bullet {
        /// The column, counted from the start of the body, where the item's text starts.
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
    /// The body's text up to the first whitespace after its leading whitespace.
    pub fn first_word(self) -> &'text str {
        self.trimmed().split_whitespace().next().unwrap_or_default()
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

    /// Classifies the body. The kinds are tried in the order [`LineKind`] lists them, and the first
    /// one that matches wins.
    ///
    /// # Returns
    ///
    /// The body's kind.
    pub fn kind(self) -> LineKind {
        let trimmed = self.trimmed();
        if trimmed.is_empty() {
            LineKind::Blank
        } else if Fence::parse(self.text).is_some() {
            LineKind::Fence
        } else if markdown::atx_header(self.text).is_some() {
            LineKind::Header
        } else if trimmed.starts_with('|') {
            LineKind::TableRow
        } else if trimmed.starts_with('>') {
            LineKind::BlockQuote
        } else if markdown::link_definition(self.text).is_some() {
            LineKind::LinkDef
        } else if let Some(bullet) = markdown::bullet_marker(self.text) {
            LineKind::Bullet {
                content_indent: bullet.content_indent,
            }
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
            ("```rust", LineKind::Fence),
            ("  ~~~", LineKind::Fence),
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
            ("[`Foo`] is a type.", LineKind::Text),
            ("* item", LineKind::Bullet { content_indent: 2 }),
            ("   - item", LineKind::Bullet { content_indent: 4 }),
            ("+   item", LineKind::Bullet { content_indent: 4 }),
            ("12. item", LineKind::Bullet { content_indent: 4 }),
            ("1) item", LineKind::Bullet { content_indent: 3 }),
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
            assert_eq!(Body::new(text).kind(), kind, "{text:?}");
        }
    }

    #[test]
    fn body_drops_only_one_leading_space() {
        let body = Body::new("    indented  word");
        assert_eq!(body.indent(), 3);
        assert_eq!(body.first_word(), "indented");
        assert_eq!(Body::new("no-space").indent(), 0);
        assert_eq!(Body::new("").first_word(), "");
    }
}
