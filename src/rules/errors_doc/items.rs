//! The item walk: finds the fns whose `# Errors` section can be checked, with the impl or trait
//! context their paths are matched in.

use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::line_ranges::LineSpan;
use crate::rules::errors_doc::known_apis;
use crate::rules::errors_doc::symbol::SymbolScope;
use crate::syntax::span;
use crate::syntax::symbol_path::SymbolPath;
use crate::syntax::test_code;
use crate::syntax::types::peel_type_parens;
use crate::syntax::types::type_last_segment;

/// A fn that returns a `Result` and isn't test code.
///
/// # Type Parameters
///
/// * `'ast` - The lifetime of the syntax tree the fn is in.
pub struct FnItem<'ast> {
    pub name: String,
    pub attrs: &'ast [syn::Attribute],

    /// The block whose errors leave the fn: the fn's block, or the `async` block that a fn
    /// returning `impl Future<Output = Result<..>>` ends with. A trait fn without a default body
    /// has none.
    pub block: Option<&'ast syn::Block>,

    /// The names of the fn's parameters that are plain identifiers.
    pub param_names: Vec<String>,

    /// The names of the parameters among [`Self::param_names`] whose type is callable or a future,
    /// as [`is_callable_type`] decides.
    pub callable_params: Vec<String>,

    pub scope: SymbolScope,

    /// Whether the fn is an associated fn of an impl or a trait, so an unqualified call can't call
    /// it.
    pub is_associated: bool,

    /// The second generic argument of the returned `Result`, such as `CacheError` in
    /// `Result<Vec<Publication>, CacheError>`, or `anyhow::Error` for `anyhow::Result`.
    pub own_error_type: Option<SymbolPath>,

    /// The lines from the fn's first attribute or doc line to its closing brace.
    pub anchor: LineSpan,

    /// The lines from the fn's first attribute or doc line through its `fn` keyword.
    pub header: LineSpan,
}

/// Collects every fn of `syntax_tree` that returns a `Result` or an `impl Future` of a `Result`:
/// free fns, inherent and trait impl methods, and trait methods with or without a default body,
/// including fns nested in other fns.
///
/// Unless `include_tests` is set, these are skipped:
///
/// * Fns with an attribute whose last path segment is `test`, such as `#[tokio::test]`.
/// * Items with `#[cfg(test)]`, including whole modules and impl blocks.
///
/// # Returns
///
/// The collected fns, in source order.
pub fn collect(syntax_tree: &syn::File, include_tests: bool) -> Vec<FnItem<'_>> {
    let mut item_walker = ItemWalker {
        include_tests,
        context_stack: Vec::new(),
        fn_items: Vec::new(),
    };
    item_walker.visit_file(syntax_tree);
    item_walker.fn_items
}

/// The impl or trait an item is nested in.
#[derive(Debug, Clone, Default)]
struct ItemContext {
    self_names: Vec<String>,
    generic_params: Vec<String>,
    is_associated: bool,
}

/// The `syn` visitor that collects [`FnItem`]s.
///
/// # Type Parameters
///
/// * `'ast` - The lifetime of the syntax tree being walked.
struct ItemWalker<'ast> {
    include_tests: bool,
    context_stack: Vec<ItemContext>,
    fn_items: Vec<FnItem<'ast>>,
}

impl<'ast> ItemWalker<'ast> {
    /// # Returns
    ///
    /// Whether an item with `attrs` is `#[cfg(test)]` code that the walk skips.
    fn is_skipped_test_code(&self, attrs: &[syn::Attribute]) -> bool {
        !self.include_tests && test_code::has_cfg_test(attrs)
    }

