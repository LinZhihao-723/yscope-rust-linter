//! Selecting a docstring's `# Errors` section and parsing it into the variants and forwarded
//! functions it documents.

use crate::diagnostic::Position;
use crate::doc::doc_lines::DocLine;
use crate::doc::links;
use crate::doc::links::LinkDefinitions;
use crate::doc::links::Reference;
use crate::doc::links::ReferenceKind;
use crate::markdown;
use crate::markdown::FenceTracker;
use crate::syntax::symbol_path::SymbolPath;
use crate::syntax::symbol_path::is_upper_camel_case;

/// The intro line that every `# Errors` section starts with.
pub const CANONICAL_INTRO: &str = "Returns an error if:";

/// What a fn's `# Errors` section documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorsSection {
    /// The position of the `# Errors` header.
    pub header: Position,

    pub variant_bullets: Vec<VariantBullet>,
    pub forward_bullets: Vec<ForwardBullet>,

    /// The link references of the section's prose and of its other bullets.
    pub prose_links: Vec<SymbolPath>,

    /// Whether the section has a top-level bullet of any kind.
    pub has_bullets: bool,

    pub format_findings: Vec<FormatFinding>,
}

/// A bullet that documents directly returned errors, such as ``* [`Error::NotFound`] if ...``.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantBullet {
    /// The link references before the bullet's first condition marker.
    pub head_paths: Vec<SymbolPath>,

    /// The link references after the head and in the sub-bullets, such as the function that a
    /// condition names.
    pub linked_paths: Vec<SymbolPath>,

    pub position: Position,
}

/// A bullet that documents forwarded errors, such as ``* Forwards [`foo`]'s return values on
/// failure.``.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardBullet {
    pub kind: ForwardKind,

    /// The functions the bullet names.
    pub paths: Vec<SymbolPath>,

    pub position: Position,
}

/// How a forward bullet names its functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardKind {
    /// ``Forwards [`a`] and [`b`]'s return values ...``.
    Named,

    /// "Forwards the return values of:" followed by one sub-bullet per function.
    List,

    /// "Forwards ..." without a possessive function reference.
    Unnamed,
}

/// A problem with the section's format rather than with its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatFinding {
    pub kind: FormatFindingKind,
    pub position: Position,
}

/// The kinds of [`FormatFinding`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatFindingKind {
    /// A forward bullet names no function. `code_names` are the one-segment bare code spans in it,
    /// such as `` `decode` ``, which may name a callable parameter.
    UnnamedForward { code_names: Vec<String> },

    /// A forward bullet's wording isn't "Forwards [`func`]'s return values on failure": it says
    /// "Forward", uses a bare `'` possessive, or words what follows the possessive differently.
    NonCanonicalForward,

    /// A forward bullet names `path`, which is a type rather than a function.
    ForwardNamesType { path: SymbolPath },

    /// The section doesn't start with "Returns an error if:" followed by bullets.
    NonCanonicalIntro,

    /// A second `# Errors` header at the same level.
    DuplicateSection,
}

/// Finds and parses the fn's `# Errors` section.
///
/// The section is selected as follows:
///
/// * Headers are ATX headers outside fenced code blocks. An Errors header's title is `Errors`.
/// * The section starts at the first Errors header of the smallest level.
/// * If a header of a smaller level comes before it, the Errors header belongs to that other
///   section, and the fn has no `# Errors` section.
/// * The section ends at the next header of the same or a smaller level.
/// * Another Errors header of the same level becomes a duplicate-section format finding.
///
/// # Returns
///
/// The parsed section, or [`None`] if the docstring has no `# Errors` section.
pub fn parse(doc_lines: &[DocLine]) -> Option<ErrorsSection> {
    let texts: Vec<&str> = doc_lines.iter().map(|line| line.text.as_str()).collect();
    let headers = headers(&texts);
    let errors_level = headers
        .iter()
        .filter(|header| header.is_errors)
        .map(|header| header.level)
        .min()?;
    let selected = headers
        .iter()
        .position(|header| header.is_errors && header.level == errors_level)?;
    if headers[..selected]
        .iter()
        .any(|header| header.level < errors_level)
    {
        return None;
    }
    let start_index = headers[selected].line_index;
    let end_index = headers[selected + 1..]
        .iter()
        .find(|header| header.level <= errors_level)
        .map_or(doc_lines.len(), |header| header.line_index);

    let definitions = LinkDefinitions::collect(&texts);
    let mut section = SectionBuilder::new(&definitions, doc_lines[start_index].position);
    for header in &headers[selected + 1..] {
        if header.is_errors && header.level == errors_level {
            section.format_findings.push(FormatFinding {
                kind: FormatFindingKind::DuplicateSection,
                position: doc_lines[header.line_index].position,
            });
        }
    }
    section.group(doc_lines, start_index + 1..end_index);
    Some(section.finish())
}

