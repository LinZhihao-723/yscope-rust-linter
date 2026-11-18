//! Rule-agnostic helpers over `syn` syntax trees: span positions, item lines, symbol paths, types,
//! test code, visibility groups, crate-local names and the items that document their signatures.

pub mod crate_names;
pub mod doc_items;
pub mod item_lines;
pub mod span;
pub mod symbol_path;
pub mod test_code;
pub mod types;
pub mod visibility;
