//! The counted items of a module or an inherent impl block, with their sort keys.

use std::collections::HashMap;

use quote::ToTokens;

use crate::diagnostic::Position;
use crate::rules::symbol_order::keys::Category;
use crate::rules::symbol_order::keys::ImplKey;
use crate::rules::symbol_order::keys::ImplRank;
use crate::rules::symbol_order::keys::ModuleKey;
use crate::syntax::crate_names::CrateNames;
use crate::syntax::item_lines;
use crate::syntax::item_lines::ItemLines;
use crate::syntax::test_code;
use crate::syntax::types;
use crate::syntax::visibility::VisibilityGroup;

/// A counted item of a scope.
///
/// # Type Parameters
///
/// * `KeyType` - The type of the scope's sort keys.
#[derive(Debug)]
pub struct Entry<KeyType> {
    pub key: KeyType,

    /// The item as diagnostics describe it, such as ``pub fn `load` `` or ``impl `State` ``.
    pub description: String,

    /// The item's first token after its outer attributes.
    pub position: Position,

    pub lines: ItemLines,
}

/// Collects the counted items of the scopes of one file.
///
/// # Type Parameters
///
/// * `'file` - The lifetime of the file's crate-local names.
#[derive(Debug)]
pub struct EntryCollector<'file> {
    pub crate_names: &'file CrateNames,

    /// Whether `#[cfg(test)]` items and `#[test]` fns are counted rather than exempt.
    pub include_tests: bool,
}