    /// Records a fn whose item isn't skipped test code, in the innermost item context:
    ///
    /// * A `#[test]` fn is skipped unless tests are included.
    /// * A fn whose return type isn't a `Result` path or an `impl Future` of one is skipped.
    /// * The errors of a fn returning an `impl Future` leave it through the `async` block its body
    ///   ends with. Without such a block, the fn's body isn't scanned.
    /// * The fn's anchor runs from its first attribute or doc line to its closing brace, and its
    ///   header ends at its `fn` keyword.
    ///
    /// # Panics
    ///
    /// Panics if the fn's closing brace or `fn` keyword comes before its first attribute, which
    /// `syn` spans rule out.
    fn add_fn(
        &mut self,
        attrs: &'ast [syn::Attribute],
        sig: &'ast syn::Signature,
        block: Option<&'ast syn::Block>,
    ) {
        if !self.include_tests && attrs.iter().any(test_code::is_test_attribute) {
            return;
        }
        let Some((result_path, is_future)) = result_return_path(&sig.output) else {
            return;
        };
        let scanned_block = if is_future {
            block.and_then(async_tail_block)
        } else {
            block
        };
        let context = self.context_stack.last().cloned().unwrap_or_default();
        let generic_params = context.generic_params.into_iter().chain(
            sig.generics
                .type_params()
                .map(|type_param| type_param.ident.to_string()),
        );
        let fn_line = span::start_position(sig.fn_token.span).line;
        let first_line = attrs
            .iter()
            .map(|attr| span::start_position(attr.pound_token.span).line)
            .chain(std::iter::once(fn_line))
            .min()
            .unwrap_or(fn_line);
        let last_line = block
            .map_or_else(
                || span::start_position(sig.span()).line,
                |block| span::end_line(block.brace_token.span.close()),
            )
            .max(fn_line);
        let typed_params: Vec<(String, &syn::Type)> = sig
            .inputs
            .iter()
            .filter_map(|input| match input {
                syn::FnArg::Typed(pat_type) => match &*pat_type.pat {
                    syn::Pat::Ident(pat_ident) => {
                        Some((pat_ident.ident.to_string(), &*pat_type.ty))
                    }
                    _ => None,
                },
                syn::FnArg::Receiver(_) => None,
            })
            .collect();
        let callable_type_params = callable_type_params(&sig.generics);
        let callable_params = typed_params
            .iter()
            .filter(|(_, ty)| is_callable_type(ty, &callable_type_params))
            .map(|(name, _)| name.clone())
            .collect();
        let param_names = typed_params.into_iter().map(|(name, _)| name).collect();
        self.fn_items.push(FnItem {
            name: sig.ident.to_string(),
            attrs,
            block: scanned_block,
            param_names,
            callable_params,
            scope: SymbolScope::new(context.self_names, generic_params),
            is_associated: context.is_associated,
            own_error_type: own_error_type(result_path),
            anchor: LineSpan::new(first_line, last_line).expect("a fn ends after it starts"),
            header: LineSpan::new(first_line, fn_line).expect("a fn's attributes precede it"),
        });
    }

    /// Runs `visit` with `context` as the innermost item context.
    fn with_context(&mut self, context: ItemContext, visit: impl FnOnce(&mut Self)) {
        self.context_stack.push(context);
        visit(self);
        self.context_stack.pop();
    }
}

impl<'ast> Visit<'ast> for ItemWalker<'ast> {
    fn visit_item_mod(&mut self, item_mod: &'ast syn::ItemMod) {
        if !self.is_skipped_test_code(&item_mod.attrs) {
            syn::visit::visit_item_mod(self, item_mod);
        }
    }

    fn visit_item_impl(&mut self, item_impl: &'ast syn::ItemImpl) {
        if self.is_skipped_test_code(&item_impl.attrs) {
            return;
        }
        let mut self_names: Vec<String> =
            type_last_segment(&item_impl.self_ty).into_iter().collect();
        if let Some((trait_path, _)) = &item_impl.trait_
            && let Some(trait_segment) = trait_path.segments.last()
        {
            self_names.push(trait_segment.ident.to_string());
        }
        let context = ItemContext {
            self_names,
            generic_params: type_param_names(&item_impl.generics),
            is_associated: true,
        };
        self.with_context(context, |item_walker| {
            syn::visit::visit_item_impl(item_walker, item_impl);
        });
    }

    fn visit_item_trait(&mut self, item_trait: &'ast syn::ItemTrait) {
        if self.is_skipped_test_code(&item_trait.attrs) {
            return;
        }
        let context = ItemContext {
            self_names: vec![item_trait.ident.to_string()],
            generic_params: type_param_names(&item_trait.generics),
            is_associated: true,
        };
        self.with_context(context, |item_walker| {
            syn::visit::visit_item_trait(item_walker, item_trait);
        });
    }

    fn visit_item_fn(&mut self, item_fn: &'ast syn::ItemFn) {
        if self.is_skipped_test_code(&item_fn.attrs) {
            return;
        }
        self.add_fn(&item_fn.attrs, &item_fn.sig, Some(&item_fn.block));
        self.with_context(ItemContext::default(), |item_walker| {
            syn::visit::visit_item_fn(item_walker, item_fn);
        });
    }

    fn visit_impl_item_fn(&mut self, impl_item_fn: &'ast syn::ImplItemFn) {
        if self.is_skipped_test_code(&impl_item_fn.attrs) {
            return;
        }
        self.add_fn(
            &impl_item_fn.attrs,
            &impl_item_fn.sig,
            Some(&impl_item_fn.block),
        );
        self.with_context(ItemContext::default(), |item_walker| {
            syn::visit::visit_impl_item_fn(item_walker, impl_item_fn);
        });
    }

    fn visit_trait_item_fn(&mut self, trait_item_fn: &'ast syn::TraitItemFn) {
        if self.is_skipped_test_code(&trait_item_fn.attrs) {
            return;
        }
        self.add_fn(
            &trait_item_fn.attrs,
            &trait_item_fn.sig,
            trait_item_fn.default.as_ref(),
        );
        self.with_context(ItemContext::default(), |item_walker| {
            syn::visit::visit_trait_item_fn(item_walker, trait_item_fn);
        });
    }
}

