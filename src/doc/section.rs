//! Selecting a `# Title` section of a docstring and grouping its lines into prose paragraphs,
//! top-level bullets and sub-bullets.

use std::ops::Range;

use crate::diagnostic::Position;
use crate::doc::doc_lines::DocLine;
use crate::doc::links::LinkDefinitions;
use crate::markdown;
use crate::markdown::FenceTracker;

/// The `# Title` section of an item's docstring.
#[derive(Debug)]
pub struct DocSection {
    /// The position of the section's header.
    pub header: Position,

    /// The indices of the doc lines after the header, through the line before the next header of
    /// the same or a smaller level.
    body: Range<usize>,

    /// The positions of the later headers with the same title and level.
    pub duplicate_headers: Vec<Position>,
}

impl DocSection {
    /// Groups the section's body lines:
    ///
    /// * Fenced code blocks, link reference definitions and blank lines are skipped.
    /// * Bullets at the body's smallest bullet indent are top-level bullets. A deeper bullet
    ///   belongs to the preceding top-level bullet. If no top-level bullet precedes it, or prose
    ///   comes between them, it is prose, marker included.
    /// * A non-blank, non-bullet line indented at least to an open bullet's content indent
    ///   continues that bullet, and is joined to it with one space.
    /// * A non-bullet line after a blank line, indented at least to the last top-level bullet's
    ///   content indent, is a later paragraph of that bullet, and is skipped.
    /// * Other non-bullet lines are prose. Headers deeper than the section's are prose too.
    ///
    /// # Returns
    ///
    /// The grouped content of the section, where `doc_lines` are the doc lines the section was
    /// found in and `definitions` are their link reference definitions.
    pub fn content(&self, doc_lines: &[DocLine], definitions: &LinkDefinitions) -> SectionContent {
        let mut builder = ContentBuilder {
            definitions,
            content: SectionContent::default(),
        };
        builder.group(doc_lines, self.body.clone());
        builder.content
    }
}

/// The lines of a section's body, grouped into prose paragraphs, top-level bullets and sub-bullets.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SectionContent {
    /// The prose paragraphs, each with its lines joined with one space.
    pub prose: Vec<String>,

    pub bullets: Vec<Bullet>,

    /// The first line outside fences and link reference definitions, if any.
    pub first_content: Option<FirstContent>,
}

/// A top-level bullet of a section, with its continuation lines joined.
#[derive(Debug, PartialEq, Eq)]
pub struct Bullet {
    /// The bullet's text after its marker, with its continuation lines joined with one space.
    pub text: String,

    /// The position of the bullet's first line.
    pub position: Position,

    /// The column, counted from the start of the line, where the bullet's text starts.
    content_indent: usize,

    /// Whether a prose paragraph of the section comes right before the bullet, with no bullet
    /// between them.
    pub follows_prose: bool,

    pub sub_bullets: Vec<SubBullet>,
}

/// A bullet nested in a top-level bullet, with its continuation lines joined.
#[derive(Debug, PartialEq, Eq)]
pub struct SubBullet {
    /// The sub-bullet's text after its marker, with its continuation lines joined with one space.
    pub text: String,

    /// The position of the sub-bullet's first line.
    position: Position,

    /// The column, counted from the start of the line, where the sub-bullet's text starts.
    content_indent: usize,
}

/// The first line of a section's body outside fences and link reference definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstContent {
    pub position: Position,

    /// Whether the line starts a prose paragraph rather than a bullet.
    pub is_prose: bool,
}

