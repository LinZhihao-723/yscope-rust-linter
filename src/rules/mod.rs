//! The lint rules.

pub mod errors_doc;
pub mod greedy_wrap;
mod registry;

pub use registry::Rule;
pub use registry::RuleId;