/// # Returns
///
/// The names of the type parameters of `generics` that a bound of [`known_apis::CALLABLE_TRAITS`]
/// constrains, inline or in the `where` clause.
fn callable_type_params(generics: &syn::Generics) -> Vec<String> {
    let mut names: Vec<String> = generics
        .type_params()
        .filter(|type_param| type_param.bounds.iter().any(is_callable_bound))
        .map(|type_param| type_param.ident.to_string())
        .collect();
    let where_predicates = generics
        .where_clause
        .iter()
        .flat_map(|where_clause| &where_clause.predicates);
    for predicate in where_predicates {
        if let syn::WherePredicate::Type(predicate_type) = predicate
            && predicate_type.bounds.iter().any(is_callable_bound)
            && let syn::Type::Path(type_path) = &predicate_type.bounded_ty
            && let Some(ident) = type_path.path.get_ident()
        {
            names.push(ident.to_string());
        }
    }
    names
}

/// # Returns
///
/// Whether `bound` is a trait bound whose last segment is one of [`known_apis::CALLABLE_TRAITS`].
fn is_callable_bound(bound: &syn::TypeParamBound) -> bool {
    match bound {
        syn::TypeParamBound::Trait(trait_bound) => {
            trait_bound.path.segments.last().is_some_and(|segment| {
                known_apis::CALLABLE_TRAITS
                    .iter()
                    .any(|name| segment.ident == name)
            })
        }
        _ => false,
    }
}

/// Decides whether a parameter of type `ty` is callable or a future:
///
/// * A fn pointer.
/// * An `impl` or `dyn` type with a bound of [`known_apis::CALLABLE_TRAITS`].
/// * One of `callable_type_params`.
/// * A reference, `Box` or `Pin` of any of these.
///
/// # Returns
///
/// Whether the type is callable.
fn is_callable_type(ty: &syn::Type, callable_type_params: &[String]) -> bool {
    match peel_type_parens(ty) {
        syn::Type::FnPtr(_) => true,
        syn::Type::ImplTrait(type_impl_trait) => {
            type_impl_trait.bounds.iter().any(is_callable_bound)
        }
        syn::Type::TraitObject(type_trait_object) => {
            type_trait_object.bounds.iter().any(is_callable_bound)
        }
        syn::Type::Reference(type_reference) => {
            is_callable_type(&type_reference.elem, callable_type_params)
        }
        syn::Type::Path(type_path) => {
            if let Some(ident) = type_path.path.get_ident() {
                return callable_type_params.iter().any(|name| ident == name);
            }
            let Some(last_segment) = type_path.path.segments.last() else {
                return false;
            };
            let syn::PathArguments::AngleBracketed(arguments) = &last_segment.arguments else {
                return false;
            };
            (last_segment.ident == "Box" || last_segment.ident == "Pin")
                && arguments.args.iter().any(|argument| {
                    matches!(
                        argument,
                        syn::GenericArgument::Type(inner)
                            if is_callable_type(inner, callable_type_params)
                    )
                })
        }
        _ => false,
    }
}