impl EntryCollector<'_> {
    /// Collects the counted items of a module whose items are `items`. The exempt items are left
    /// out: `use`, `extern crate`, modules, macros, `extern` blocks, verbatim items, skipped test
    /// code, and impl blocks whose self type isn't a struct, enum or union defined in `items`.
    ///
    /// # Returns
    ///
    /// The counted items, in source order.
    pub fn module_entries(&self, items: &[syn::Item]) -> Vec<Entry<ModuleKey>> {
        let types = self.type_definitions(items);
        items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| self.module_entry(item, index, &types))
            .collect()
    }

    /// Collects the counted items of the inherent impl block `item_impl`: its associated consts and
    /// fns, without skipped test code.
    ///
    /// # Returns
    ///
    /// The counted items, in source order.
    pub fn impl_entries(&self, item_impl: &syn::ItemImpl) -> Vec<Entry<ImplKey>> {
        item_impl
            .items
            .iter()
            .filter_map(|impl_item| {
                let (attrs, key, description, is_fn) = match impl_item {
                    syn::ImplItem::Const(impl_item_const) => {
                        let group = VisibilityGroup::of(&impl_item_const.vis);
                        (
                            &impl_item_const.attrs,
                            ImplKey::associated_const(),
                            describe(group, "const", &impl_item_const.ident),
                            false,
                        )
                    }
                    syn::ImplItem::Fn(impl_item_fn) => {
                        let group = VisibilityGroup::of(&impl_item_fn.vis);
                        (
                            &impl_item_fn.attrs,
                            ImplKey::associated_fn(group),
                            describe(group, "fn", &impl_item_fn.sig.ident),
                            true,
                        )
                    }
                    _ => return None,
                };
                if self.is_skipped(attrs, is_fn) {
                    return None;
                }
                entry(impl_item, key, description)
            })
            .collect()
    }

    /// # Returns
    ///
    /// Whether an item with `attrs` is test code that isn't counted or checked: an item with
    /// `#[cfg(test)]`, or a fn with a `#[test]`-like attribute, unless tests are included.
    pub fn is_skipped(&self, attrs: &[syn::Attribute], is_fn: bool) -> bool {
        !self.include_tests && test_code::is_test_item(attrs, is_fn)
    }

    /// Finds the structs, enums and unions of a module that impl blocks can belong to.
    ///
    /// # Returns
    ///
    /// The module's counted structs, enums and unions by name, each with its index in `items` and
    /// its visibility group, in source order.
    fn type_definitions(&self, items: &[syn::Item]) -> HashMap<String, Vec<TypeDefinition>> {
        let mut types: HashMap<String, Vec<TypeDefinition>> = HashMap::new();
        for (index, item) in items.iter().enumerate() {
            let (attrs, vis, ident) = match item {
                syn::Item::Struct(item_struct) => {
                    (&item_struct.attrs, &item_struct.vis, &item_struct.ident)
                }
                syn::Item::Enum(item_enum) => (&item_enum.attrs, &item_enum.vis, &item_enum.ident),
                syn::Item::Union(item_union) => {
                    (&item_union.attrs, &item_union.vis, &item_union.ident)
                }
                _ => continue,
            };
            if self.is_skipped(attrs, false) {
                continue;
            }
            types
                .entry(ident.to_string())
                .or_default()
                .push(TypeDefinition {
                    index,
                    group: VisibilityGroup::of(vis),
                });
        }
        types
    }

    /// Builds the entry of the module item `item`, at `index` in its module, whose module defines
    /// the structs, enums and unions `types`.
    ///
    /// # Returns
    ///
    /// The item's entry, or [`None`] if the item is exempt.
    fn module_entry(
        &self,
        item: &syn::Item,
        index: usize,
        types: &HashMap<String, Vec<TypeDefinition>>,
    ) -> Option<Entry<ModuleKey>> {
        let type_category = Category::Type {
            anchor: index,
            impl_rank: ImplRank::Type,
        };
        let (attrs, vis, category, noun, ident) = match item {
            syn::Item::Const(item_const) => (
                &item_const.attrs,
                &item_const.vis,
                Category::Const,
                "const",
                &item_const.ident,
            ),
            syn::Item::Static(item_static) => (
                &item_static.attrs,
                &item_static.vis,
                Category::Static,
                "static",
                &item_static.ident,
            ),
            syn::Item::Struct(item_struct) => (
                &item_struct.attrs,
                &item_struct.vis,
                type_category,
                "struct",
                &item_struct.ident,
            ),
            syn::Item::Enum(item_enum) => (
                &item_enum.attrs,
                &item_enum.vis,
                type_category,
                "enum",
                &item_enum.ident,
            ),
            syn::Item::Union(item_union) => (
                &item_union.attrs,
                &item_union.vis,
                type_category,
                "union",
                &item_union.ident,
            ),
            syn::Item::Trait(item_trait) => (
                &item_trait.attrs,
                &item_trait.vis,
                type_category,
                "trait",
                &item_trait.ident,
            ),
            syn::Item::TraitAlias(item_trait_alias) => (
                &item_trait_alias.attrs,
                &item_trait_alias.vis,
                type_category,
                "trait alias",
                &item_trait_alias.ident,
            ),
            syn::Item::Type(item_type) => (
                &item_type.attrs,
                &item_type.vis,
                type_category,
                "type",
                &item_type.ident,
            ),
            syn::Item::Fn(item_fn) => (
                &item_fn.attrs,
                &item_fn.vis,
                Category::Fn,
                "fn",
                &item_fn.sig.ident,
            ),
            syn::Item::Impl(item_impl) => return self.impl_block_entry(item_impl, index, types),
            _ => return None,
        };
        if self.is_skipped(attrs, matches!(category, Category::Fn)) {
            return None;
        }
        let group = VisibilityGroup::of(vis);
        let key = ModuleKey { group, category };
        entry(item, key, describe(group, noun, ident))
    }

    /// Builds the entry of the impl block `item_impl`, at `index` in its module, whose module
    /// defines the structs, enums and unions `types`.
    ///
    /// # Returns
    ///
    /// The impl block's entry, or [`None`] if it is skipped test code or its self type isn't one of
    /// `types`.
    fn impl_block_entry(
        &self,
        item_impl: &syn::ItemImpl,
        index: usize,
        types: &HashMap<String, Vec<TypeDefinition>>,
    ) -> Option<Entry<ModuleKey>> {
        if self.is_skipped(&item_impl.attrs, false) {
            return None;
        }
        let syn::Type::Path(type_path) = types::peel_type_parens(&item_impl.self_ty) else {
            return None;
        };
        if type_path.qself.is_some()
            || type_path.path.leading_colon.is_some()
            || type_path.path.segments.len() != 1
        {
            return None;
        }
        let type_name = type_path.path.segments.last()?.ident.to_string();
        let definitions = types.get(&type_name)?;
        let definition = definitions
            .iter()
            .rev()
            .find(|definition| definition.index < index)
            .or_else(|| definitions.first())?;
        let (impl_rank, description) = match &item_impl.trait_ {
            None => (ImplRank::Inherent, format!("impl `{type_name}`")),
            Some((trait_path, _)) => {
                let impl_rank = if self.crate_names.is_crate_path(trait_path) {
                    ImplRank::CrateTrait
                } else {
                    ImplRank::ExternalTrait
                };
                let polarity = if item_impl.modifiers.polarity.is_some() {
                    "!"
                } else {
                    ""
                };
                let trait_name = trait_path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::");
                (
                    impl_rank,
                    format!("impl `{polarity}{trait_name} for {type_name}`"),
                )
            }
        };
        let key = ModuleKey {
            group: definition.group,
            category: Category::Type {
                anchor: definition.index,
                impl_rank,
            },
        };
        entry(item_impl, key, description)
    }
}

/// A struct, enum or union that impl blocks of its module can belong to.
#[derive(Debug)]
struct TypeDefinition {
    /// The index of the type in its module's items.
    index: usize,

    group: VisibilityGroup,
}

/// Builds the entry of `item` from its tokens' lines and position.
///
/// # Type Parameters
///
/// * `KeyType` - The type of the scope's sort keys.
///
/// # Returns
///
/// The entry, or [`None`] if `item` has no token after its outer attributes.
fn entry<KeyType>(
    item: &impl ToTokens,
    key: KeyType,
    description: String,
) -> Option<Entry<KeyType>> {
    Some(Entry {
        key,
        description,
        position: item_lines::own_start(item)?,
        lines: item_lines::of(item)?,
    })
}

/// # Returns
///
/// The description of an item of the visibility group `group`, the kind `noun` and the name
/// `ident`, such as ``pub(crate) fn `load` ``.
fn describe(group: VisibilityGroup, noun: &str, ident: &syn::Ident) -> String {
    format!("{} {noun} `{ident}`", group.as_str())
}
