//! The inline Markdown of docstrings: references to paths in code spans and links, and the link
//! reference definitions that resolve them.

use std::collections::HashMap;
use std::collections::HashSet;

use crate::markdown;
use crate::markdown::FenceTracker;

/// Where a reference's path comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// A link: an intra-doc shortcut, an inline link or a reference link.
    Link,

    /// A code span outside any link, such as `` `tx` ``.
    BareCode,
}

/// A reference found by the inline scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub kind: ReferenceKind,

    /// The text of the path the reference names.
    pub path_text: String,

    /// The byte offset of the reference's first character.
    pub start: usize,

    /// The byte offset just past the reference's last character.
    pub end: usize,
}

/// The link reference definitions of one docstring, such as ``[`Foo`]: crate::foo::Foo``.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkDefinitions {
    targets: HashMap<String, String>,

    /// The indices of the doc lines that belong to a definition, including target continuation
    /// lines.
    line_indices: HashSet<usize>,
}

impl LinkDefinitions {
    /// Collects every link reference definition outside fenced code blocks.
    ///
    /// A definition is a `[label]: target` line indented by at most 3 spaces. An empty target is
    /// taken from the following lines that are indented deeper than the definition.
    ///
    /// # Returns
    ///
    /// The definitions of `lines`.
    pub fn collect(lines: &[&str]) -> Self {
        let mut definitions = Self::default();
        let mut fence_tracker = FenceTracker::default();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            index += 1;
            if fence_tracker.is_code(line) {
                continue;
            }
            let Some(definition_line) = markdown::link_definition(line) else {
                continue;
            };
            let definition_indent = markdown::indent(line);
            definitions.line_indices.insert(index - 1);
            let mut target_parts: Vec<&str> = (!definition_line.target.is_empty())
                .then_some(definition_line.target)
                .into_iter()
                .collect();
            while let Some(next_line) = lines.get(index)
                && !next_line.trim().is_empty()
                && markdown::indent(next_line) > definition_indent
                && markdown::link_definition(next_line).is_none()
            {
                target_parts.push(next_line.trim());
                definitions.line_indices.insert(index);
                index += 1;
            }
            definitions
                .targets
                .entry(normalize_label(definition_line.label))
                .or_insert_with(|| target_parts.concat());
        }
        definitions
    }

    /// # Returns
    ///
    /// The target of the definition labeled `label`, if any.
    pub fn target(&self, label: &str) -> Option<&str> {
        self.targets
            .get(&normalize_label(label))
            .map(String::as_str)
    }

    /// # Returns
    ///
    /// Whether the doc line at `index` is part of a definition.
    pub fn contains_line(&self, index: usize) -> bool {
        self.line_indices.contains(&index)
    }
}

/// Finds, in order, the references in `text`:
///
/// * `` [`code`] ``, `` [`code`](target) `` and `` [`code`][label] `` name the code text.
/// * `[text](target)` names the target.
/// * `[text][label]` and `[text]` name the target of the definition of `label` or `text`.
/// * `[text]` without a definition names `text` when it looks like a path.
/// * A code span outside any link names its code text, as a [`ReferenceKind::BareCode`].
///
/// # Returns
///
/// The references of `text`.
pub fn references(text: &str, definitions: &LinkDefinitions) -> Vec<Reference> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => match code_span(text, index) {
                Some(code_span) => {
                    found.push(Reference {
                        kind: ReferenceKind::BareCode,
                        path_text: code_span.code.to_owned(),
                        start: index,
                        end: code_span.end,
                    });
                    index = code_span.end;
                }
                None => {
                    index += bytes[index..]
                        .iter()
                        .take_while(|byte| **byte == b'`')
                        .count();
                }
            },
            b'[' => match link(text, index, definitions) {
                Some(link) => {
                    if let Some(path_text) = link.path_text {
                        found.push(Reference {
                            kind: ReferenceKind::Link,
                            path_text,
                            start: index,
                            end: link.end,
                        });
                    }
                    index = link.end;
                }
                None => index += 1,
            },
            _ => index += 1,
        }
    }
    found
}

/// A code span found at a backtick run.
///
/// # Type Parameters
///
/// * `'text` - The lifetime of the text the code span is in.
struct CodeSpan<'text> {
    code: &'text str,
    end: usize,
}

/// A link found at a `[`.
struct Link {
    /// The path the link names, if it names one.
    path_text: Option<String>,
    end: usize,
}

/// Parses the code span that starts with the backtick run at `start`.
///
/// # Returns
///
/// The code span, or [`None`] if the run has no closing run of the same length.
fn code_span(text: &str, start: usize) -> Option<CodeSpan<'_>> {
    let bytes = text.as_bytes();
    let run_len = bytes[start..]
        .iter()
        .take_while(|byte| **byte == b'`')
        .count();
    let mut index = start + run_len;
    while index < bytes.len() {
        if bytes[index] == b'`' {
            let closing_len = bytes[index..]
                .iter()
                .take_while(|byte| **byte == b'`')
                .count();
            if closing_len == run_len {
                return Some(CodeSpan {
                    code: text[start + run_len..index].trim(),
                    end: index + closing_len,
                });
            }
            index += closing_len;
        } else {
            index += 1;
        }
    }
    None
}