/// Finds the section titled `title` in an item's docstring:
///
/// * Headers are ATX headers outside fenced code blocks.
/// * The section starts at the first header titled exactly `title` of the smallest level among the
///   headers with that title.
/// * If a header of a smaller level comes before it, the header belongs to that other section, and
///   the item has no section titled `title`.
/// * The section ends at the next header of the same or a smaller level.
/// * Each later header with the same title and level is a duplicate header.
///
/// # Returns
///
/// The section, or [`None`] if the docstring has no section titled `title`.
pub fn find(doc_lines: &[DocLine], title: &str) -> Option<DocSection> {
    let headers = headers(doc_lines);
    let level = headers
        .iter()
        .filter(|header| header.title == title)
        .map(|header| header.level)
        .min()?;
    let selected = headers
        .iter()
        .position(|header| header.title == title && header.level == level)?;
    if headers[..selected]
        .iter()
        .any(|header| header.level < level)
    {
        return None;
    }
    let start_index = headers[selected].line_index;
    let end_index = headers[selected + 1..]
        .iter()
        .find(|header| header.level <= level)
        .map_or(doc_lines.len(), |header| header.line_index);
    let duplicate_headers = headers[selected + 1..]
        .iter()
        .filter(|header| header.title == title && header.level == level)
        .map(|header| doc_lines[header.line_index].position)
        .collect();
    Some(DocSection {
        header: doc_lines[start_index].position,
        body: start_index + 1..end_index,
        duplicate_headers,
    })
}

/// A header of a docstring.
///
/// # Type Parameters
///
/// * `'doc` - The lifetime of the doc line the header is on.
struct Header<'doc> {
    line_index: usize,
    level: usize,
    title: &'doc str,
}

/// The item that the next continuation line may extend.
enum OpenItem {
    Prose { index: usize },
    Bullet { index: usize },
    SubBullet { index: usize, sub_index: usize },
}

/// Accumulates the content of a section while its lines are grouped.
///
/// # Type Parameters
///
/// * `'definitions` - The lifetime of the link reference definitions of the docstring.
struct ContentBuilder<'definitions> {
    definitions: &'definitions LinkDefinitions,
    content: SectionContent,
}

impl ContentBuilder<'_> {
    /// Groups the lines of `line_range`, as [`DocSection::content`] describes.
    fn group(&mut self, doc_lines: &[DocLine], line_range: Range<usize>) {
        let top_indent = self.top_bullet_indent(doc_lines, line_range.clone());
        let mut fence_tracker = FenceTracker::default();
        let mut open_item: Option<OpenItem> = None;
        let mut last_bullet: Option<usize> = None;
        let mut follows_prose = false;
        for line_index in line_range {
            let doc_line = &doc_lines[line_index];
            let text = doc_line.text.as_str();
            if fence_tracker.is_code(text)
                || self.definitions.contains_line(line_index)
                || text.trim().is_empty()
            {
                open_item = None;
                continue;
            }
            if self.content.first_content.is_none() {
                self.content.first_content = Some(FirstContent {
                    position: doc_line.position,
                    is_prose: markdown::bullet_marker(text).is_none(),
                });
            }
            if let Some(bullet) = markdown::bullet_marker(text) {
                let bullets = &mut self.content.bullets;
                match (top_indent, last_bullet) {
                    (Some(top_indent), _) if bullet.indent <= top_indent => {
                        bullets.push(Bullet {
                            text: bullet.content.trim().to_owned(),
                            position: doc_line.position,
                            content_indent: bullet.content_indent,
                            follows_prose,
                            sub_bullets: Vec::new(),
                        });
                        follows_prose = false;
                        last_bullet = Some(bullets.len() - 1);
                        open_item = Some(OpenItem::Bullet {
                            index: bullets.len() - 1,
                        });
                    }
                    (_, Some(index)) => {
                        let sub_bullets = &mut bullets[index].sub_bullets;
                        sub_bullets.push(SubBullet {
                            text: bullet.content.trim().to_owned(),
                            position: doc_line.position,
                            content_indent: bullet.content_indent,
                        });
                        open_item = Some(OpenItem::SubBullet {
                            index,
                            sub_index: sub_bullets.len() - 1,
                        });
                    }
                    (_, None) => {
                        open_item = Some(self.push_prose(text));
                        follows_prose = true;
                    }
                }
                continue;
            }
            let indent = markdown::indent(text);
            let bullets = &mut self.content.bullets;
            match open_item {
                Some(OpenItem::Bullet { index }) if indent >= bullets[index].content_indent => {
                    append_line(&mut bullets[index].text, text);
                }
                Some(OpenItem::SubBullet { index, sub_index })
                    if indent >= bullets[index].sub_bullets[sub_index].content_indent =>
                {
                    append_line(&mut bullets[index].sub_bullets[sub_index].text, text);
                }
                Some(OpenItem::Prose { index }) => {
                    append_line(&mut self.content.prose[index], text);
                }
                _ => {
                    let continues_bullet =
                        last_bullet.is_some_and(|index| indent >= bullets[index].content_indent);
                    if !continues_bullet {
                        last_bullet = None;
                        open_item = Some(self.push_prose(text));
                        follows_prose = true;
                    }
                }
            }
        }
    }

    /// # Returns
    ///
    /// The smallest indent of a bullet in `line_range`, outside fences and definitions.
    fn top_bullet_indent(&self, doc_lines: &[DocLine], line_range: Range<usize>) -> Option<usize> {
        let mut fence_tracker = FenceTracker::default();
        let mut top_indent = None;
        for line_index in line_range {
            let text = doc_lines[line_index].text.as_str();
            if fence_tracker.is_code(text) || self.definitions.contains_line(line_index) {
                continue;
            }
            if let Some(bullet) = markdown::bullet_marker(text) {
                top_indent = Some(
                    top_indent.map_or(bullet.indent, |indent: usize| indent.min(bullet.indent)),
                );
            }
        }
        top_indent
    }

    /// Starts a prose paragraph with `text`.
    ///
    /// # Returns
    ///
    /// The new paragraph as the open item.
    fn push_prose(&mut self, text: &str) -> OpenItem {
        self.content.prose.push(text.trim().to_owned());
        OpenItem::Prose {
            index: self.content.prose.len() - 1,
        }
    }
}

