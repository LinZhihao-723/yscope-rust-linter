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
pub struct DocItem<'ast> {
    pub kind: DocItemKind<'ast>,
    name: String,

    /// The item's outer attributes, including its doc comments.
    pub attrs: &'ast [syn::Attribute],

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

    /// # Returns
    ///
    /// The item's own generic parameters. An impl method's generics don't include the impl's.
    pub const fn generics(&self) -> &syn::Generics {
        match self.kind {
            DocItemKind::FreeFn(sig)
            | DocItemKind::InherentMethod(sig)
            | DocItemKind::TraitImplMethod(sig)
            | DocItemKind::TraitMethod(sig) => &sig.generics,
            DocItemKind::Struct(generics)
            | DocItemKind::Enum(generics)
            | DocItemKind::Union(generics)
            | DocItemKind::Trait(generics)
            | DocItemKind::TypeAlias(generics) => generics,
        }
    }
}

/// The kinds of [`DocItem`], each with the signature of a fn or the generic parameters of another
/// item.
///
/// # Type Parameters
///
/// * `'ast` - The lifetime of the syntax tree the item is in.
#[derive(Clone, Copy)]
pub enum DocItemKind<'ast> {
    /// A fn outside any impl or trait, including a fn nested in another fn's body.
    FreeFn(&'ast syn::Signature),

    /// A fn in an impl block without a trait.
    InherentMethod(&'ast syn::Signature),

    /// A fn in an `impl Trait for Type` block.
    TraitImplMethod(&'ast syn::Signature),

    /// A fn declared in a `trait` block, with or without a default body.
    TraitMethod(&'ast syn::Signature),

    Struct(&'ast syn::Generics),
    Enum(&'ast syn::Generics),
    Union(&'ast syn::Generics),
    Trait(&'ast syn::Generics),
    TypeAlias(&'ast syn::Generics),
}

impl DocItemKind<'_> {
    /// # Returns
    ///
    /// The noun that diagnostics name an item of this kind with, such as `fn` or `trait method`.
    const fn noun(self) -> &'static str {
        match self {
            Self::FreeFn(_) => "fn",
            Self::InherentMethod(_) => "method",
            Self::TraitImplMethod(_) => "trait impl method",
            Self::TraitMethod(_) => "trait method",
            Self::Struct(_) => "struct",
            Self::Enum(_) => "enum",
            Self::Union(_) => "union",
            Self::Trait(_) => "trait",
            Self::TypeAlias(_) => "type alias",
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
    impl_stack: Vec<ImplKind>,

    doc_items: Vec<DocItem<'ast>>,
}

impl<'ast> ItemWalker<'ast> {
    /// # Returns
    ///
    /// Whether an item with `attrs` is test code that the walk skips.
    fn is_skipped(&self, attrs: &[syn::Attribute], is_fn: bool) -> bool {
        !self.include_tests && test_code::is_test_item(attrs, is_fn)
    }

    /// Records the item printed by `item` unless it has no tokens after its attributes.
    fn add(
        &mut self,
        kind: DocItemKind<'ast>,
        ident: &syn::Ident,
        attrs: &'ast [syn::Attribute],
        item: &impl quote::ToTokens,
    ) {
        if let Some(lines) = item_lines::of(item) {
            self.doc_items.push(DocItem {
                kind,
                name: ident.to_string(),
                attrs,
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
                DocItemKind::FreeFn(&item_fn.sig),
                &item_fn.sig.ident,
                &item_fn.attrs,
                item_fn,
            ),
            syn::Item::Struct(item_struct) => self.add(
                DocItemKind::Struct(&item_struct.generics),
                &item_struct.ident,
                &item_struct.attrs,
                item_struct,
            ),
            syn::Item::Enum(item_enum) => self.add(
                DocItemKind::Enum(&item_enum.generics),
                &item_enum.ident,
                &item_enum.attrs,
                item_enum,
            ),
            syn::Item::Union(item_union) => self.add(
                DocItemKind::Union(&item_union.generics),
                &item_union.ident,
                &item_union.attrs,
                item_union,
            ),
            syn::Item::Trait(item_trait) => self.add(
                DocItemKind::Trait(&item_trait.generics),
                &item_trait.ident,
                &item_trait.attrs,
                item_trait,
            ),
            syn::Item::Type(item_type) => self.add(
                DocItemKind::TypeAlias(&item_type.generics),
                &item_type.ident,
                &item_type.attrs,
                item_type,
            ),
            _ => {}
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_item_impl(&mut self, item_impl: &'ast syn::ItemImpl) {
        self.impl_stack.push(if item_impl.trait_.is_some() {
            ImplKind::Trait
        } else {
            ImplKind::Inherent
        });
        syn::visit::visit_item_impl(self, item_impl);
        self.impl_stack.pop();
    }

    fn visit_impl_item_fn(&mut self, impl_item_fn: &'ast syn::ImplItemFn) {
        if self.is_skipped(&impl_item_fn.attrs, true) {
            return;
        }
        let impl_kind = self
            .impl_stack
            .last()
            .expect("an impl item fn is visited inside its impl block");
        let kind = match impl_kind {
            ImplKind::Inherent => DocItemKind::InherentMethod(&impl_item_fn.sig),
            ImplKind::Trait => DocItemKind::TraitImplMethod(&impl_item_fn.sig),
        };
        self.add(
            kind,
            &impl_item_fn.sig.ident,
            &impl_item_fn.attrs,
            impl_item_fn,
        );
        syn::visit::visit_impl_item_fn(self, impl_item_fn);
    }

    fn visit_trait_item_fn(&mut self, trait_item_fn: &'ast syn::TraitItemFn) {
        if self.is_skipped(&trait_item_fn.attrs, true) {
            return;
        }
        self.add(
            DocItemKind::TraitMethod(&trait_item_fn.sig),
            &trait_item_fn.sig.ident,
            &trait_item_fn.attrs,
            trait_item_fn,
        );
        syn::visit::visit_trait_item_fn(self, trait_item_fn);
    }
}

/// Whether an impl block implements a trait.
#[derive(Debug)]
enum ImplKind {
    Inherent,
    Trait,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(source_text: &str, include_tests: bool) -> Vec<String> {
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        collect(&syntax_tree, include_tests)
            .iter()
            .map(DocItem::description)
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
                "fn `free`",
                "fn `nested`",
                "struct `S`",
                "enum `E`",
                "union `U`",
                "trait `T`",
                "trait method `declared`",
                "trait method `provided`",
                "type alias `Alias`",
                "method `inherent`",
                "trait impl method `in_body`",
                "trait impl method `implemented`",
                "fn `in_mod`",
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
            "#[cfg(test)]\nimpl S { fn test_impl_method() {} }\n",
            "trait T {\n    #[cfg(test)]\n    fn test_trait_method();\n}\n",
            "fn kept() {}\n",
        );
        assert_eq!(items(source_text, false), ["trait `T`", "fn `kept`"]);
        assert_eq!(
            items(source_text, true),
            [
                "fn `unit_test`",
                "fn `helper`",
                "struct `Fixture`",
                "method `method_test`",
                "method `test_impl_method`",
                "trait `T`",
                "trait method `test_trait_method`",
                "fn `kept`",
            ]
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
            .generics()
            .params
            .iter()
            .map(|param| match param {
                syn::GenericParam::Lifetime(lifetime_param) => lifetime_param.lifetime.to_string(),
                syn::GenericParam::Type(type_param) => type_param.ident.to_string(),
                syn::GenericParam::Const(const_param) => const_param.ident.to_string(),
            })
            .collect();
        assert_eq!(generic_names, ["'a", "Inner", "N"]);
        assert_eq!(method.attrs.len(), 1);
        assert_eq!(
            (
                method.lines.span.to_string(),
                method.lines.header.to_string()
            ),
            ("2-3".to_owned(), "2-3".to_owned())
        );
        assert_eq!(method.description(), "method `method`");
    }
}
