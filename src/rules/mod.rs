//! The lint rules.

pub mod errors_doc;
pub mod greedy_wrap;
mod registry;
pub mod symbol_order;
pub mod trait_params_doc;
pub mod type_params_doc;

pub use registry::Rule;
pub use registry::RuleId;
