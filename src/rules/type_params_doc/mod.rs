//! The `type-params-doc` rule: the `# Type Parameters` section of an item's docstring documents
//! each of its generic parameters, and nothing else.

mod check;

pub use check::Checker;
