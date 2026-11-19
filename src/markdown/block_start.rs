//! The Markdown block that the start of a line opens, as CommonMark 0.31.2 and the GFM extensions
//! that rustdoc enables define it.
//!
//! The functions take a line's text relative to the container the line is in: after its comment
//! marker and the one space that usually follows it, and after the content indent of the list item
//! it belongs to.

use crate::markdown::line;
use crate::markdown::line::Fence;
use crate::markdown::line::MAX_BLOCK_INDENT;
use crate::markdown::line::MAX_HEADER_LEVEL;
use crate::markdown::line::MAX_ORDERED_LIST_DIGITS;

/// The Markdown block that a line opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockStart {
    /// An ATX heading: 1 to 6 `#`, then a space, a tab or the end of the line.
    AtxHeading,

    /// The opening fence of a fenced code block, as [`Fence::parse`] recognizes it.
    FencedCode { fence: Fence },

    /// A block quote: `>`.
    BlockQuote,

    /// A thematic break: 3 or more of the same `*`, `-` or `_`, with nothing but spaces and tabs
    /// between and after them.
    ThematicBreak,

    /// A setext heading underline: a run of `=` or `-`, then nothing but spaces and tabs. It turns
    /// the paragraph above it into a heading, so it exists only right after a paragraph line.
    SetextUnderline,

    /// A bullet list item: `-`, `+` or `*`, then a space, a tab or the end of the line.
    BulletItem {
        /// The number of bytes before the item's text, counted from the start of the line.
        content_indent: usize,

        /// Whether nothing but spaces and tabs follows the marker.
        is_empty: bool,
    },

    /// An ordered list item: 1 to 9 digits, then `.` or `)`, then a space, a tab or the end of the
    /// line.
    OrderedItem {
        /// The number the item starts its list with.
        start: u32,

        /// The number of bytes before the item's text, counted from the start of the line.
        content_indent: usize,

        /// Whether nothing but spaces and tabs follows the marker.
        is_empty: bool,
    },

    /// An HTML block, by the start condition the line meets.
    HtmlBlock { condition: HtmlBlockCondition },

    /// A footnote definition: `[^label]:`.
    FootnoteDefinition,

    /// The first line of a link reference definition, as [`line::link_definition`] recognizes it.
    LinkReferenceDefinition,

    /// An indented code block: an indent of 4 columns or more.
    IndentedCode,
}

impl BlockStart {
    /// Classifies a line by the line that `preceding` describes.
    ///
    /// # Returns
    ///
    /// The block that `line` opens, which ends any paragraph before it, or [`None`] if `line` is
    /// blank, continues the paragraph before it, or starts a paragraph.
    pub fn parse(line: &str, preceding: Preceding) -> Option<Self> {
        match preceding {
            Preceding::NoParagraph => Self::parse_opened_block(line),
            Preceding::Paragraph => {
                let (rest_column, rest) = split_indent(line);
                if rest_column.index <= MAX_BLOCK_INDENT && is_setext_underline(rest) {
                    return Some(Self::SetextUnderline);
                }
                Self::parse_opened_block(line)
                    .filter(|block_start| block_start.can_interrupt_paragraph())
            }
            Preceding::LazyParagraph => Self::parse_opened_block(line)
                .filter(|block_start| block_start.can_end_lazy_paragraph()),
        }
    }

    /// # Returns
    ///
    /// Whether the block can start on the line right after a paragraph line, which ends the
    /// paragraph. Otherwise, that line continues the paragraph:
    ///
    /// * An empty list item can't interrupt a paragraph.
    /// * An ordered list item can interrupt one only if it starts at 1.
    /// * An HTML block can interrupt one only under start conditions 1 to 6.
    /// * Neither a link reference definition nor indented code can interrupt one.
    pub const fn can_interrupt_paragraph(self) -> bool {
        match self {
            Self::AtxHeading
            | Self::FencedCode { .. }
            | Self::BlockQuote
            | Self::ThematicBreak
            | Self::SetextUnderline
            | Self::FootnoteDefinition => true,
            Self::BulletItem { is_empty, .. } => !is_empty,
            Self::OrderedItem {
                start, is_empty, ..
            } => start == 1 && !is_empty,
            Self::HtmlBlock { condition } => condition.can_interrupt_paragraph(),
            Self::LinkReferenceDefinition | Self::IndentedCode => false,
        }
    }

