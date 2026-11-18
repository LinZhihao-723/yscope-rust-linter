//! The `symbol-order` rule: the items of each module and inherent impl block follow the symbol
//! ordering convention.

mod check;
mod keys;
mod moves;
mod scope;

pub use check::Checker;
