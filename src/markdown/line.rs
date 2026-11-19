//! The line-level Markdown that the rules read comment and doc lines by: indents, ATX headers, code
//! fences, list items and link reference definitions.

/// The deepest indent at which Markdown recognizes the start of a block other than indented code.
pub const MAX_BLOCK_INDENT: usize = 3;

/// The highest level of an ATX header.
pub const MAX_HEADER_LEVEL: usize = 6;

/// The most digits that the number of an ordered list item can have.
pub const MAX_ORDERED_LIST_DIGITS: usize = 9;

/// An ATX header line, such as `# Errors`.
///
/// # Type Parameters
///
/// * `'text` - The lifetime of the line the header is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtxHeader<'text> {
    pub level: usize,
    pub title: &'text str,
}

/// The opening line of a fenced code block: a run of backticks or tildes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fence {
    marker: char,
    length: usize,
}

impl Fence {
    /// Recognizes a fence line: three or more backticks or tildes after any indent. A backtick
    /// fence's info string can't hold a backtick, so ```` ```a``` ```` is inline code.
    ///
    /// # Returns
    ///
    /// The fence, or [`None`] if `line` isn't a fence line.
    pub fn parse(line: &str) -> Option<Self> {
        let trimmed = line.trim_start();
        let marker = trimmed
            .chars()
            .next()
            .filter(|first| matches!(first, '`' | '~'))?;
        let length = trimmed
            .chars()
            .take_while(|current| *current == marker)
            .count();
        if length < MIN_FENCE_LENGTH {
            return None;
        }
        let info = &trimmed[length * marker.len_utf8()..];
        if marker == '`' && info.contains('`') {
            return None;
        }
        Some(Self { marker, length })
    }

    /// A fence is closed by a fence line of the same character that is at least as long and has no
    /// info string.
    ///
    /// # Returns
    ///
    /// Whether `line` closes the fenced code block that `self` opened.
    pub fn is_closed_by(self, line: &str) -> bool {
        let trimmed = line.trim();
        Self::parse(trimmed).is_some_and(|closing| {
            closing.marker == self.marker
                && closing.length >= self.length
                && closing.length == trimmed.chars().count()
        })
    }
}

/// Tracks whether the lines of one block are inside a fenced code block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FenceTracker {
    open_fence: Option<Fence>,
}

impl FenceTracker {
    /// Moves the tracker past `line`: a fence line outside a fenced block opens one, and a matching
    /// fence line inside one closes it.
    ///
    /// # Returns
    ///
    /// Whether `line` is a fence line or a line inside a fenced code block.
    pub fn is_code(&mut self, line: &str) -> bool {
        let Some(open_fence) = self.open_fence else {
            self.open_fence = Fence::parse(line);
            return self.open_fence.is_some();
        };
        if open_fence.is_closed_by(line) {
            self.open_fence = None;
        }
        true
    }
}

/// The marker of a list item line, such as `* ` or `1. `.
///
/// # Type Parameters
///
/// * `'text` - The lifetime of the line the marker is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulletMarker<'text> {
    /// The indent of the marker.
    pub indent: usize,

    /// The column, counted from the start of the line, where the item's text starts.
    pub content_indent: usize,

    /// The item's text after the marker and its whitespace.
    pub content: &'text str,
}

/// The first line of a link reference definition, such as ``[`Foo`]: crate::foo::Foo``.
///
/// # Type Parameters
///
/// * `'text` - The lifetime of the line the definition is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkDefinitionLine<'text> {
    pub label: &'text str,

    /// The trimmed target after the `:`. It is empty when the target is on the following lines.
    pub target: &'text str,
}

/// # Returns
///
/// The number of spaces and tabs that start `line`.
pub fn indent(line: &str) -> usize {
    line.chars()
        .take_while(|current| matches!(current, ' ' | '\t'))
        .count()
}

/// Recognizes an ATX header: an indent of at most 3, 1 to 6 `#`, then whitespace or the end of the
/// line. A closing `#` run after whitespace isn't part of the title.
///
/// # Returns
///
/// The header, or [`None`] if `line` isn't one. `#544)` and `#[derive]` aren't headers.
pub fn atx_header(line: &str) -> Option<AtxHeader<'_>> {
    let indent = indent(line);
    if indent > MAX_BLOCK_INDENT {
        return None;
    }
    let after_indent = &line[indent..];
    let level = after_indent
        .chars()
        .take_while(|current| *current == '#')
        .count();
    if !(1..=MAX_HEADER_LEVEL).contains(&level) {
        return None;
    }
    let rest = &after_indent[level..];
    if !rest.chars().next().is_none_or(char::is_whitespace) {
        return None;
    }
    let title = rest.trim();
    let without_closing_run = title.trim_end_matches('#');
    let title = if without_closing_run.is_empty() {
        without_closing_run
    } else if without_closing_run.ends_with(char::is_whitespace) {
        without_closing_run.trim_end()
    } else {
        title
    };
    Some(AtxHeader { level, title })
}

