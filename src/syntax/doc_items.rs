//! The items whose docstrings document their signatures: fns of every kind, structs, enums, unions,
//! traits and type aliases.

use syn::visit::Visit;

use crate::syntax::item_lines;
use crate::syntax::item_lines::ItemLines;
use crate::syntax::test_code;

/// An item whose docstring may document its generic or fn parameters.
///
/// # Type Parameters
///
/// * `'ast` - The lifetime of the syntax tree the item is in.
#[derive(Clone)]
pub struct DocItem<'ast> {
    pub kind: DocItemKind,
    pub name: String,

    /// The item's outer attributes, including its doc comments.
    pub attrs: &'ast [syn::Attribute],

    /// The item's own generic parameters. An impl method's generics don't include the impl's.
    pub generics: &'ast syn::Generics,

    /// The signature of a fn, or [`None`] for other items.
    pub sig: Option<&'ast syn::Signature>,

    /// The item's span and header, as [`item_lines::of`] finds them.
    pub lines: ItemLines,
}

impl DocItem<'_> {
    /// # Returns
    ///
    /// The item as diagnostics name it, such as ``fn `load` `` or ``trait method `submit` ``.
    pub fn description(&self) -> String {
        format!("{} `{}`", self.kind.noun(), self.name)
    }
}

/// The kinds of [`DocItem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocItemKind {
    /// A fn outside any impl or trait, including a fn nested in another fn's body.
    FreeFn,

    /// A fn in an impl block without a trait.
    InherentMethod,

    /// A fn in an `impl Trait for Type` block.
    TraitImplMethod,

    /// A fn declared in a `trait` block, with or without a default body.
    TraitMethod,

    Struct,
    Enum,
    Union,
    Trait,
    TypeAlias,
}

impl DocItemKind {
    /// # Returns
    ///
    /// The noun that diagnostics name an item of this kind with, such as `fn` or `trait method`.
    pub const fn noun(self) -> &'static str {
        match self {
            Self::FreeFn => "fn",
            Self::InherentMethod => "method",
            Self::TraitImplMethod => "trait impl method",
            Self::TraitMethod => "trait method",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Union => "union",
            Self::Trait => "trait",
            Self::TypeAlias => "type alias",
        }
    }
}

/// Collects every [`DocItem`] of `syntax_tree`, documented or not, including the items nested in
/// modules and fn bodies.
///
/// Unless `include_tests` is set, these are skipped:
///
/// * Fns with an attribute whose last path segment is `test`, such as `#[tokio::test]`.
/// * Items with `#[cfg(test)]`, including whole modules and impl blocks, with everything in them.
///
/// # Returns
///
/// The collected items, in source order.
pub fn collect(syntax_tree: &syn::File, include_tests: bool) -> Vec<DocItem<'_>> {
    let mut item_walker = ItemWalker {
        include_tests,
        impl_stack: Vec::new(),
        doc_items: Vec::new(),
    };
    item_walker.visit_file(syntax_tree);
    item_walker.doc_items
}

/// The `syn` visitor that collects [`DocItem`]s.
///
/// # Type Parameters
///
/// * `'ast` - The lifetime of the syntax tree being walked.
struct ItemWalker<'ast> {
    include_tests: bool,

    /// Whether each impl block the walk is in implements a trait, innermost last.
    impl_stack: Vec<bool>,

    doc_items: Vec<DocItem<'ast>>,
}

impl<'ast> ItemWalker<'ast> {
    /// # Returns
    ///
    /// Whether an item with `attrs` is test code that the walk skips.
    fn is_skipped(&self, attrs: &[syn::Attribute], is_fn: bool) -> bool {
        !self.include_tests
            && (test_code::has_cfg_test(attrs)
                || (is_fn && attrs.iter().any(test_code::is_test_attribute)))
    }