/// The wording that follows the possessive of a canonical forward bullet.
const CANONICAL_FORWARD_WORDING: &str = " return values on failure";

/// The wording between the possessive of a canonical mapped forward bullet and the reference to the
/// variant the errors are mapped into, as in ``Forwards [`f`]'s return values as [`E::V`] on
/// failure``.
const MAPPED_FORWARD_WORDING: &str = " return values as ";

/// The wording that follows the variant reference of a canonical mapped forward bullet.
const MAPPED_FORWARD_SUFFIX: &str = " on failure";

/// A top-level bullet of the section, with its continuation lines joined.
struct BulletItem {
    text: String,
    position: Position,
    content_indent: usize,
    sub_bullets: Vec<SubBullet>,
}

impl BulletItem {
    /// Classifies one top-level bullet and adds it to `section`, in this order:
    ///
    /// 1. A bullet starting with `Forwards` or `Forward` whose references include one followed by
    ///    `'s` or `'` is a [`ForwardKind::Named`] bullet. It names every reference up to that one.
    ///    Text after `except` is ignored. Unless the possessive `'s` is followed by a wording that
    ///    [`is_canonical_forward_wording`] accepts, the bullet gets a format finding.
    /// 2. Such a bullet without a possessive reference that ends with `:` and has sub-bullets is a
    ///    [`ForwardKind::List`] bullet. It names the first reference of each sub-bullet.
    /// 3. Any other `Forwards` bullet is a [`ForwardKind::Unnamed`] bullet. It names every
    ///    reference in it and gets a format finding.
    /// 4. A bullet starting with a link reference, optionally after "Returns a " or "Returns an ",
    ///    is a variant bullet. Its head is its text up to the first condition marker.
    /// 5. Any other bullet adds its link references to the prose links.
    fn classify_into(&self, definitions: &LinkDefinitions, section: &mut ErrorsSection) {
        let text = self.text.as_str();
        if let Some(forward_word) = forward_word(text) {
            self.classify_forward_into(forward_word, definitions, section);
            return;
        }

        let body = text
            .strip_prefix("Returns an ")
            .or_else(|| text.strip_prefix("Returns a "))
            .unwrap_or(text);
        let references = links::references(body, definitions);
        let starts_with_link = references
            .first()
            .is_some_and(|reference| reference.kind == ReferenceKind::Link && reference.start == 0);
        if starts_with_link {
            let head_end = head_end(body);
            let head_paths = link_paths(
                &references
                    .iter()
                    .filter(|reference| reference.start < head_end)
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            if !head_paths.is_empty() {
                let linked_paths = link_paths(
                    &references
                        .iter()
                        .filter(|reference| reference.start >= head_end)
                        .cloned()
                        .collect::<Vec<_>>(),
                )
                .into_iter()
                .chain(self.sub_bullets.iter().flat_map(|sub_bullet| {
                    link_paths(&links::references(&sub_bullet.text, definitions))
                }))
                .collect();
                section.variant_bullets.push(VariantBullet {
                    head_paths,
                    linked_paths,
                    position: self.position,
                });
                return;
            }
        }
        section.prose_links.extend(link_paths(&references));
    }

    /// Classifies a bullet that starts with `forward_word` (`Forwards` or `Forward`) and adds it to
    /// `section`, as steps 1 to 3 of [`Self::classify_into`] describe. A named or listed function
    /// whose last segment is in upper camel case is a type, which gets a format finding.
    fn classify_forward_into(
        &self,
        forward_word: &str,
        definitions: &LinkDefinitions,
        section: &mut ErrorsSection,
    ) {
        let text = self.text.as_str();
        let named_text = text.find(" except ").map_or(text, |index| &text[..index]);
        let references = links::references(named_text, definitions);
        let possessive = references
            .iter()
            .enumerate()
            .find_map(|(index, reference)| {
                let after = &named_text[reference.end..];
                after.starts_with('\'').then(|| Possessive {
                    reference_index: index,
                    is_canonical: is_canonical_forward_wording(named_text, &references, index),
                })
            });
        let (kind, paths) = if let Some(possessive) = possessive {
            if forward_word == "Forward" || !possessive.is_canonical {
                section.format_findings.push(FormatFinding {
                    kind: FormatFindingKind::NonCanonicalForward,
                    position: self.position,
                });
            }
            (
                ForwardKind::Named,
                all_paths(&references[..=possessive.reference_index]),
            )
        } else if text.trim_end().ends_with(':') && !self.sub_bullets.is_empty() {
            let paths = self
                .sub_bullets
                .iter()
                .filter_map(|sub_bullet| {
                    links::references(&sub_bullet.text, definitions)
                        .first()
                        .and_then(|reference| SymbolPath::parse(&reference.path_text))
                })
                .collect();
            (ForwardKind::List, paths)
        } else {
            let code_names = references
                .iter()
                .filter(|reference| reference.kind == ReferenceKind::BareCode)
                .filter_map(|reference| SymbolPath::parse(&reference.path_text))
                .filter(|path| path.len() == 1)
                .map(|path| path.last().to_owned())
                .collect();
            section.format_findings.push(FormatFinding {
                kind: FormatFindingKind::UnnamedForward { code_names },
                position: self.position,
            });
            (ForwardKind::Unnamed, all_paths(&references))
        };
        let named_paths = paths.iter().filter(|_| kind != ForwardKind::Unnamed);
        for path in named_paths.filter(|path| is_upper_camel_case(path.last())) {
            section.format_findings.push(FormatFinding {
                kind: FormatFindingKind::ForwardNamesType { path: path.clone() },
                position: self.position,
            });
        }
        section.forward_bullets.push(ForwardBullet {
            kind,
            paths,
            position: self.position,
        });
    }
}

/// Decides whether the possessive after the reference at `reference_index` of `text` is followed by
/// a canonical forward wording:
///
/// * `'s return values on failure`.
/// * `'s return values as`, then a reference to the variant the errors are mapped into, then `on
///   failure`.
///
/// # Returns
///
/// Whether the wording is canonical.
fn is_canonical_forward_wording(
    text: &str,
    references: &[Reference],
    reference_index: usize,
) -> bool {
    let possessive_end = references[reference_index].end + "'s".len();
    let Some(wording) = text[references[reference_index].end..].strip_prefix("'s") else {
        return false;
    };
    if wording.starts_with(CANONICAL_FORWARD_WORDING) {
        return true;
    }
    wording.starts_with(MAPPED_FORWARD_WORDING)
        && references.get(reference_index + 1).is_some_and(|mapped| {
            mapped.start == possessive_end + MAPPED_FORWARD_WORDING.len()
                && text[mapped.end..].starts_with(MAPPED_FORWARD_SUFFIX)
        })
}

/// A sub-bullet, with its continuation lines joined.
struct SubBullet {
    text: String,
    content_indent: usize,
}

/// The reference of a forward bullet that is followed by a possessive `'s` or `'`.
struct Possessive {
    reference_index: usize,

