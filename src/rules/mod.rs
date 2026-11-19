//! The lint rules.

mod errors_doc;
mod greedy_wrap;
mod registry;
mod symbol_order;
mod trait_params_doc;
mod type_params_doc;

pub use registry::Code;
pub use registry::CodeSet;
pub use registry::InvalidRuleSelector;
pub use registry::Rule;
pub use registry::RuleId;
pub use registry::RuleSelector;