/// Finds the `Result` path a fn returns:
///
/// * Its return type, if that is a path whose last segment ends with `Result`, such as `Result`,
///   `anyhow::Result` or `DbResult`.
/// * The `Output` of an `impl Future<Output = ..>` return type that is such a path.
///
/// # Returns
///
/// The path, and whether it is the `Output` of an `impl Future`, or [`None`] if the fn returns
/// neither.
fn result_return_path(output: &syn::ReturnType) -> Option<(&syn::Path, bool)> {
    let syn::ReturnType::Type(_, return_type) = output else {
        return None;
    };
    match peel_type_parens(return_type) {
        syn::Type::Path(type_path) => {
            is_result_path(&type_path.path).then_some((&type_path.path, false))
        }
        syn::Type::ImplTrait(type_impl_trait) => type_impl_trait
            .bounds
            .iter()
            .find_map(|bound| match bound {
                syn::TypeParamBound::Trait(trait_bound) => future_output(&trait_bound.path),
                _ => None,
            })
            .map(|path| (path, true)),
        _ => None,
    }
}

/// # Returns
///
/// Whether `path`'s last segment ends with `Result`.
fn is_result_path(path: &syn::Path) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident.to_string().ends_with("Result"))
}

/// # Returns
///
/// The `Output` of a `Future<Output = ..>` bound if it is a `Result` path, or [`None`] otherwise.
fn future_output(bound_path: &syn::Path) -> Option<&syn::Path> {
    let last_segment = bound_path.segments.last()?;
    if last_segment.ident != "Future" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &last_segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| match argument {
        syn::GenericArgument::AssocType(assoc_type) if assoc_type.ident == "Output" => {
            match peel_type_parens(&assoc_type.ty) {
                syn::Type::Path(type_path) if is_result_path(&type_path.path) => {
                    Some(&type_path.path)
                }
                _ => None,
            }
        }
        _ => None,
    })
}

/// # Returns
///
/// The block of the `async` block that `block` ends with, or [`None`] if it ends with none.
fn async_tail_block(block: &syn::Block) -> Option<&syn::Block> {
    match block.stmts.last() {
        Some(syn::Stmt::Expr(syn::Expr::Async(expr_async), None)) => Some(&expr_async.block),
        _ => None,
    }
}

/// # Returns
///
/// The error type a `Result` path names: its second generic argument, or `anyhow::Error` for a
/// one-argument `anyhow::Result`.
fn own_error_type(result_path: &syn::Path) -> Option<SymbolPath> {
    let last_segment = result_path.segments.last()?;
    let syn::PathArguments::AngleBracketed(arguments) = &last_segment.arguments else {
        return None;
    };
    let mut type_arguments = arguments.args.iter().filter_map(|argument| match argument {
        syn::GenericArgument::Type(argument_type) => Some(argument_type),
        _ => None,
    });
    match type_arguments.nth(1).map(peel_type_parens) {
        Some(syn::Type::Path(type_path)) => SymbolPath::from_syn_path(&type_path.path),
        Some(_) => None,
        None => result_path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == "anyhow")
            .then(SymbolPath::anyhow_error),
    }
}

