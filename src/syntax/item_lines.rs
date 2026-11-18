//! The lines of the items in a `syn` syntax tree.

use proc_macro2::Delimiter;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use syn::visit::Visit;

use crate::diagnostic::Position;
use crate::line_ranges::LineSpan;
use crate::syntax::span;

/// The lines of one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemLines {
    /// From the item's first attribute or doc line through its last line.
    pub span: LineSpan,

    /// From the item's first attribute or doc line through the line where the item itself starts,
    /// after its outer attributes.
    pub header: LineSpan,
}

/// Collects the lines of every item of `syntax_tree`, including nested items. These count as items:
///
/// * Fns, impls, traits, inline modules, structs, enums, unions, consts, statics, type aliases and
///   `macro_rules!` definitions.
/// * The fns, consts and types inside impls and traits.
///
/// # Returns
///
/// The lines of the items, in source order.
pub fn collect(syntax_tree: &syn::File) -> Vec<ItemLines> {
    let mut item_collector = ItemCollector {
        item_lines: Vec::new(),
    };
    item_collector.visit_file(syntax_tree);
    item_collector.item_lines
}

/// Finds the lines of one item, such as a `syn::Item`, a `syn::ImplItemFn` or a `syn::TraitItemFn`.
///
/// # Returns
///
/// The lines of `item`, or [`None`] if it has no token after its outer attributes.
pub fn of(item: &impl ToTokens) -> Option<ItemLines> {
    lines_of(item.to_token_stream())
}

/// Finds the first token of `item` after its outer attributes.
///
/// # Returns
///
/// The token's position, or [`None`] if `item` has no token after its attributes.
pub fn own_start(item: &impl ToTokens) -> Option<Position> {
    let token_trees: Vec<TokenTree> = item.to_token_stream().into_iter().collect();
    Some(span::start_position(
        token_trees.get(attrs_end(&token_trees))?.span(),
    ))
}

/// The `syn` visitor that collects [`ItemLines`].
struct ItemCollector {
    item_lines: Vec<ItemLines>,
}

impl ItemCollector {
    /// Records the lines of the item printed as `tokens`.
    fn add(&mut self, tokens: TokenStream) {
        if let Some(item_lines) = lines_of(tokens) {
            self.item_lines.push(item_lines);
        }
    }
}

impl<'ast> Visit<'ast> for ItemCollector {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let is_counted = match item {
            syn::Item::Fn(_)
            | syn::Item::Impl(_)
            | syn::Item::Trait(_)
            | syn::Item::Struct(_)
            | syn::Item::Enum(_)
            | syn::Item::Union(_)
            | syn::Item::Const(_)
            | syn::Item::Static(_)
            | syn::Item::Type(_) => true,
            syn::Item::Mod(item_mod) => item_mod.content.is_some(),
            syn::Item::Macro(item_macro) => item_macro.mac.path.is_ident("macro_rules"),
            _ => false,
        };
        if is_counted {
            self.add(item.to_token_stream());
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, impl_item: &'ast syn::ImplItem) {
        if matches!(
            impl_item,
            syn::ImplItem::Fn(_) | syn::ImplItem::Const(_) | syn::ImplItem::Type(_)
        ) {
            self.add(impl_item.to_token_stream());
        }
        syn::visit::visit_impl_item(self, impl_item);
    }

    fn visit_trait_item(&mut self, trait_item: &'ast syn::TraitItem) {
        if matches!(
            trait_item,
            syn::TraitItem::Fn(_) | syn::TraitItem::Const(_) | syn::TraitItem::Type(_)
        ) {
            self.add(trait_item.to_token_stream());
        }
        syn::visit::visit_trait_item(self, trait_item);
    }
}

/// Finds the lines of an item from its tokens. Its outer attributes, including doc comments, are
/// the leading `#` and `[..]` pairs.
///
/// # Returns
///
/// The item's lines, or [`None`] if it has no token after its attributes.
fn lines_of(tokens: TokenStream) -> Option<ItemLines> {
    let token_trees: Vec<TokenTree> = tokens.into_iter().collect();
    let first_line = span::start_position(token_trees.first()?.span()).line;
    let own_line = span::start_position(token_trees.get(attrs_end(&token_trees))?.span()).line;
    let last_line = span::end_line(token_trees.last()?.span());
    Some(ItemLines {
        span: LineSpan::new(first_line, last_line)?,
        header: LineSpan::new(first_line, own_line)?,
    })
}

/// # Returns
///
/// The index of the first of `token_trees` after the leading `#` and `[..]` pairs of the outer
/// attributes.
fn attrs_end(token_trees: &[TokenTree]) -> usize {
    let mut own_start = 0;
    while let (Some(TokenTree::Punct(punct)), Some(TokenTree::Group(group))) =
        (token_trees.get(own_start), token_trees.get(own_start + 1))
        && punct.as_char() == '#'
        && group.delimiter() == Delimiter::Bracket
    {
        own_start += 2;
    }
    own_start
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(source_text: &str) -> Vec<(String, String)> {
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        collect(&syntax_tree)
            .into_iter()
            .map(|item_lines| (item_lines.span.to_string(), item_lines.header.to_string()))
            .collect()
    }

    #[test]
    fn items_of_every_counted_kind_are_collected_with_nested_ones() {
        let source_text = concat!(
            "use std::fmt;\n",
            "mod declared;\n",
            "mod inline { struct S; }\n",
            "enum E { A }\n",
            "union U { a: u8 }\n",
            "const C: u8 = 0;\n",
            "static V: u8 = 0;\n",
            "type T = u8;\n",
            "macro_rules! m { () => {} }\n",
            "other! {}\n",
            "trait Tr { fn f(); const K: u8; type A; }\n",
            "impl Tr for S { fn f() { fn nested() {} } const K: u8 = 0; type A = u8; }\n",
        );
        assert_eq!(
            spans(source_text)
                .into_iter()
                .map(|(span, _)| span)
                .collect::<Vec<_>>(),
            [
                "3", "3", "4", "5", "6", "7", "8", "9", "11", "11", "11", "11", "12", "12", "12",
                "12", "12"
            ]
        );
    }

    #[test]
    fn lines_of_one_item_match_the_collected_lines() {
        let source_text = "/// Doc.\n#[inline]\npub fn f() {\n}\n";
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        let item_lines = of(&syntax_tree.items[0]).expect("an item with tokens");
        assert_eq!(collect(&syntax_tree), [item_lines]);
        assert_eq!(
            (item_lines.span.to_string(), item_lines.header.to_string()),
            ("1-4".to_owned(), "1-3".to_owned())
        );
    }

    #[test]
    fn spans_start_at_the_first_attribute_or_doc_line_and_headers_end_at_the_item() {
        let source_text = concat!(
            "/// Doc.\n",
            "#[derive(Debug)]\n",
            "pub struct S {\n",
            "    a: u8,\n",
            "}\n",
            "\n",
            "fn f() {}\n",
        );
        assert_eq!(
            spans(source_text),
            [
                ("1-5".to_owned(), "1-3".to_owned()),
                ("7".to_owned(), "7".to_owned())
            ]
        );
    }
}