    /// Whether the possessive is `'s` followed by the canonical wording.
    is_canonical: bool,
}

/// The item that the next continuation line may extend.
#[derive(Clone, Copy)]
enum OpenItem {
    Prose { index: usize },
    Bullet { index: usize },
    SubBullet { index: usize, sub_index: usize },
}

/// A header of the docstring.
struct Header {
    line_index: usize,
    level: usize,
    is_errors: bool,
}

/// Accumulates the parts of a section while its lines are grouped and its bullets classified.
struct SectionBuilder<'definitions> {
    definitions: &'definitions LinkDefinitions,
    header: Position,
    prose: Vec<String>,
    bullets: Vec<BulletItem>,
    format_findings: Vec<FormatFinding>,

    /// The position of the section's first line outside fences and definitions, and whether that
    /// line starts the first prose paragraph.
    first_content: Option<(Position, bool)>,
}

impl<'definitions> SectionBuilder<'definitions> {
    const fn new(definitions: &'definitions LinkDefinitions, header: Position) -> Self {
        Self {
            definitions,
            header,
            prose: Vec::new(),
            bullets: Vec::new(),
            format_findings: Vec::new(),
            first_content: None,
        }
    }

    /// Groups the section's lines into prose paragraphs, top-level bullets and sub-bullets.
    ///
    /// * Fenced code blocks and link reference definitions are skipped.
    /// * Bullets at the section's smallest bullet indent are top-level bullets. Deeper bullets
    ///   belong to the preceding top-level bullet.
    /// * A non-blank, non-bullet line indented at least to an open bullet's content indent
    ///   continues that bullet, and is joined to it with one space.
    /// * Other non-bullet lines are prose.
    fn group(&mut self, doc_lines: &[DocLine], line_range: std::ops::Range<usize>) {
        let top_indent = self.top_bullet_indent(doc_lines, line_range.clone());
        let mut fence_tracker = FenceTracker::default();
        let mut open_item: Option<OpenItem> = None;
        let mut last_bullet: Option<usize> = None;
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
            if self.first_content.is_none() {
                let is_prose = markdown::bullet_marker(text).is_none();
                self.first_content = Some((doc_line.position, is_prose));
            }
            if let Some(bullet) = markdown::bullet_marker(text) {
                match (top_indent, last_bullet) {
                    (Some(top_indent), _) if bullet.indent <= top_indent => {
                        self.bullets.push(BulletItem {
                            text: bullet.content.trim().to_owned(),
                            position: doc_line.position,
                            content_indent: bullet.content_indent,
                            sub_bullets: Vec::new(),
                        });
                        last_bullet = Some(self.bullets.len() - 1);
                        open_item = Some(OpenItem::Bullet {
                            index: self.bullets.len() - 1,
                        });
                    }
                    (_, Some(index)) => {
                        let sub_bullets = &mut self.bullets[index].sub_bullets;
                        sub_bullets.push(SubBullet {
                            text: bullet.content.trim().to_owned(),
                            content_indent: bullet.content_indent,
                        });
                        open_item = Some(OpenItem::SubBullet {
                            index,
                            sub_index: sub_bullets.len() - 1,
                        });
                    }
                    (_, None) => {
                        open_item = Some(self.push_prose(text));
                    }
                }
                continue;
            }
            let indent = markdown::indent(text);
            match open_item {
                Some(OpenItem::Bullet { index })
                    if indent >= self.bullets[index].content_indent =>
                {
                    append_line(&mut self.bullets[index].text, text);
                }
                Some(OpenItem::SubBullet { index, sub_index })
                    if indent >= self.bullets[index].sub_bullets[sub_index].content_indent =>
                {
                    append_line(&mut self.bullets[index].sub_bullets[sub_index].text, text);
                }
                Some(OpenItem::Prose { index }) => append_line(&mut self.prose[index], text),
                _ => {
                    let continues_bullet = last_bullet
                        .is_some_and(|index| indent >= self.bullets[index].content_indent);
                    if !continues_bullet {
                        last_bullet = None;
                        open_item = Some(self.push_prose(text));
                    }
                }
            }
        }
    }

    /// # Returns
    ///
    /// The smallest indent of a bullet in the section, outside fences and definitions.
    fn top_bullet_indent(
        &self,
        doc_lines: &[DocLine],
        line_range: std::ops::Range<usize>,
    ) -> Option<usize> {
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
        self.prose.push(text.trim().to_owned());
        OpenItem::Prose {
            index: self.prose.len() - 1,
        }
    }

    /// Classifies every top-level bullet and collects the prose links. A section whose first
    /// paragraph isn't [`CANONICAL_INTRO`], or that has no bullet, gets a format finding at its
    /// first line.
    ///
    /// # Returns
    ///
    /// The finished section.
    fn finish(mut self) -> ErrorsSection {
        let has_canonical_intro = matches!(self.first_content, Some((_, true)))
            && self
                .prose
                .first()
                .is_some_and(|intro| intro == CANONICAL_INTRO);
        if !has_canonical_intro || self.bullets.is_empty() {
            self.format_findings.push(FormatFinding {
                kind: FormatFindingKind::NonCanonicalIntro,
                position: self
                    .first_content
                    .map_or(self.header, |(position, _)| position),
            });
        }
        let mut section = ErrorsSection {
            header: self.header,
            variant_bullets: Vec::new(),
            forward_bullets: Vec::new(),
            prose_links: Vec::new(),
            has_bullets: !self.bullets.is_empty(),
            format_findings: self.format_findings,
        };
        for paragraph in &self.prose {
            section
                .prose_links
                .extend(link_paths(&links::references(paragraph, self.definitions)));
        }
        for bullet in &self.bullets {
            bullet.classify_into(self.definitions, &mut section);
        }
        section
    }
}

