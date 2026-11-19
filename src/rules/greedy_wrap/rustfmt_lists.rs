//! The comment lines that rustfmt wraps one column early: the lines inside the field lists of
//! structs, unions and enum variants, and inside the parameter lists of fns
//! (rust-lang/rustfmt#6180).

use proc_macro2::extra::DelimSpan;
use syn::visit::Visit;

use crate::line_ranges::LineRanges;
use crate::line_ranges::LineSpan;

/// # Returns
///
/// The lines of `syntax_tree` strictly between the delimiters of each field list and each fn
/// parameter list.
pub fn early_wrap_lines(syntax_tree: &syn::File) -> LineRanges {
    let mut collector = Collector::default();
    collector.visit_file(syntax_tree);
    collector.line_spans.into_iter().collect()
}

/// Collects the lines inside field and parameter lists.
#[derive(Default)]
struct Collector {
    line_spans: Vec<LineSpan>,
}

impl Collector {
    /// Records the lines strictly between the open and close delimiters of `delimiters`, if any.
    fn record(&mut self, delimiters: &DelimSpan) {
        let open_line = delimiters.open().start().line;
        let close_line = delimiters.close().start().line;
        if let Some(line_span) = LineSpan::new(open_line + 1, close_line.saturating_sub(1)) {
            self.line_spans.push(line_span);
        }
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_fields_named(&mut self, fields_named: &'ast syn::FieldsNamed) {
        self.record(&fields_named.brace_token.span);
        syn::visit::visit_fields_named(self, fields_named);
    }

    fn visit_fields_unnamed(&mut self, fields_unnamed: &'ast syn::FieldsUnnamed) {
        self.record(&fields_unnamed.paren_token.span);
        syn::visit::visit_fields_unnamed(self, fields_unnamed);
    }

    fn visit_signature(&mut self, signature: &'ast syn::Signature) {
        self.record(&signature.paren_token.span);
        syn::visit::visit_signature(self, signature);
    }
}