    /// Records the item printed by `item` unless it has no tokens after its attributes.
    fn add(
        &mut self,
        kind: DocItemKind,
        ident: &syn::Ident,
        attrs: &'ast [syn::Attribute],
        generics: &'ast syn::Generics,
        sig: Option<&'ast syn::Signature>,
        item: &impl quote::ToTokens,
    ) {
        if let Some(lines) = item_lines::of(item) {
            self.doc_items.push(DocItem {
                kind,
                name: ident.to_string(),
                attrs,
                generics,
                sig,
                lines,
            });
        }
    }
}

impl<'ast> Visit<'ast> for ItemWalker<'ast> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let (attrs, is_fn) = match item {
            syn::Item::Fn(item_fn) => (&item_fn.attrs, true),
            syn::Item::Struct(item_struct) => (&item_struct.attrs, false),
            syn::Item::Enum(item_enum) => (&item_enum.attrs, false),
            syn::Item::Union(item_union) => (&item_union.attrs, false),
            syn::Item::Trait(item_trait) => (&item_trait.attrs, false),
            syn::Item::Type(item_type) => (&item_type.attrs, false),
            syn::Item::Impl(item_impl) => (&item_impl.attrs, false),
            syn::Item::Mod(item_mod) => (&item_mod.attrs, false),
            _ => {
                syn::visit::visit_item(self, item);
                return;
            }
        };
        if self.is_skipped(attrs, is_fn) {
            return;
        }
        match item {
            syn::Item::Fn(item_fn) => self.add(
                DocItemKind::FreeFn,
                &item_fn.sig.ident,
                &item_fn.attrs,
                &item_fn.sig.generics,
                Some(&item_fn.sig),
                item_fn,
            ),
            syn::Item::Struct(item_struct) => self.add(
                DocItemKind::Struct,
                &item_struct.ident,
                &item_struct.attrs,
                &item_struct.generics,
                None,
                item_struct,
            ),
            syn::Item::Enum(item_enum) => self.add(
                DocItemKind::Enum,
                &item_enum.ident,
                &item_enum.attrs,
                &item_enum.generics,
                None,
                item_enum,
            ),
            syn::Item::Union(item_union) => self.add(
                DocItemKind::Union,
                &item_union.ident,
                &item_union.attrs,
                &item_union.generics,
                None,
                item_union,
            ),
            syn::Item::Trait(item_trait) => self.add(
                DocItemKind::Trait,
                &item_trait.ident,
                &item_trait.attrs,
                &item_trait.generics,
                None,
                item_trait,
            ),
            syn::Item::Type(item_type) => self.add(
                DocItemKind::TypeAlias,
                &item_type.ident,
                &item_type.attrs,
                &item_type.generics,
                None,
                item_type,
            ),
            _ => {}
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_item_impl(&mut self, item_impl: &'ast syn::ItemImpl) {
        self.impl_stack.push(item_impl.trait_.is_some());
        syn::visit::visit_item_impl(self, item_impl);
        self.impl_stack.pop();
    }

    fn visit_impl_item_fn(&mut self, impl_item_fn: &'ast syn::ImplItemFn) {
        if self.is_skipped(&impl_item_fn.attrs, true) {
            return;
        }
        let kind = if self.impl_stack.last().copied().unwrap_or_default() {
            DocItemKind::TraitImplMethod
        } else {
            DocItemKind::InherentMethod
        };
        self.add(
            kind,
            &impl_item_fn.sig.ident,
            &impl_item_fn.attrs,
            &impl_item_fn.sig.generics,
            Some(&impl_item_fn.sig),
            impl_item_fn,
        );
        syn::visit::visit_impl_item_fn(self, impl_item_fn);
    }

    fn visit_trait_item_fn(&mut self, trait_item_fn: &'ast syn::TraitItemFn) {
        if self.is_skipped(&trait_item_fn.attrs, true) {
            return;
        }
        self.add(
            DocItemKind::TraitMethod,
            &trait_item_fn.sig.ident,
            &trait_item_fn.attrs,
            &trait_item_fn.sig.generics,
            Some(&trait_item_fn.sig),
            trait_item_fn,
        );
        syn::visit::visit_trait_item_fn(self, trait_item_fn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(source_text: &str, include_tests: bool) -> Vec<(DocItemKind, String)> {
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        collect(&syntax_tree, include_tests)
            .into_iter()
            .map(|doc_item| (doc_item.kind, doc_item.name))
            .collect()
    }

    #[test]
    fn items_of_every_kind_are_collected_with_nested_ones() {
        let source_text = concat!(
            "fn free() { fn nested() {} }\n",
            "struct S;\n",
            "enum E { A }\n",
            "union U { a: u8 }\n",
            "trait T { fn declared(); fn provided() {} }\n",
            "type Alias = u8;\n",
            "impl S { fn inherent() { impl T for S { fn in_body() {} } } }\n",
            "impl T for S { fn implemented() {} }\n",
            "mod inner { const C: u8 = 0; fn in_mod() {} }\n",
            "use std::fmt;\n",
        );
        assert_eq!(
            items(source_text, false),
            [
                (DocItemKind::FreeFn, "free".to_owned()),
                (DocItemKind::FreeFn, "nested".to_owned()),
                (DocItemKind::Struct, "S".to_owned()),
                (DocItemKind::Enum, "E".to_owned()),
                (DocItemKind::Union, "U".to_owned()),
                (DocItemKind::Trait, "T".to_owned()),
                (DocItemKind::TraitMethod, "declared".to_owned()),
                (DocItemKind::TraitMethod, "provided".to_owned()),
                (DocItemKind::TypeAlias, "Alias".to_owned()),
                (DocItemKind::InherentMethod, "inherent".to_owned()),
                (DocItemKind::TraitImplMethod, "in_body".to_owned()),
                (DocItemKind::TraitImplMethod, "implemented".to_owned()),
                (DocItemKind::FreeFn, "in_mod".to_owned()),
            ]
        );
    }

    #[test]
    fn test_code_is_skipped_unless_included() {
        let source_text = concat!(
            "#[test]\nfn unit_test() {}\n",
            "#[cfg(test)]\nmod tests { fn helper() {} }\n",
            "#[cfg(test)]\nstruct Fixture;\n",
            "impl S {\n    #[tokio::test]\n    async fn method_test() {}\n}\n",
            "fn kept() {}\n",
        );
        assert_eq!(
            items(source_text, false),
            [(DocItemKind::FreeFn, "kept".to_owned())]
        );
        assert_eq!(
            items(source_text, true)
                .into_iter()
                .map(|(_, name)| name)
                .collect::<Vec<_>>(),
            ["unit_test", "helper", "Fixture", "method_test", "kept"]
        );
    }

    #[test]
    fn items_keep_their_own_generics_signature_and_lines() {
        let source_text = "impl<Outer> S<Outer> {\n    /// Doc.\n    fn method<'a, Inner, const \
                           N: usize>(&self) {}\n}\n";
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        let doc_items = collect(&syntax_tree, false);
        let method = &doc_items[0];
        let generic_names: Vec<String> = method
            .generics
            .params
            .iter()
            .map(|param| match param {
                syn::GenericParam::Lifetime(lifetime_param) => lifetime_param.lifetime.to_string(),
                syn::GenericParam::Type(type_param) => type_param.ident.to_string(),
                syn::GenericParam::Const(const_param) => const_param.ident.to_string(),
            })
            .collect();
        assert_eq!(generic_names, ["'a", "Inner", "N"]);
        assert!(method.sig.is_some());
        assert_eq!(method.attrs.len(), 1);
        assert_eq!(
            (
                method.lines.span.to_string(),
                method.lines.header.to_string()
            ),
            ("2-3".to_owned(), "2-3".to_owned())
        );
        assert_eq!(method.description(), "method `method`");
        assert_eq!(DocItemKind::TraitMethod.noun(), "trait method");
    }
}
