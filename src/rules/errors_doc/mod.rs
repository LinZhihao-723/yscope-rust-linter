//! The `errors-doc` rule: the `# Errors` section of a fn's docstring matches the errors its body
//! returns and forwards.

mod body;
mod check;
mod errors_section;
mod file_index;
mod items;
mod known_apis;
mod macros;
mod pass_through;
mod symbol;

pub use check::Checker;