/// # Returns
///
/// The ATX headers of `texts` outside fenced code blocks.
fn headers(texts: &[&str]) -> Vec<Header> {
    let mut headers = Vec::new();
    let mut fence_tracker = FenceTracker::default();
    for (line_index, text) in texts.iter().enumerate() {
        if fence_tracker.is_code(text) {
            continue;
        }
        if let Some(header) = markdown::atx_header(text) {
            headers.push(Header {
                line_index,
                level: header.level,
                is_errors: header.title == "Errors",
            });
        }
    }
    headers
}

/// # Returns
///
/// `Forwards` or `Forward` if `text` starts with that word, or [`None`] otherwise.
fn forward_word(text: &str) -> Option<&'static str> {
    ["Forwards", "Forward"].into_iter().find(|word| {
        text.strip_prefix(word)
            .is_some_and(|rest| !rest.starts_with(|character: char| character.is_alphanumeric()))
    })
}

/// # Returns
///
/// The byte offset where a variant bullet's head ends: at the first condition marker (the word
/// `if`, `when`, `for` or `unless` between spaces), or at a `:` that ends the bullet.
fn head_end(text: &str) -> usize {
    let marker_index = [" if ", " when ", " for ", " unless "]
        .into_iter()
        .filter_map(|marker| text.find(marker))
        .min();
    marker_index.unwrap_or_else(|| {
        let trimmed = text.trim_end();
        trimmed.strip_suffix(':').map_or(text.len(), str::len)
    })
}

