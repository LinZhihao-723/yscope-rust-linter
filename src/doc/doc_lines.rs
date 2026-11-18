//! Turning an item's `#[doc]` attributes into positioned doc lines.

use crate::diagnostic::Position;
use crate::syntax::span;

/// One line of a docstring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocLine {
    /// The line's text, with one leading space removed.
    pub text: String,

    /// The line and the column of the attribute (for `///`, the column of the first `/`).
    pub position: Position,
}

/// Collects the doc lines of `attrs`.
///
/// * Every outer `#[doc = "..."]` attribute whose value is a string literal is split on `\n`. A
///   `///` comment is one such attribute with one line.
/// * When the literal spans as many source lines as its value has lines, such as a `/** */` block
///   or a literal with real line breaks, each line is numbered by its source line.
/// * Otherwise, such as for an escaped `\n` in `#[doc = "a\nb"]`, every line gets the literal's
///   first line.
/// * Attributes with a non-literal value, such as `include_str!`, are skipped.
///
/// # Returns
///
/// The doc lines in source order.
pub fn collect(attrs: &[syn::Attribute]) -> Vec<DocLine> {
    let mut doc_lines = Vec::new();
    for attr in attrs {
        if !matches!(attr.style, syn::AttrStyle::Outer) {
            continue;
        }
        let syn::Meta::NameValue(name_value) = &attr.meta else {
            continue;
        };
        if !name_value.path.is_ident("doc") {
            continue;
        }
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(lit_str),
            ..
        }) = &name_value.value
        else {
            continue;
        };
        let column = span::start_position(attr.pound_token.span).column;
        let literal_span = lit_str.span();
        let first_line = span::start_position(literal_span).line;
        let num_source_lines = span::end_line(literal_span).saturating_sub(first_line) + 1;
        let value = lit_str.value();
        let follows_source_lines = value.split('\n').count() == num_source_lines;
        for (line_offset, raw_line) in value.split('\n').enumerate() {
            let raw_line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            let line = if follows_source_lines {
                first_line + line_offset
            } else {
                first_line
            };
            doc_lines.push(DocLine {
                text: raw_line.strip_prefix(' ').unwrap_or(raw_line).to_owned(),
                position: Position { line, column },
            });
        }
    }
    doc_lines
}

/// Collects the doc lines of `attrs`, as [`collect`] does, if they document the item.
///
/// # Returns
///
/// The doc lines, or [`None`] if none of them is non-blank.
pub fn collect_documented(attrs: &[syn::Attribute]) -> Option<Vec<DocLine>> {
    let doc_lines = collect(attrs);
    doc_lines
        .iter()
        .any(|doc_line| !doc_line.text.trim().is_empty())
        .then_some(doc_lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_lines_of(source_text: &str) -> Vec<(String, usize, usize)> {
        let item_fn: syn::ItemFn = syn::parse_str(source_text).expect("valid fn");
        collect(&item_fn.attrs)
            .into_iter()
            .map(|doc_line| {
                (
                    doc_line.text,
                    doc_line.position.line,
                    doc_line.position.column,
                )
            })
            .collect()
    }

    #[test]
    fn line_comments_and_multi_line_attributes_keep_their_lines() {
        let source_text = "#[inline]\n    /// First.\n    ///\n    ///   Indented.\n#[doc = \
                           \"a\\nb\"]\n#[doc = include_str!(\"x.md\")]\n#[doc = \"c\nd\"]\n/** \
                           e\n f */\nfn f() {}";
        assert_eq!(
            doc_lines_of(source_text),
            [
                ("First.".to_owned(), 2, 5),
                (String::new(), 3, 5),
                ("  Indented.".to_owned(), 4, 5),
                ("a".to_owned(), 5, 1),
                ("b".to_owned(), 5, 1),
                ("c".to_owned(), 7, 1),
                ("d".to_owned(), 8, 1),
                ("e".to_owned(), 9, 1),
                ("f ".to_owned(), 10, 1),
            ]
        );
    }
}
