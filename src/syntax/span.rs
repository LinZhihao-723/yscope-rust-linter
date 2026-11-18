//! Converting `proc-macro2` span locations to diagnostic positions.

use proc_macro2::Span;

use crate::diagnostic::Position;

/// # Returns
///
/// The 1-based position of the first character of `span`. `proc-macro2` columns are 0-based, so 1
/// is added to the column.
pub fn start_position(span: Span) -> Position {
    let start = span.start();
    Position {
        line: start.line.max(1),
        column: start.column + 1,
    }
}

/// # Returns
///
/// The 1-based line of the last character of `span`.
pub fn end_line(span: Span) -> usize {
    span.end().line.max(1)
}
