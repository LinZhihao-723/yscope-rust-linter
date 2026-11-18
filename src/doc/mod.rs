//! Rule-agnostic helpers over docstrings: the doc lines of `#[doc]` attributes, the references of
//! their inline Markdown, their `# Title` sections, and the sections that document parameters one
//! bullet each.

pub mod doc_lines;
pub mod links;
pub mod param_section;
pub mod section;