/// # Returns
///
/// The paths of the link references among `references`, skipping texts that aren't paths.
fn link_paths(references: &[Reference]) -> Vec<SymbolPath> {
    references
        .iter()
        .filter(|reference| reference.kind == ReferenceKind::Link)
        .filter_map(|reference| SymbolPath::parse(&reference.path_text))
        .collect()
}

/// # Returns
///
/// The paths of all `references`, including bare code spans, skipping texts that aren't paths.
fn all_paths(references: &[Reference]) -> Vec<SymbolPath> {
    references
        .iter()
        .filter_map(|reference| SymbolPath::parse(&reference.path_text))
        .collect()
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

    fn section(text: &str) -> ErrorsSection {
        parse(&doc_lines(text)).expect("an errors section")
    }

    fn strings(paths: &[SymbolPath]) -> Vec<String> {
        paths.iter().map(ToString::to_string).collect()
    }

    fn forward_paths(section: &ErrorsSection) -> Vec<(ForwardKind, Vec<String>)> {
        section
            .forward_bullets
            .iter()
            .map(|bullet| (bullet.kind, strings(&bullet.paths)))
            .collect()
    }

    fn variant_heads(section: &ErrorsSection) -> Vec<Vec<String>> {
        section
            .variant_bullets
            .iter()
            .map(|bullet| strings(&bullet.head_paths))
            .collect()
    }

    /// # Returns
    ///
    /// The kinds and lines of the section's format findings, except the intro findings, which
    /// [`intro_finding_lines`] reports.
    fn format_kinds(section: &ErrorsSection) -> Vec<(FormatFindingKind, usize)> {
        section
            .format_findings
            .iter()
            .filter(|finding| finding.kind != FormatFindingKind::NonCanonicalIntro)
            .map(|finding| (finding.kind.clone(), finding.position.line))
            .collect()
    }

    fn intro_finding_lines(section: &ErrorsSection) -> Vec<usize> {
        section
            .format_findings
            .iter()
            .filter(|finding| finding.kind == FormatFindingKind::NonCanonicalIntro)
            .map(|finding| finding.position.line)
            .collect()
    }

    #[test]
    fn smallest_level_errors_header_is_selected() {
        let text = "Summary.\n\n# Returns\n\nA stream.\n\n## Errors\n\n* [`StreamError::A`] if \
                    a.\n\n# Errors\n\n* [`Error::B`] if b.\n\n### Errors\n\n* [`Error::C`] if \
                    c.\n\n# Panics\n\n* [`Error::D`] if d.";
        let section = section(text);
        assert_eq!(section.header.line, 11);
        assert_eq!(variant_heads(&section), [["Error::B"], ["Error::C"]]);
    }

    #[test]
    fn errors_header_under_a_smaller_level_header_is_not_the_fns_section() {
        let text = "# Returns\n\n## Errors\n\n* [`StreamError::A`] if a.";
        assert_eq!(parse(&doc_lines(text)), None);
        assert_eq!(parse(&doc_lines("No headers.")), None);
    }

    #[test]
    fn section_ends_at_a_smaller_level_header() {
        let text = "## Errors\n\n* [`Error::A`] if a.\n\n# Panics\n\n* [`Error::B`] if b.";
        assert_eq!(variant_heads(&section(text)), [["Error::A"]]);
    }

    #[test]
    fn duplicate_same_level_header_is_a_format_finding() {
        let section = section("# Errors\n\n* [`A`] if a.\n\n# Errors\n\n* [`B`] if b.");
        assert_eq!(variant_heads(&section), [["A"]]);
        assert_eq!(
            format_kinds(&section),
            [(FormatFindingKind::DuplicateSection, 5)]
        );
    }

    #[test]
    fn fences_inside_the_section_are_skipped() {
        let text = "# Errors\n\n```\n# Not a header\n* [`Fake`] if x.\n```\n\n* [`Real`] if y.";
        assert_eq!(variant_heads(&section(text)), [["Real"]]);
        let text = "# Errors\n\n````\n```\n* [`Fake`] if x.\n````\n\n* [`Real`] if y.";
        assert_eq!(variant_heads(&section(text)), [["Real"]]);
    }

    #[test]
    fn multi_line_bullets_are_joined() {
        let text = "# Errors\n\nReturns an error if:\n\n* Forwards\n  \
                    [`aws::ReceiveMessageFluentBuilder::send`]'s\n  return values on failure.\n* \
                    [`Error::A`] if the first\n  condition holds.";
        let section = section(text);
        assert_eq!(
            forward_paths(&section),
            [(
                ForwardKind::Named,
                vec!["aws::ReceiveMessageFluentBuilder::send".to_owned()]
            )]
        );
        assert_eq!(variant_heads(&section), [["Error::A"]]);
        assert_eq!(section.format_findings, []);
    }

    #[test]
    fn forward_list_takes_one_target_per_sub_bullet() {
        let text = "# Errors\n\n* Forwards the return values of:\n  * [`a`]\n  * [`B::b`]'s \
                    errors.\n* Forwards [`c`]'s return values on failure if:\n  * [`Ignored`] is \
                    set.\n* [`syn::Error`] if:\n  * [`AlsoIgnored`] is set.";
        let section = section(text);
        assert_eq!(
            forward_paths(&section),
            [
                (ForwardKind::List, vec!["a".to_owned(), "B::b".to_owned()]),
                (ForwardKind::Named, vec!["c".to_owned()]),
            ]
        );
        assert_eq!(variant_heads(&section), [["syn::Error"]]);
        assert_eq!(section.prose_links, []);
    }

    #[test]
    fn forwards_name_every_possessive_target_and_ignore_except_clauses() {
        let text = "# Errors\n\n* Forwards [`a`] and [`b`]'s return values on failure.\n* \
                    Forwards [`c`]'s return values on failure, except \
                    [`ClientError::InvalidJobState`]'s.";
        assert_eq!(
            forward_paths(&section(text)),
            [
                (ForwardKind::Named, vec!["a".to_owned(), "b".to_owned()]),
                (ForwardKind::Named, vec!["c".to_owned()]),
            ]
        );
    }

    #[test]
    fn forward_typos_are_named_forwards_with_a_format_finding() {
        let text = "# Errors\n\n* Forward [`x`]'s return values.\n* Forwards [`y`]' return values.";
        let section = section(text);
        assert_eq!(
            forward_paths(&section),
            [
                (ForwardKind::Named, vec!["x".to_owned()]),
                (ForwardKind::Named, vec!["y".to_owned()]),
            ]
        );
        assert_eq!(
            format_kinds(&section),
            [
                (FormatFindingKind::NonCanonicalForward, 3),
                (FormatFindingKind::NonCanonicalForward, 4)
            ]
        );
    }

    #[test]
    fn forwards_bullet_without_a_function_is_unnamed() {
        let section = section("# Errors\n\n* Forwards [`sqlx::Error`] on DB failure.");
        assert_eq!(
            forward_paths(&section),
            [(ForwardKind::Unnamed, vec!["sqlx::Error".to_owned()])]
        );
        assert_eq!(
            format_kinds(&section),
            [(
                FormatFindingKind::UnnamedForward {
                    code_names: Vec::new()
                },
                3
            )]
        );
    }

    #[test]
    fn references_resolve_through_inline_links_definitions_and_bare_code() {
        let text = "# Errors\n\n* [`Error::A`](crate::Error::A) if a.\n* [the error][err] if \
                    b.\n* Forwards `tx`'s return values on failure.\n\n# \
                    Panics\n\nNever.\n\n[err]: crate::Error::B";
        let section = section(text);
        assert_eq!(variant_heads(&section), [["Error::A"], ["Error::B"]]);
        assert_eq!(
            forward_paths(&section),
            [(ForwardKind::Named, vec!["tx".to_owned()])]
        );
    }

    #[test]
    fn variant_head_holds_every_link_before_the_condition() {
        let text = "# Errors\n\n* [`CacheError`] converted from [`DbError::Corrupted`] if [`x`] \
                    fails.\n* Returns an [`anyhow::Error`] if it fails.\n* [`A`] or [`B`]:";
        assert_eq!(
            variant_heads(&section(text)),
            [
                vec!["CacheError".to_owned(), "DbError::Corrupted".to_owned()],
                vec!["anyhow::Error".to_owned()],
                vec!["A".to_owned(), "B".to_owned()],
            ]
        );
    }

    #[test]
    fn bare_code_bullets_and_prose_are_not_variants() {
        let text = "# Errors\n\nReturns [`ClientError::InvalidInput`] if:\n\n* `limit <= 0`.\n* \
                    The [`Job`] is missing.";
        let section = section(text);
        assert_eq!(section.variant_bullets, []);
        assert_eq!(
            strings(&section.prose_links),
            ["ClientError::InvalidInput", "Job"]
        );
    }

    #[test]
    fn forward_wording_after_the_possessive_must_be_canonical() {
        let text = "# Errors\n\nReturns an error if:\n\n* Forwards [`a`]'s return values on \
                    failure, except [`E::X`].\n* Forwards [`b`]'s return values on failure if:\n  \
                    * It times out.\n* Forwards [`c`]'s status on failure.\n* Forwards [`d`]'s \
                    return values.\n* Forwards [`e`]'s return values of failure.";
        let section = section(text);
        assert_eq!(
            format_kinds(&section),
            [
                (FormatFindingKind::NonCanonicalForward, 8),
                (FormatFindingKind::NonCanonicalForward, 9),
                (FormatFindingKind::NonCanonicalForward, 10),
            ]
        );
        assert_eq!(forward_paths(&section).len(), 5);
    }

    #[test]
    fn forward_wording_may_name_the_variant_the_errors_are_mapped_into() {
        let text = "# Errors\n\nReturns an error if:\n\n* Forwards [`a`]'s return values as\n  \
                    [`E::BadRequest`] on failure.\n* Forwards [`b`]'s return values as a bad \
                    request on failure.\n* Forwards [`c`]'s return values as [`E::X`].";
        let section = section(text);
        assert_eq!(
            format_kinds(&section),
            [
                (FormatFindingKind::NonCanonicalForward, 7),
                (FormatFindingKind::NonCanonicalForward, 8),
            ]
        );
        assert_eq!(
            forward_paths(&section),
            [
                (ForwardKind::Named, vec!["a".to_owned()]),
                (ForwardKind::Named, vec!["b".to_owned()]),
                (ForwardKind::Named, vec!["c".to_owned()]),
            ]
        );
    }

    #[test]
    fn named_forwards_of_a_type_get_a_format_finding() {
        let text = "# Errors\n\nReturns an error if:\n\n* Forwards [`tokio::task::JoinHandle`]'s \
                    return values on failure.\n* Forwards [`sqlx::Error`] on failure.";
        let kinds = format_kinds(&section(text));
        let path = SymbolPath::parse("tokio::task::JoinHandle").expect("valid path");
        assert_eq!(
            kinds,
            [
                (FormatFindingKind::ForwardNamesType { path }, 5),
                (
                    FormatFindingKind::UnnamedForward {
                        code_names: Vec::new()
                    },
                    6
                ),
            ]
        );
    }

    #[test]
    fn unnamed_forwards_keep_their_one_segment_code_names() {
        let text =
            "# Errors\n\nReturns an error if:\n\n* Forwards the error from `decode` or `a::b`.";
        assert_eq!(
            format_kinds(&section(text)),
            [(
                FormatFindingKind::UnnamedForward {
                    code_names: vec!["decode".to_owned()]
                },
                5
            )]
        );
    }

    #[test]
    fn intro_must_be_the_canonical_line_followed_by_bullets() {
        let canonical = "# Errors\n\nReturns an error if:\n\n* [`E::A`] if a.";
        assert_eq!(
            intro_finding_lines(&section(canonical)),
            Vec::<usize>::new()
        );
        for (text, line) in [
            ("# Errors\n\n* [`E::A`] if a.", 3),
            ("# Errors\n\nReturns [`E::A`] if a.", 3),
            ("# Errors\n\nReturns [`E::A`] if:\n\n- a.", 3),
            ("# Errors\n\nReturns an error if:", 3),
            ("# Errors", 1),
        ] {
            assert_eq!(intro_finding_lines(&section(text)), [line], "{text:?}");
        }
    }

    #[test]
    fn variant_bullets_keep_the_links_after_their_head() {
        let text = "# Errors\n\nReturns an error if:\n\n* [`E::Stale`] if [`check`] fails:\n  * \
                    The state is gone (see [`Requirement::ensure_met`]).";
        let section = section(text);
        assert_eq!(
            strings(&section.variant_bullets[0].linked_paths),
            ["check", "Requirement::ensure_met"]
        );
    }
}
