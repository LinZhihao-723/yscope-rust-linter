//! The `comment-greedy-wrap` rule: every `//`, `///` and `//!` comment paragraph is filled greedily
//! to the configured width.

mod check;
mod line_kind;
mod paragraph;
mod rustfmt_lists;

pub use check::Checker;