/// # Returns
///
/// The ATX headers of `doc_lines` outside fenced code blocks.
fn headers(doc_lines: &[DocLine]) -> Vec<Header<'_>> {
    let mut headers = Vec::new();
    let mut fence_tracker = FenceTracker::default();
    for (line_index, doc_line) in doc_lines.iter().enumerate() {
        let text = doc_line.text.as_str();
        if fence_tracker.is_code(text) {
            continue;
        }
        if let Some(header) = markdown::atx_header(text) {
            headers.push(Header {
                line_index,
                level: header.level,
                title: header.title,
            });
        }
    }
    headers
}

/// Appends a continuation line to a bullet or paragraph, joined with one space.
fn append_line(text: &mut String, line: &str) {
    text.push(' ');
    text.push_str(line.trim());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_lines(text: &str) -> Vec<DocLine> {
        text.lines()
            .enumerate()
            .map(|(index, line)| DocLine {
                text: line.to_owned(),
                position: Position {
                    line: index + 1,
                    column: 1,
                },
            })
            .collect()
    }

    /// # Returns
    ///
    /// The header line, the 1-based body lines and the duplicate header lines of the section of
    /// `text` titled `title`.
    fn found(text: &str, title: &str) -> Option<(usize, Range<usize>, Vec<usize>)> {
        find(&doc_lines(text), title).map(|section| {
            (
                section.header.line,
                section.body.start + 1..section.body.end + 1,
                section
                    .duplicate_headers
                    .iter()
                    .map(|position| position.line)
                    .collect(),
            )
        })
    }

    fn content_of(text: &str, title: &str) -> SectionContent {
        let doc_lines = doc_lines(text);
        let texts: Vec<&str> = doc_lines.iter().map(|line| line.text.as_str()).collect();
        let definitions = LinkDefinitions::collect(&texts);
        find(&doc_lines, title)
            .expect("a section")
            .content(&doc_lines, &definitions)
    }

    #[test]
    fn smallest_level_header_with_the_title_is_selected() {
        let text =
            "Summary.\n\n## Errors\n\nA.\n\n# Errors\n\nB.\n\n### Errors\n\nC.\n\n# Panics\n\nD.";
        assert_eq!(found(text, "Errors"), Some((7, 8..15, Vec::new())));
        assert_eq!(found(text, "Panics"), Some((15, 16..18, Vec::new())));
        assert_eq!(found(text, "Missing"), None);
    }

    #[test]
    fn header_under_a_smaller_level_header_belongs_to_that_section() {
        assert_eq!(found("# Returns\n\n## Errors\n\nA.", "Errors"), None);
        assert_eq!(
            found("## Returns\n\n## Errors\n\nA.", "Errors"),
            Some((3, 4..6, Vec::new()))
        );
    }

    #[test]
    fn titles_match_exactly_and_headers_in_fences_are_skipped() {
        let text =
            "```\n# Type Parameters\n```\n\n# Type parameters\n\n# Type Parameters\n\n* `T` - A.";
        assert_eq!(found(text, "Type Parameters"), Some((7, 8..10, Vec::new())));
    }

    #[test]
    fn later_headers_with_the_same_title_and_level_are_duplicates() {
        let text = "# Parameters\n\nA.\n\n## Parameters\n\nB.\n\n# Parameters\n\nC.";
        assert_eq!(found(text, "Parameters"), Some((1, 2..9, vec![9])));
    }

    #[test]
    fn body_lines_are_grouped_into_prose_bullets_and_sub_bullets() {
        let text =
            "# Parameters\n\nThe inputs:\n\n* `a` - The first\n  input.\n  * A detail.\n    More \
             detail.\n* `b` - The second.\n\nTrailing\nprose.\n\n```\n* `c` - \
             Fenced.\n```\n\n[`x`]: crate::x";
        let content = content_of(text, "Parameters");
        assert_eq!(content.prose, ["The inputs:", "Trailing prose."]);
        let bullets: Vec<(&str, usize, usize, bool)> = content
            .bullets
            .iter()
            .map(|bullet| {
                (
                    bullet.text.as_str(),
                    bullet.position.line,
                    bullet.content_indent,
                    bullet.follows_prose,
                )
            })
            .collect();
        assert_eq!(
            bullets,
            [
                ("`a` - The first input.", 5, 2, true),
                ("`b` - The second.", 9, 2, false)
            ]
        );
        let sub_bullets: Vec<(&str, usize)> = content.bullets[0]
            .sub_bullets
            .iter()
            .map(|sub_bullet| (sub_bullet.text.as_str(), sub_bullet.position.line))
            .collect();
        assert_eq!(sub_bullets, [("A detail. More detail.", 7)]);
        assert_eq!(
            content.first_content,
            Some(FirstContent {
                position: Position { line: 3, column: 1 },
                is_prose: true,
            })
        );
    }

    #[test]
    fn later_bullet_paragraphs_are_dropped_and_deeper_bullets_after_prose_are_prose() {
        let text = "# Parameters\n\n* `a` - The first.\n\n  More about `a`.\n\nProse.\n  * A \
                    deeper bullet.\n* `b` - The second.";
        let content = content_of(text, "Parameters");
        assert_eq!(content.prose, ["Prose.", "* A deeper bullet."]);
        let bullets: Vec<(&str, usize, bool)> = content
            .bullets
            .iter()
            .map(|bullet| {
                (
                    bullet.text.as_str(),
                    bullet.sub_bullets.len(),
                    bullet.follows_prose,
                )
            })
            .collect();
        assert_eq!(
            bullets,
            [
                ("`a` - The first.", 0, false),
                ("`b` - The second.", 0, true)
            ]
        );
    }

    #[test]
    fn indented_top_level_bullets_and_bullet_first_sections_are_grouped() {
        let content = content_of("# Errors\n\n  * A.\n  * B.\n    * C.", "Errors");
        assert_eq!(content.bullets.len(), 2);
        assert_eq!(content.bullets[1].sub_bullets.len(), 1);
        assert_eq!(
            content
                .first_content
                .map(|first_content| first_content.is_prose),
            Some(false)
        );
        assert_eq!(content_of("# Errors", "Errors"), SectionContent::default());
    }
}