/// Recognizes a list item: `*`, `-`, `+`, or 1 to 9 digits followed by `.` or `)`, after any
/// indent, followed by at least one space or tab.
///
/// # Returns
///
/// The item's marker, or [`None`] if `line` isn't a list item.
pub fn bullet_marker(line: &str) -> Option<BulletMarker<'_>> {
    let indent = indent(line);
    let after_indent = &line[indent..];
    let num_digits = after_indent
        .chars()
        .take_while(char::is_ascii_digit)
        .count();
    let marker_length = if after_indent.starts_with(['*', '-', '+']) {
        1
    } else if (1..=MAX_ORDERED_LIST_DIGITS).contains(&num_digits)
        && after_indent[num_digits..].starts_with(['.', ')'])
    {
        num_digits + 1
    } else {
        return None;
    };
    let after_marker = &after_indent[marker_length..];
    let num_spaces = self::indent(after_marker);
    (num_spaces > 0).then(|| BulletMarker {
        indent,
        content_indent: indent + marker_length + num_spaces,
        content: &after_marker[num_spaces..],
    })
}

/// Recognizes the first line of a link reference definition: `[label]:` with an indent of at most
/// 3, where the label is non-blank and has no `[` or `]`, and the `:` ends the line or is followed
/// by whitespace.
///
/// # Returns
///
/// The label and the target, or [`None`] if `line` isn't a definition.
pub fn link_definition(line: &str) -> Option<LinkDefinitionLine<'_>> {
    let indent = indent(line);
    if indent > MAX_BLOCK_INDENT {
        return None;
    }
    let (label, after_label) = line[indent..].strip_prefix('[')?.split_once(']')?;
    if label.trim().is_empty() || label.contains('[') {
        return None;
    }
    let target = after_label.strip_prefix(':')?;
    target
        .chars()
        .next()
        .is_none_or(char::is_whitespace)
        .then(|| LinkDefinitionLine {
            label,
            target: target.trim(),
        })
}

/// The shortest run of backticks or tildes that opens a fenced code block.
const MIN_FENCE_LENGTH: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;

    /// # Returns
    ///
    /// For each of `lines`, whether the tracker sees it as code.
    fn code_lines(lines: &[&str]) -> Vec<bool> {
        let mut fence_tracker = FenceTracker::default();
        lines
            .iter()
            .map(|line| fence_tracker.is_code(line))
            .collect()
    }

    #[test]
    fn atx_headers_need_whitespace_after_the_hashes_and_a_small_indent() {
        assert_eq!(
            atx_header("# Errors"),
            Some(AtxHeader {
                level: 1,
                title: "Errors"
            })
        );
        assert_eq!(
            atx_header("## Errors ##").map(|header| header.title),
            Some("Errors")
        );
        assert_eq!(atx_header("# C#").map(|header| header.title), Some("C#"));
        assert_eq!(
            atx_header("   ###### Six").map(|header| header.level),
            Some(6)
        );
        assert_eq!(atx_header("#").map(|header| header.title), Some(""));
        for line in [
            "#544).",
            "#[derive(Debug)]",
            "####### Seven",
            "    # Indented",
            "Text # not a header",
        ] {
            assert_eq!(atx_header(line), None, "{line:?}");
        }
    }

    #[test]
    fn fences_close_only_on_the_same_character_with_a_long_enough_run() {
        assert_eq!(
            code_lines(&["```", "code", "~~~", "still code", "````", "text"]),
            [true, true, true, true, true, false]
        );
        assert_eq!(
            code_lines(&["  ~~~~ rust", "```", "~~~", "~~~~", "text"]),
            [true, true, true, true, false]
        );
        assert_eq!(
            code_lines(&["```rust", "```rust", "```", "text"]),
            [true, true, true, false]
        );
        assert_eq!(
            code_lines(&["``not a fence", "```a``` is code"]),
            [false, false]
        );
    }

    #[test]
    fn bullets_need_whitespace_after_the_marker() {
        assert_eq!(
            bullet_marker("  * Forwards x"),
            Some(BulletMarker {
                indent: 2,
                content_indent: 4,
                content: "Forwards x"
            })
        );
        let content_indents: Vec<_> = ["+   item", "12. item", "1) item", "-\titem"]
            .into_iter()
            .map(|line| bullet_marker(line).map(|bullet| bullet.content_indent))
            .collect();
        assert_eq!(content_indents, [Some(4), Some(4), Some(3), Some(2)]);
        for line in [
            "*emphasis*",
            "-1 is invalid",
            "3.5 seconds",
            "1234567890. ten digits",
            "--flag",
            "Text",
        ] {
            assert_eq!(bullet_marker(line), None, "{line:?}");
        }
    }

    #[test]
    fn link_definitions_need_a_label_a_colon_and_whitespace_after_it() {
        assert_eq!(
            link_definition("[`Foo::bar`]:"),
            Some(LinkDefinitionLine {
                label: "`Foo::bar`",
                target: ""
            })
        );
        assert_eq!(
            link_definition("   [label]: https://example.com ").map(|definition| definition.target),
            Some("https://example.com")
        );
        for line in [
            "     [label]:",
            "[]:",
            "[ ]: x",
            "[a[b]: x",
            "[`Foo`]:bar",
            "[`Foo`] is a type.",
        ] {
            assert_eq!(link_definition(line), None, "{line:?}");
        }
    }

    #[test]
    fn indent_counts_spaces_and_tabs() {
        assert_eq!(indent("  \tx"), 3);
        assert_eq!(indent("x  "), 0);
        assert_eq!(indent(""), 0);
    }
}