/// Parses the link whose label starts with the `[` at `start`.
///
/// # Returns
///
/// The link, or [`None`] if the label has no closing `]`.
fn link(text: &str, start: usize, definitions: &LinkDefinitions) -> Option<Link> {
    let label_end = closing_bracket(text, start, b'[', b']')?;
    let label = &text[start + 1..label_end];
    let after_label = label_end + 1;
    let label_code = single_code_span(label);
    let rest = &text[after_label..];

    let (target, end) = if rest.starts_with('(') {
        let close = closing_bracket(text, after_label, b'(', b')')?;
        let inner = text[after_label + 1..close].trim();
        let target = inner.split_whitespace().next().unwrap_or("");
        let target = target
            .strip_prefix('<')
            .and_then(|target| target.strip_suffix('>'))
            .unwrap_or(target);
        (Some(target.to_owned()), close + 1)
    } else if rest.starts_with('[') {
        let close = closing_bracket(text, after_label, b'[', b']')?;
        let reference_label = &text[after_label + 1..close];
        let reference_label = if reference_label.trim().is_empty() {
            label
        } else {
            reference_label
        };
        (
            definitions.target(reference_label).map(str::to_owned),
            close + 1,
        )
    } else {
        let target = definitions.target(label).map(str::to_owned).or_else(|| {
            (label_code.is_none() && looks_like_path(label)).then(|| label.trim().to_owned())
        });
        (target, after_label)
    };

    Some(Link {
        path_text: label_code.map(str::to_owned).or(target),
        end,
    })
}

/// # Returns
///
/// The index of the bracket that closes the one at `start`, skipping nested pairs and code spans,
/// or [`None`] if there is none.
fn closing_bracket(text: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0_usize;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' {
            index += 2;
            continue;
        }
        if byte == b'`'
            && let Some(code_span) = code_span(text, index)
        {
            index = code_span.end;
            continue;
        }
        if byte == open {
            depth += 1;
        } else if byte == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

/// # Returns
///
/// The code of `label` if the whole label is one code span, such as `` `Foo::bar` ``.
fn single_code_span(label: &str) -> Option<&str> {
    let trimmed = label.trim();
    if !trimmed.starts_with('`') {
        return None;
    }
    let code_span = code_span(trimmed, 0)?;
    (code_span.end == trimmed.len()).then_some(code_span.code)
}

/// # Returns
///
/// Whether `label` looks like a Rust path: identifier characters and `::` only, with at least one
/// `::` or an uppercase first letter.
fn looks_like_path(label: &str) -> bool {
    let label = label.trim();
    !label.is_empty()
        && label
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_' || character == ':')
        && (label.contains("::") || label.starts_with(char::is_uppercase))
}

/// # Returns
///
/// `label` trimmed, lowercased and with whitespace runs collapsed, as rustdoc compares labels.
fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(text: &str, definitions: &LinkDefinitions) -> Vec<(ReferenceKind, String)> {
        references(text, definitions)
            .into_iter()
            .map(|reference| (reference.kind, reference.path_text))
            .collect()
    }

    #[test]
    fn inline_links_reference_links_and_bare_code_resolve() {
        let lines = [
            "[`Pool::begin`]: sqlx::Pool::begin",
            "[pool]:",
            "    sqlx::Pool::acquire",
        ];
        let definitions = LinkDefinitions::collect(&lines);
        assert!(definitions.contains_line(2));
        let text = "[`Error::A`] or [`x`](crate::X) or [the pool][pool] or [docs](https://a.b) \
                    and [Error::B] with `tx` or [`Pool::begin`][] or [unknown text]";
        assert_eq!(
            paths(text, &definitions),
            [
                (ReferenceKind::Link, "Error::A".to_owned()),
                (ReferenceKind::Link, "x".to_owned()),
                (ReferenceKind::Link, "sqlx::Pool::acquire".to_owned()),
                (ReferenceKind::Link, "https://a.b".to_owned()),
                (ReferenceKind::Link, "Error::B".to_owned()),
                (ReferenceKind::BareCode, "tx".to_owned()),
                (ReferenceKind::Link, "Pool::begin".to_owned()),
            ]
        );
    }

    #[test]
    fn reference_offsets_cover_the_whole_link() {
        let text = "Forwards [`foo`]'s return values";
        let found = references(text, &LinkDefinitions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].start..found[0].end], "[`foo`]");
        assert!(text[found[0].end..].starts_with("'s"));
    }

    #[test]
    fn definitions_inside_fences_are_ignored() {
        let lines = ["```", "[a]: b", "```", "[c]: d"];
        let definitions = LinkDefinitions::collect(&lines);
        assert_eq!(definitions.target("a"), None);
        assert_eq!(definitions.target("C"), Some("d"));
    }
}