    /// Classifies a line where no paragraph precedes it.
    ///
    /// # Returns
    ///
    /// The block that `line` opens, or [`None`] if `line` is blank or starts a paragraph.
    fn parse_opened_block(line: &str) -> Option<Self> {
        let (rest_column, rest) = split_indent(line);
        if rest.is_empty() {
            return None;
        }
        if rest_column.index > MAX_BLOCK_INDENT {
            return Some(Self::IndentedCode);
        }
        let indent = line.len() - rest.len();
        if rest.starts_with('>') {
            Some(Self::BlockQuote)
        } else if is_atx_heading(rest) {
            Some(Self::AtxHeading)
        } else if let Some(fence) = Fence::parse(rest) {
            Some(Self::FencedCode { fence })
        } else if let Some(condition) = HtmlBlockCondition::parse(rest) {
            Some(Self::HtmlBlock { condition })
        } else if is_thematic_break(rest) {
            Some(Self::ThematicBreak)
        } else if let Some(list_item) = list_item(rest, indent, rest_column) {
            Some(list_item)
        } else if is_footnote_definition(rest) {
            Some(Self::FootnoteDefinition)
        } else if line::link_definition(rest).is_some() {
            Some(Self::LinkReferenceDefinition)
        } else {
            None
        }
    }

    /// A lazy continuation line is in an outer container of the paragraph it continues, so any list
    /// item opens there, whatever its number or content. A setext heading underline can't be a lazy
    /// continuation line, so it isn't one there.
    ///
    /// # Returns
    ///
    /// Whether the block can start on a line that would otherwise be a lazy continuation line of a
    /// paragraph, which ends the paragraph. Neither a link reference definition, indented code nor
    /// an HTML block under start condition 7 can.
    const fn can_end_lazy_paragraph(self) -> bool {
        match self {
            Self::AtxHeading
            | Self::FencedCode { .. }
            | Self::BlockQuote
            | Self::ThematicBreak
            | Self::SetextUnderline
            | Self::BulletItem { .. }
            | Self::OrderedItem { .. }
            | Self::FootnoteDefinition => true,
            Self::HtmlBlock { condition } => condition.can_interrupt_paragraph(),
            Self::LinkReferenceDefinition | Self::IndentedCode => false,
        }
    }
}

/// The line before a line, as far as it decides which block the line can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preceding {
    /// No paragraph line: the line is the first of its container, or follows a blank line or a
    /// block other than a paragraph.
    NoParagraph,

    /// A paragraph line of the same container. The line continues the paragraph unless it opens a
    /// block that [can interrupt a paragraph](BlockStart::can_interrupt_paragraph).
    Paragraph,

    /// A paragraph line of a list item that the line isn't indented enough to belong to. The line
    /// is a lazy continuation line of the paragraph unless it opens a block of its own container.
    LazyParagraph,
}

impl Preceding {
    /// # Returns
    ///
    /// How a prose paragraph precedes its next line: lazily if `is_lazy`.
    pub const fn paragraph(is_lazy: bool) -> Self {
        if is_lazy {
            Self::LazyParagraph
        } else {
            Self::Paragraph
        }
    }
}

/// The start conditions of an HTML block, numbered 1 to 7 in CommonMark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlBlockCondition {
    /// Condition 1: `<pre`, `<script`, `<style` or `<textarea`, then whitespace, `>` or the end of
    /// the line.
    RawText,

    /// Condition 2: `<!--`.
    Comment,

    /// Condition 3: `<?`.
    ProcessingInstruction,

    /// Condition 4: `<!` and an ASCII letter.
    Declaration,

    /// Condition 5: `<![CDATA[`.
    Cdata,

    /// Condition 6: `<` or `</` and one of [`BLOCK_TAGS`], then whitespace, `>`, `/>` or the end of
    /// the line.
    BlockTag,

    /// Condition 7: a complete open or closing tag other than a [`RAW_TEXT_TAGS`] one, then nothing
    /// but spaces and tabs.
    CompleteTag,
}

impl HtmlBlockCondition {
    /// Applies the end condition of the block to `line`, which may be the block's first line:
    ///
    /// * [`Self::RawText`]: the line holds `</pre>`, `</script>`, `</style>` or `</textarea>`,
    ///   ignoring ASCII case.
    /// * [`Self::Comment`]: the line holds `-->`.
    /// * [`Self::ProcessingInstruction`]: the line holds `?>`.
    /// * [`Self::Declaration`]: the line holds `>`.
    /// * [`Self::Cdata`]: the line holds `]]>`.
    /// * [`Self::BlockTag`] and [`Self::CompleteTag`]: the line is blank, and isn't part of the
    ///   block.
    ///
    /// # Returns
    ///
    /// Whether the block ends at `line`.
    pub fn is_ended_by(self, line: &str) -> bool {
        match self {
            Self::RawText => {
                let lowercase = line.to_ascii_lowercase();
                RAW_TEXT_TAGS
                    .iter()
                    .any(|tag| lowercase.contains(&format!("</{tag}>")))
            }
            Self::Comment => line.contains("-->"),
            Self::ProcessingInstruction => line.contains("?>"),
            Self::Declaration => line.contains('>'),
            Self::Cdata => line.contains("]]>"),
            Self::BlockTag | Self::CompleteTag => is_blank(line),
        }
    }

