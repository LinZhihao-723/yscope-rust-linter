//! Rule-agnostic helpers over docstrings: the doc lines of `#[doc]` attributes, the references of
//! their inline Markdown, their `# Title` sections, and the sections that document parameters one
//! bullet each.

pub mod doc_lines;
pub mod links;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the stubs of type-params-doc and trait-params-doc use it once they are \
                  implemented"
    )
)]
pub mod param_section;
pub mod section;