/// # Returns
///
/// The names of the type parameters of `generics`.
fn type_param_names(generics: &syn::Generics) -> Vec<String> {
    generics
        .type_params()
        .map(|type_param| type_param.ident.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fn_names(source_text: &str, include_tests: bool) -> Vec<String> {
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        collect(&syntax_tree, include_tests)
            .into_iter()
            .map(|fn_item| fn_item.name)
            .collect()
    }

    #[test]
    fn result_fns_of_every_item_kind_are_collected() {
        let source_text = "fn free() -> Result<(), E> { Ok(()) }\nfn unit() {}\nfn opt() -> \
                           Option<u8> { None }\nimpl S { fn inherent(&self) -> io::Result<()> { \
                           fn nested() -> DbResult<()> { Ok(()) } Ok(()) } }\nimpl T for S { fn \
                           implemented() -> anyhow::Result<()> { Ok(()) } }\ntrait T { fn \
                           provided() -> Result<(), E> { Ok(()) } fn required() -> Result<(), E>; \
                           }";
        assert_eq!(
            fn_names(source_text, false),
            [
                "free",
                "inherent",
                "nested",
                "implemented",
                "provided",
                "required"
            ]
        );
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        let bodiless: Vec<String> = collect(&syntax_tree, false)
            .into_iter()
            .filter(|fn_item| fn_item.block.is_none())
            .map(|fn_item| fn_item.name)
            .collect();
        assert_eq!(bodiless, ["required"]);
    }

    #[test]
    fn fns_marked_test_and_cfg_test_modules_are_skipped_unless_included() {
        let source_text = "#[test]\nfn unit_test() -> Result<(), E> { Ok(()) \
                           }\n#[tokio::test]\nasync fn async_test() -> Result<(), E> { Ok(()) \
                           }\n#[cfg(test)]\nmod tests { fn helper() -> Result<(), E> { Ok(()) } \
                           }\nfn kept() -> Result<(), E> { Ok(()) }";
        assert_eq!(fn_names(source_text, false), ["kept"]);
        assert_eq!(
            fn_names(source_text, true),
            ["unit_test", "async_test", "helper", "kept"]
        );
    }

    #[test]
    fn own_error_type_scope_and_spans_are_recorded() {
        let source_text = "impl<StorageType> Trait for Server<StorageType> {\n    /// Doc.\n    \
                           #[inline]\n    fn run<Item>(&self)\n        -> Result<(), CacheError> \
                           {\n        Ok(())\n    }\n}\nfn any() -> anyhow::Result<()> { Ok(()) }";
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        let fn_items = collect(&syntax_tree, false);
        let run = &fn_items[0];
        assert_eq!(run.own_error_type, SymbolPath::parse("CacheError"));
        assert_eq!(run.anchor, LineSpan::new(2, 7).expect("valid span"));
        assert_eq!(run.header, LineSpan::new(2, 4).expect("valid span"));
        let self_path = SymbolPath::parse("Self::load").expect("valid path");
        for qualified in ["Server::load", "Trait::load"] {
            let path = SymbolPath::parse(qualified).expect("valid path");
            assert!(
                run.scope.matches_exactly(&self_path, &path),
                "`{qualified}`"
            );
        }
        for generic in ["StorageType::new", "Item::new"] {
            let path = SymbolPath::parse(generic).expect("valid path");
            assert_eq!(run.scope.normalize(path).to_string(), "new");
        }
        assert!(run.is_associated);
        assert_eq!(fn_items[1].own_error_type, Some(SymbolPath::anyhow_error()));
        assert!(!fn_items[1].is_associated);
    }

    #[test]
    fn impl_future_fns_are_collected_with_their_async_tail_as_the_block() {
        let source_text = "fn future() -> impl Future<Output = Result<(), E>> + Send { let x = 1; \
                           async move { Ok(()) } }\nfn opaque() -> impl Future<Output = \
                           Result<(), E>> { ready(Ok(())) }\nfn other() -> impl Future<Output = \
                           u8> { async { 1 } }";
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        let fn_items = collect(&syntax_tree, false);
        let blocks: Vec<(String, usize)> = fn_items
            .iter()
            .map(|fn_item| {
                let num_stmts = fn_item.block.map_or(0, |block| block.stmts.len());
                (fn_item.name.clone(), num_stmts)
            })
            .collect();
        assert_eq!(blocks, [("future".to_owned(), 1), ("opaque".to_owned(), 0)]);
        assert_eq!(fn_items[0].own_error_type, SymbolPath::parse("E"));
    }

    #[test]
    fn callable_parameters_are_recognized_by_their_types() {
        let source_text = "fn f<Call: FnMut() -> u8, Plain, Bounded>(a: impl FnOnce(), b: fn(u8), \
                           c: &dyn Fn(), d: Box<dyn Future<Output = ()>>, e: Call, f: Bounded, g: \
                           Plain, h: u8) -> Result<(), E> where Bounded: AsyncFnOnce() { Ok(()) }";
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        let fn_items = collect(&syntax_tree, false);
        assert_eq!(fn_items[0].callable_params, ["a", "b", "c", "d", "e", "f"]);
        assert_eq!(
            fn_items[0].param_names,
            ["a", "b", "c", "d", "e", "f", "g", "h"]
        );
    }
}
