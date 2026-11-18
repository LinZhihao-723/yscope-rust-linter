//! The `trait-params-doc` rule: the `# Parameters` section of a trait method declaration's
//! docstring documents each of its parameters, and no other item has one.

mod check;

pub use check::Checker;