    /// # Returns
    ///
    /// Whether a block under this condition can start on the line right after a paragraph line:
    /// under every condition but [`Self::CompleteTag`].
    const fn can_interrupt_paragraph(self) -> bool {
        match self {
            Self::RawText
            | Self::Comment
            | Self::ProcessingInstruction
            | Self::Declaration
            | Self::Cdata
            | Self::BlockTag => true,
            Self::CompleteTag => false,
        }
    }

    /// Matches the start conditions against `rest`, a line without its indent, in their order.
    ///
    /// # Returns
    ///
    /// The first condition that `rest` meets, or [`None`] if it starts no HTML block.
    fn parse(rest: &str) -> Option<Self> {
        let after_bracket = rest.strip_prefix('<')?;
        let is_raw_text = RAW_TEXT_TAGS.iter().any(|tag| {
            after_bracket
                .get(..tag.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(tag))
                && after_bracket[tag.len()..]
                    .chars()
                    .next()
                    .is_none_or(|next| matches!(next, ' ' | '\t' | '>'))
        });
        if is_raw_text {
            return Some(Self::RawText);
        }
        if after_bracket.starts_with("!--") {
            return Some(Self::Comment);
        }
        if after_bracket.starts_with('?') {
            return Some(Self::ProcessingInstruction);
        }
        if after_bracket.starts_with("![CDATA[") {
            return Some(Self::Cdata);
        }
        if after_bracket.strip_prefix('!').is_some_and(|declaration| {
            declaration.starts_with(|first: char| first.is_ascii_alphabetic())
        }) {
            return Some(Self::Declaration);
        }
        let name_and_rest = after_bracket.strip_prefix('/').unwrap_or(after_bracket);
        let name_end = name_and_rest
            .find(|current: char| !current.is_ascii_alphanumeric())
            .unwrap_or(name_and_rest.len());
        let after_name = &name_and_rest[name_end..];
        if BLOCK_TAGS
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(&name_and_rest[..name_end]))
            && (after_name.is_empty()
                || after_name.starts_with([' ', '\t', '>'])
                || after_name.starts_with("/>"))
        {
            return Some(Self::BlockTag);
        }
        complete_tag_length(rest.as_bytes())
            .is_some_and(|length| {
                rest[length..]
                    .chars()
                    .all(|current| matches!(current, ' ' | '\t'))
            })
            .then_some(Self::CompleteTag)
    }
}

/// The tags of [`HtmlBlockCondition::RawText`], whose blocks hold raw text.
const RAW_TEXT_TAGS: [&str; 4] = ["pre", "script", "style", "textarea"];

