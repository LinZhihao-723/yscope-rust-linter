//! Rule-agnostic helpers over `syn` syntax trees: span positions, item lines, symbol paths, types,
//! test code, visibility groups, crate-local names and the items that document their signatures.

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the stub of symbol-order uses it once the rule is implemented"
    )
)]
pub mod crate_names;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the stubs of type-params-doc and trait-params-doc use it once they are \
                  implemented"
    )
)]
pub mod doc_items;
pub mod item_lines;
pub mod span;
pub mod symbol_path;
pub mod test_code;
pub mod types;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the stub of symbol-order uses it once the rule is implemented"
    )
)]
pub mod visibility;