/// The tags of [`HtmlBlockCondition::BlockTag`].
const BLOCK_TAGS: [&str; 62] = [
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

/// The fewest `*`, `-` or `_` that make a thematic break.
const MIN_THEMATIC_BREAK_LENGTH: usize = 3;

/// The most columns of whitespace after a list item's marker that still belong to the marker. With
/// more, the item's text is indented code, and its content starts one column after the marker.
const MAX_LIST_MARKER_SPACING: usize = 4;

/// The tab stop that a tab advances the column to a multiple of.
const TAB_STOP: usize = 4;

/// A column of a line, counted from 0 at the start of the line. A tab advances to the next multiple
/// of [`TAB_STOP`], and any other character takes one column.
#[derive(Debug, Clone, Copy)]
struct Column {
    index: usize,
}

impl Column {
    /// The column that a line starts at.
    const START: Self = Self { index: 0 };

    /// # Returns
    ///
    /// The column right after `text` when `text` starts at this column.
    fn after(self, text: &str) -> Self {
        let index = text.chars().fold(self.index, |index, current| {
            if current == '\t' {
                index + TAB_STOP - index % TAB_STOP
            } else {
                index + 1
            }
        });
        Self { index }
    }

    /// # Returns
    ///
    /// The number of columns that `text` spans when it starts at this column.
    fn width_of(self, text: &str) -> usize {
        self.after(text).index - self.index
    }
}

/// # Returns
///
/// A tuple containing:
///
/// * The column where `line`'s leading spaces and tabs end.
/// * The rest of `line`.
fn split_indent(line: &str) -> (Column, &str) {
    let rest = line.trim_start_matches([' ', '\t']);
    (Column::START.after(&line[..line.len() - rest.len()]), rest)
}

/// # Returns
///
/// Whether `text` is empty or holds only spaces and tabs.
fn is_blank(text: &str) -> bool {
    text.chars().all(|current| matches!(current, ' ' | '\t'))
}

/// # Returns
///
/// Whether `rest`, a line without its indent, starts an ATX heading.
fn is_atx_heading(rest: &str) -> bool {
    let level = rest.chars().take_while(|current| *current == '#').count();
    (1..=MAX_HEADER_LEVEL).contains(&level)
        && (rest[level..].is_empty() || rest[level..].starts_with([' ', '\t']))
}

/// # Returns
///
/// Whether `rest`, a line without its indent, is a thematic break.
fn is_thematic_break(rest: &str) -> bool {
    let Some(marker) = rest
        .chars()
        .next()
        .filter(|first| matches!(first, '*' | '-' | '_'))
    else {
        return false;
    };
    rest.chars()
        .all(|current| current == marker || matches!(current, ' ' | '\t'))
        && rest.chars().filter(|current| *current == marker).count() >= MIN_THEMATIC_BREAK_LENGTH
}

/// # Returns
///
/// Whether `rest`, a line without its indent, is a setext heading underline.
fn is_setext_underline(rest: &str) -> bool {
    let Some(marker) = rest
        .chars()
        .next()
        .filter(|first| matches!(first, '=' | '-'))
    else {
        return false;
    };
    is_blank(rest.trim_start_matches(marker))
}

/// Recognizes a list item in `rest`, the part of a line after its `indent` leading bytes, which
/// starts at `rest_column`.
///
/// # Returns
///
/// The [`BlockStart::BulletItem`] or [`BlockStart::OrderedItem`] that `rest` starts, or [`None`] if
/// it starts no list item.
///
/// # Panics
///
/// Panics if the ordered list marker's digits don't parse as a `u32`, which
/// [`MAX_ORDERED_LIST_DIGITS`] rules out.
fn list_item(rest: &str, indent: usize, rest_column: Column) -> Option<BlockStart> {
    const _: () = assert!(
        MAX_ORDERED_LIST_DIGITS <= u32::MAX.ilog10() as usize,
        "every ordered list number of at most `MAX_ORDERED_LIST_DIGITS` digits fits in a `u32`"
    );
    let num_digits = rest.chars().take_while(char::is_ascii_digit).count();
    let (marker_length, start) = if rest.starts_with(['-', '+', '*']) {
        (1, None)
    } else if (1..=MAX_ORDERED_LIST_DIGITS).contains(&num_digits)
        && rest[num_digits..].starts_with(['.', ')'])
    {
        let start = rest[..num_digits]
            .parse()
            .expect("at most `MAX_ORDERED_LIST_DIGITS` decimal digits fit in a `u32`");
        (num_digits + 1, Some(start))
    } else {
        return None;
    };
    let after_marker = &rest[marker_length..];
    let content = after_marker.trim_start_matches([' ', '\t']);
    let spacing = &after_marker[..after_marker.len() - content.len()];
    if spacing.is_empty() && !content.is_empty() {
        return None;
    }
    let is_empty = content.is_empty();
    let spacing_column = rest_column.after(&rest[..marker_length]);
    let spacing_length = if is_empty || spacing_column.width_of(spacing) > MAX_LIST_MARKER_SPACING {
        1
    } else {
        spacing.len()
    };
    let content_indent = indent + marker_length + spacing_length;
    Some(start.map_or(
        BlockStart::BulletItem {
            content_indent,
            is_empty,
        },
        |start| BlockStart::OrderedItem {
            start,
            content_indent,
            is_empty,
        },
    ))
}

/// # Returns
///
/// Whether `rest`, a line without its indent, starts a footnote definition: `[^`, a non-blank label
/// without brackets, then `]:`.
fn is_footnote_definition(rest: &str) -> bool {
    rest.strip_prefix("[^")
        .and_then(|after_caret| after_caret.split_once(']'))
        .is_some_and(|(label, after_label)| {
            !label.trim().is_empty() && !label.contains('[') && after_label.starts_with(':')
        })
}

/// Scans a complete open tag, such as `<a href="x">` or `<br/>`, or a complete closing tag, such as
/// `</a>`, at the start of `bytes`. The tag name must not be one of [`RAW_TEXT_TAGS`].
///
/// # Returns
///
/// The length of the tag in bytes, or [`None`] if `bytes` doesn't start with one.
fn complete_tag_length(bytes: &[u8]) -> Option<usize> {
    let is_closing = bytes.get(1) == Some(&b'/');
    let name_start = if is_closing { 2 } else { 1 };
    if !bytes.get(name_start)?.is_ascii_alphabetic() {
        return None;
    }
    let name_end = scan_while(bytes, name_start + 1, |current| {
        current.is_ascii_alphanumeric() || current == b'-'
    });
    let name = &bytes[name_start..name_end];
    if RAW_TEXT_TAGS
        .iter()
        .any(|tag| tag.as_bytes().eq_ignore_ascii_case(name))
    {
        return None;
    }
    if is_closing {
        let index = skip_spaces_and_tabs(bytes, name_end);
        return (bytes.get(index) == Some(&b'>')).then_some(index + 1);
    }
    let mut index = name_end;
    loop {
        let after_whitespace = skip_spaces_and_tabs(bytes, index);
        match bytes.get(after_whitespace) {
            Some(b'>') => return Some(after_whitespace + 1),
            Some(b'/') => {
                return (bytes.get(after_whitespace + 1) == Some(&b'>'))
                    .then_some(after_whitespace + 2);
            }
            _ => {}
        }
        if after_whitespace == index {
            return None;
        }
        index = attribute_end(bytes, after_whitespace)?;
    }
}

/// Scans one HTML attribute that starts at `start`: a name, then optionally `=` and a value, with
/// optional spaces and tabs around the `=`.
///
/// # Returns
///
/// The index right after the attribute, or [`None`] if no valid attribute starts at `start`.
fn attribute_end(bytes: &[u8], start: usize) -> Option<usize> {
    let first = *bytes.get(start)?;
    if !(first.is_ascii_alphabetic() || matches!(first, b'_' | b':')) {
        return None;
    }
    let name_end = scan_while(bytes, start + 1, |current| {
        current.is_ascii_alphanumeric() || matches!(current, b'_' | b'.' | b':' | b'-')
    });
    let equals_sign = skip_spaces_and_tabs(bytes, name_end);
    if bytes.get(equals_sign) != Some(&b'=') {
        return Some(name_end);
    }
    let value_start = skip_spaces_and_tabs(bytes, equals_sign + 1);
    let first_value_byte = *bytes.get(value_start)?;
    if matches!(first_value_byte, b'"' | b'\'') {
        let value_length = bytes[value_start + 1..]
            .iter()
            .position(|current| *current == first_value_byte)?;
        return Some(value_start + value_length + 2);
    }
    let value_end = scan_while(bytes, value_start, |current| {
        !matches!(
            current,
            b' ' | b'\t' | b'"' | b'\'' | b'=' | b'<' | b'>' | b'`'
        )
    });
    (value_end > value_start).then_some(value_end)
}

/// # Returns
///
/// The index of the first byte at or after `start` that isn't a space or a tab.
fn skip_spaces_and_tabs(bytes: &[u8], start: usize) -> usize {
    scan_while(bytes, start, |current| matches!(current, b' ' | b'\t'))
}

/// # Returns
///
/// The index of the first byte at or after `start` that doesn't satisfy `predicate`.
fn scan_while(bytes: &[u8], start: usize, mut predicate: impl FnMut(u8) -> bool) -> usize {
    bytes
        .get(start..)
        .and_then(|tail| tail.iter().position(|current| !predicate(*current)))
        .map_or(bytes.len(), |offset| start + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// # Returns
    ///
    /// The block that `line` opens where no paragraph precedes it, and right after a paragraph line
    /// of the same container.
    fn both_contexts(line: &str) -> (Option<BlockStart>, Option<BlockStart>) {
        (
            BlockStart::parse(line, Preceding::NoParagraph),
            BlockStart::parse(line, Preceding::Paragraph),
        )
    }

    fn html(condition: HtmlBlockCondition) -> BlockStart {
        BlockStart::HtmlBlock { condition }
    }

    fn bullet(content_indent: usize, is_empty: bool) -> BlockStart {
        BlockStart::BulletItem {
            content_indent,
            is_empty,
        }
    }

    fn ordered(start: u32, content_indent: usize, is_empty: bool) -> BlockStart {
        BlockStart::OrderedItem {
            start,
            content_indent,
            is_empty,
        }
    }

    #[test]
    fn atx_headings_need_one_to_six_hashes_then_whitespace_or_the_end() {
        for line in [
            "# Errors",
            "###### Six",
            "#",
            "#\tTab",
            "   ## Three spaces",
            "# C#",
        ] {
            assert_eq!(
                both_contexts(line),
                (Some(BlockStart::AtxHeading), Some(BlockStart::AtxHeading)),
                "{line:?}"
            );
        }
        for line in [
            "#hashtag",
            "####### Seven",
            "#544).",
            "#[derive(Debug)]",
            "\\# escaped",
        ] {
            assert_eq!(both_contexts(line), (None, None), "{line:?}");
        }
        assert_eq!(
            both_contexts("    # Four spaces"),
            (Some(BlockStart::IndentedCode), None)
        );
        assert_eq!(
            both_contexts("\t# Tab"),
            (Some(BlockStart::IndentedCode), None)
        );
    }

    #[test]
    fn fences_need_three_backticks_or_tildes_and_no_backtick_in_a_backtick_info_string() {
        let backtick_fence =
            BlockStart::parse("```rust", Preceding::NoParagraph).expect("a backtick fence");
        assert!(matches!(backtick_fence, BlockStart::FencedCode { .. }));
        for line in ["```", "~~~", "   ````text", "~~~ a`b`"] {
            assert!(
                matches!(
                    both_contexts(line),
                    (
                        Some(BlockStart::FencedCode { .. }),
                        Some(BlockStart::FencedCode { .. })
                    )
                ),
                "{line:?}"
            );
        }
        for line in ["``", "~~", "```a``` is code", "~~strikethrough~~"] {
            assert_eq!(both_contexts(line), (None, None), "{line:?}");
        }
        assert_eq!(
            both_contexts("    ```"),
            (Some(BlockStart::IndentedCode), None)
        );
    }

    #[test]
    fn block_quotes_start_with_a_greater_than_sign() {
        for line in [">", "> quoted", ">quoted", "   > three spaces"] {
            assert_eq!(
                both_contexts(line),
                (Some(BlockStart::BlockQuote), Some(BlockStart::BlockQuote)),
                "{line:?}"
            );
        }
        assert_eq!(both_contexts("a > b"), (None, None));
        assert_eq!(
            both_contexts("    > four spaces"),
            (Some(BlockStart::IndentedCode), None)
        );
    }

    #[test]
    fn thematic_breaks_need_three_of_one_character_and_nothing_else() {
        for line in [
            "***",
            "___",
            "* * *",
            "- - -",
            " -  -  - ",
            "_\t_\t_",
            "**********",
        ] {
            assert_eq!(
                both_contexts(line),
                (
                    Some(BlockStart::ThematicBreak),
                    Some(BlockStart::ThematicBreak)
                ),
                "{line:?}"
            );
        }
        for line in ["**", "__", "*-*", "***a", "_ _ x"] {
            assert_eq!(both_contexts(line), (None, None), "{line:?}");
        }
        assert_eq!(
            BlockStart::parse("---", Preceding::NoParagraph),
            Some(BlockStart::ThematicBreak)
        );
        assert_eq!(BlockStart::parse("--", Preceding::NoParagraph), None);
    }

    #[test]
    fn setext_underlines_exist_only_right_after_a_paragraph_line() {
        for line in ["=", "===", "---", "--", "-", "   ==  ", "--\t"] {
            assert_eq!(
                BlockStart::parse(line, Preceding::Paragraph),
                Some(BlockStart::SetextUnderline),
                "{line:?}"
            );
        }
        assert_eq!(BlockStart::parse("===", Preceding::NoParagraph), None);
        assert_eq!(
            BlockStart::parse("-", Preceding::NoParagraph),
            Some(bullet(2, true))
        );
        for line in ["= = =", "=== x", "-- a", "==a", "    ==="] {
            assert_eq!(
                BlockStart::parse(line, Preceding::Paragraph),
                None,
                "{line:?}"
            );
        }
    }

    #[test]
    fn bullets_need_a_space_a_tab_or_the_end_of_the_line_after_the_marker() {
        assert_eq!(
            both_contexts("- item"),
            (Some(bullet(2, false)), Some(bullet(2, false)))
        );
        assert_eq!(
            both_contexts("+   item"),
            (Some(bullet(4, false)), Some(bullet(4, false)))
        );
        assert_eq!(
            both_contexts("*\titem"),
            (Some(bullet(2, false)), Some(bullet(2, false)))
        );
        assert_eq!(
            both_contexts("  * item"),
            (Some(bullet(4, false)), Some(bullet(4, false)))
        );
        assert_eq!(
            both_contexts("- [ ] task"),
            (Some(bullet(2, false)), Some(bullet(2, false)))
        );
        assert_eq!(
            both_contexts("- 1`, in order."),
            (Some(bullet(2, false)), Some(bullet(2, false)))
        );
        assert_eq!(both_contexts("-1 is negative"), (None, None));
        for line in ["--flag", "*emphasis*", "+x"] {
            assert_eq!(both_contexts(line), (None, None), "{line:?}");
        }
        assert_eq!(
            both_contexts("    - four spaces"),
            (Some(BlockStart::IndentedCode), None)
        );
    }

    #[test]
    fn empty_bullets_do_not_interrupt_a_paragraph() {
        assert_eq!(both_contexts("*"), (Some(bullet(2, true)), None));
        assert_eq!(both_contexts("+   "), (Some(bullet(2, true)), None));
        assert_eq!(both_contexts("   *\t"), (Some(bullet(5, true)), None));
        assert_eq!(
            both_contexts("-"),
            (Some(bullet(2, true)), Some(BlockStart::SetextUnderline))
        );
    }

    #[test]
    fn ordered_items_interrupt_a_paragraph_only_when_they_start_at_one_and_are_not_empty() {
        assert_eq!(
            both_contexts("1. item"),
            (Some(ordered(1, 3, false)), Some(ordered(1, 3, false)))
        );
        assert_eq!(
            both_contexts("1) item"),
            (Some(ordered(1, 3, false)), Some(ordered(1, 3, false)))
        );
        assert_eq!(
            both_contexts("01. item"),
            (Some(ordered(1, 4, false)), Some(ordered(1, 4, false)))
        );
        assert_eq!(both_contexts("2. item"), (Some(ordered(2, 3, false)), None));
        assert_eq!(
            both_contexts("10) item"),
            (Some(ordered(10, 4, false)), None)
        );
        assert_eq!(both_contexts("1."), (Some(ordered(1, 3, true)), None));
        assert_eq!(both_contexts("1.   "), (Some(ordered(1, 3, true)), None));
        assert_eq!(
            both_contexts("123456789. nine digits"),
            (Some(ordered(123_456_789, 11, false)), None)
        );
        for line in [
            "1234567890. ten digits",
            "3.5 seconds",
            "1.item",
            "1: x",
            "v1. x",
        ] {
            assert_eq!(both_contexts(line), (None, None), "{line:?}");
        }
    }

    #[test]
    fn list_item_content_starts_one_column_after_the_marker_when_five_columns_follow_it() {
        assert_eq!(
            BlockStart::parse("-    four", Preceding::NoParagraph),
            Some(bullet(5, false))
        );
        assert_eq!(
            BlockStart::parse("-     five", Preceding::NoParagraph),
            Some(bullet(2, false))
        );
        assert_eq!(
            BlockStart::parse("1.     five", Preceding::NoParagraph),
            Some(ordered(1, 3, false))
        );
        assert_eq!(
            BlockStart::parse("-\t\tcode", Preceding::NoParagraph),
            Some(bullet(2, false))
        );
    }

    #[test]
    fn list_items_and_headings_allow_three_columns_of_indent_but_not_four() {
        assert_eq!(
            BlockStart::parse("   - three", Preceding::NoParagraph),
            Some(bullet(5, false))
        );
        assert_eq!(
            BlockStart::parse("    - four", Preceding::NoParagraph),
            Some(BlockStart::IndentedCode)
        );
        assert_eq!(
            BlockStart::parse("  \t- tab", Preceding::NoParagraph),
            Some(BlockStart::IndentedCode)
        );
        assert_eq!(
            BlockStart::parse("   # three", Preceding::NoParagraph),
            Some(BlockStart::AtxHeading)
        );
        assert_eq!(
            BlockStart::parse("    # four", Preceding::NoParagraph),
            Some(BlockStart::IndentedCode)
        );
    }

    #[test]
    fn html_blocks_meet_one_of_seven_start_conditions() {
        let cases = [
            ("<pre>", HtmlBlockCondition::RawText),
            ("<SCRIPT type=\"x\">", HtmlBlockCondition::RawText),
            ("<style", HtmlBlockCondition::RawText),
            ("<textarea>text", HtmlBlockCondition::RawText),
            ("<!-- comment -->", HtmlBlockCondition::Comment),
            ("<?php", HtmlBlockCondition::ProcessingInstruction),
            ("<!DOCTYPE html>", HtmlBlockCondition::Declaration),
            ("<![CDATA[x]]>", HtmlBlockCondition::Cdata),
            ("<div>", HtmlBlockCondition::BlockTag),
            ("<div class=\"x\">text", HtmlBlockCondition::BlockTag),
            ("</TABLE>", HtmlBlockCondition::BlockTag),
            ("<hr/>", HtmlBlockCondition::BlockTag),
            ("<p", HtmlBlockCondition::BlockTag),
            ("<details open>", HtmlBlockCondition::BlockTag),
        ];
        for (line, condition) in cases {
            assert_eq!(
                both_contexts(line),
                (Some(html(condition)), Some(html(condition))),
                "{line:?}"
            );
        }
        for line in [
            "<span>",
            "<span class=\"x\" data-y='1' z=w hidden>",
            "<a\thref=x/>",
            "</span>",
            "</em >  ",
            "<custom-tag>",
        ] {
            assert_eq!(
                both_contexts(line),
                (Some(html(HtmlBlockCondition::CompleteTag)), None),
                "{line:?}"
            );
        }
        for line in [
            "<span>text",
            "<https://example.com>",
            "<a href=\"x>",
            "<a b=>",
            "<a\"b>",
            "</pre>",
            "<prefix",
            "< div>",
            "<1a>",
            "a <div>",
        ] {
            assert_eq!(both_contexts(line), (None, None), "{line:?}");
        }
        assert_eq!(
            both_contexts("<!x"),
            (
                Some(html(HtmlBlockCondition::Declaration)),
                Some(html(HtmlBlockCondition::Declaration))
            )
        );
        assert_eq!(
            both_contexts("<prefix>"),
            (Some(html(HtmlBlockCondition::CompleteTag)), None)
        );
        assert_eq!(
            both_contexts("<divider>"),
            (Some(html(HtmlBlockCondition::CompleteTag)), None)
        );
    }

    #[test]
    fn footnote_definitions_interrupt_a_paragraph_but_link_definitions_do_not() {
        for line in ["[^1]: A footnote.", "[^note]:text", "   [^a b]:"] {
            assert_eq!(
                both_contexts(line),
                (
                    Some(BlockStart::FootnoteDefinition),
                    Some(BlockStart::FootnoteDefinition)
                ),
                "{line:?}"
            );
        }
        for line in [
            "[`Foo`]: crate::Foo",
            "[label]:",
            "   [a]: https://example.com",
        ] {
            assert_eq!(
                both_contexts(line),
                (Some(BlockStart::LinkReferenceDefinition), None),
                "{line:?}"
            );
        }
        for line in ["[^1] text", "[`Foo`] is a type.", "[`Foo`]:bar"] {
            assert_eq!(both_contexts(line), (None, None), "{line:?}");
        }
    }

    #[test]
    fn table_rows_and_plain_text_open_no_block() {
        for line in [
            "| a | b |",
            "|---|---|",
            "a | b",
            "Plain prose.",
            "`code` span",
            "NOTE: tag",
        ] {
            assert_eq!(both_contexts(line), (None, None), "{line:?}");
        }
    }

    #[test]
    fn blank_lines_open_no_block() {
        for line in ["", "   ", "\t", "      "] {
            assert_eq!(both_contexts(line), (None, None), "{line:?}");
        }
    }

    #[test]
    fn any_list_item_ends_a_lazy_paragraph_but_a_setext_underline_does_not() {
        let cases = [
            ("2. item", Some(ordered(2, 3, false))),
            ("1.", Some(ordered(1, 3, true))),
            ("*", Some(bullet(2, true))),
            ("-", Some(bullet(2, true))),
            ("---", Some(BlockStart::ThematicBreak)),
            ("===", None),
            ("# Heading", Some(BlockStart::AtxHeading)),
            ("<div>", Some(html(HtmlBlockCondition::BlockTag))),
            ("<span>", None),
            ("[label]: https://example.com", None),
            ("[^1]: A footnote.", Some(BlockStart::FootnoteDefinition)),
            ("    indented", None),
            ("Plain prose.", None),
        ];
        for (line, block_start) in cases {
            assert_eq!(
                BlockStart::parse(line, Preceding::LazyParagraph),
                block_start,
                "{line:?}"
            );
        }
    }

    #[test]
    fn indented_code_does_not_interrupt_a_paragraph() {
        assert_eq!(
            both_contexts("    let a = 1;"),
            (Some(BlockStart::IndentedCode), None)
        );
        assert_eq!(
            both_contexts("\tlet a = 1;"),
            (Some(BlockStart::IndentedCode), None)
        );
    }

    #[test]
    fn list_item_content_indent_is_the_byte_offset_of_the_item_text() {
        for line in ["- \titem", "*\titem", "1.\titem", "  1)  \titem"] {
            let Some(
                BlockStart::BulletItem { content_indent, .. }
                | BlockStart::OrderedItem { content_indent, .. },
            ) = BlockStart::parse(line, Preceding::NoParagraph)
            else {
                panic!("{line:?} starts a list item");
            };
            assert_eq!(&line[content_indent..], "item", "{line:?}");
        }
    }

    #[test]
    fn html_blocks_end_by_the_end_condition_of_their_start_condition() {
        use HtmlBlockCondition::BlockTag;
        use HtmlBlockCondition::Cdata;
        use HtmlBlockCondition::Comment;
        use HtmlBlockCondition::CompleteTag;
        use HtmlBlockCondition::Declaration;
        use HtmlBlockCondition::ProcessingInstruction;
        use HtmlBlockCondition::RawText;
        let cases = [
            (RawText, "</PRE>", true),
            (RawText, "text </script> more", true),
            (RawText, "<pre>", false),
            (Comment, "a -->", true),
            (Comment, "a ->", false),
            (ProcessingInstruction, "?>", true),
            (ProcessingInstruction, "? >", false),
            (Declaration, ">", true),
            (Declaration, "x", false),
            (Cdata, "]]>", true),
            (Cdata, "]>", false),
            (BlockTag, " \t", true),
            (BlockTag, "</div>", false),
            (CompleteTag, "", true),
            (CompleteTag, "text", false),
        ];
        for (condition, line, is_ended) in cases {
            assert_eq!(
                condition.is_ended_by(line),
                is_ended,
                "{condition:?}: {line:?}"
            );
        }
    }
}
