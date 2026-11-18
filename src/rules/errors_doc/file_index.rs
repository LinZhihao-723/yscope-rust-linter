//! A per-file index of fn bodies by name, of every identifier in the file, of the names under which
//! two different qualifiers may name the same symbol, and of the `From` conversions between error
//! types.

use std::collections::HashMap;
use std::collections::HashSet;

use proc_macro2::Delimiter;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use syn::visit::Visit;

use crate::rules::errors_doc::symbol::QualifierLinks;
use crate::syntax::symbol_path::SymbolPath;
use crate::syntax::symbol_path::is_upper_camel_case;
use crate::syntax::types::type_last_segment;

/// What the `errors-doc` checks need to know about the whole file a fn is in.
///
/// # Type Parameters
///
/// * `'ast` - The lifetime of the file's syntax tree.
#[derive(Default)]
pub struct FileIndex<'ast> {
    /// Every fn with a body in the file (free fns, inherent and trait impl methods, trait default
    /// methods), keyed by the fn's name.
    helper_fns: HashMap<String, Vec<HelperFn<'ast>>>,

    /// How often each identifier appears in the file, including inside macro invocations, which
    /// tells the names that a `use` item imports but the file never names again.
    identifier_counts: HashMap<String, usize>,

    /// The last path segments and method names of the calls anywhere in the file, including the
    /// calls inside macro invocations, such as `run` for `tokio::spawn(run(..))` or `send` for
    /// `client.send(..)`.
    called_names: HashSet<String>,

    /// The traits the file declares or implements.
    trait_names: HashSet<String>,

    /// The names that `use` items import into the file.
    imported_names: HashSet<String>,

    /// The `(original, alias)` pairs of `use .. as ..` renames and `type .. = ..` aliases.
    aliases: Vec<(String, String)>,

    /// The conversions of the file's `impl From<S> for T` items and `#[from]` variant fields.
    conversions: Vec<Conversion>,

    /// The enums the file declares.
    enum_names: HashSet<String>,

    /// The impl self types and traits around the item the walk is in.
    owner_stack: Vec<Option<&'ast syn::Ident>>,
}

impl<'ast> FileIndex<'ast> {
    /// # Returns
    ///
    /// The index of `syntax_tree`.
    pub fn build(syntax_tree: &'ast syn::File) -> Self {
        let mut file_index = Self::default();
        file_index.visit_file(syntax_tree);
        file_index
    }

    /// # Returns
    ///
    /// The fns named `name` anywhere in the file.
    pub fn helper_fns(&self, name: &str) -> &[HelperFn<'ast>] {
        self.helper_fns.get(name).map_or(&[], Vec::as_slice)
    }

    /// # Returns
    ///
    /// Whether some call anywhere in the file calls a fn or method named `name`.
    pub fn is_called(&self, name: &str) -> bool {
        self.called_names.contains(name)
    }

    /// Builds the qualifier links of the file:
    ///
    /// * The traits the file declares or implements are trait names.
    /// * So is a name that a `use` item imports but that the file never names again, since a trait
    ///   imported only for its methods looks like that.
    /// * Every `use .. as ..` rename and `type .. = ..` alias links its two names.
    ///
    /// # Returns
    ///
    /// The file's qualifier links.
    pub fn qualifier_links(&self) -> QualifierLinks {
        let imported_only = self
            .imported_names
            .iter()
            .filter(|name| self.identifier_counts.get(*name).copied() == Some(1));
        QualifierLinks::new(
            self.trait_names.iter().chain(imported_only).cloned(),
            self.aliases.iter().cloned(),
        )
    }

    /// Finds the variants that a `From` conversion of an error of `source_type` into the type named
    /// `target` produces, through the file's conversions whose source matches `source_type`
    /// exactly.
    ///
    /// # Returns
    ///
    /// The variants, such as `DecodeError::Invalid`, or [`None`] if no conversion matches or one
    /// that matches has variants the index can't name.
    pub fn convert(&self, source_type: &SymbolPath, target: &str) -> Option<Vec<SymbolPath>> {
        let mut variants: Vec<SymbolPath> = Vec::new();
        for conversion in &self.conversions {
            if conversion.target != target || !conversion.source.matches_type(source_type) {
                continue;
            }
            for variant in conversion.variants.as_ref()? {
                if !variants.contains(variant) {
                    variants.push(variant.clone());
                }
            }
        }
        (!variants.is_empty()).then_some(variants)
    }

    /// # Returns
    ///
    /// The variants that some `From` conversion into the type named `target` produces, or [`None`]
    /// if the file doesn't declare `target` as an enum or a conversion into it has variants the
    /// index can't name.
    pub fn conversion_variants_into(&self, target: &str) -> Option<Vec<SymbolPath>> {
        if !self.enum_names.contains(target) {
            return None;
        }
        let mut variants: Vec<SymbolPath> = Vec::new();
        for conversion in self
            .conversions
            .iter()
            .filter(|conversion| conversion.target == target)
        {
            for variant in conversion.variants.as_ref()? {
                if !variants.contains(variant) {
                    variants.push(variant.clone());
                }
            }
        }
        Some(variants)
    }

    /// Records the conversion of an `impl From<S> for T` item, unless `S` is a type parameter of
    /// the impl. Its variants are those that the tail and `return` expressions of its `from` fn
    /// build.
    fn add_from_impl(&mut self, item_impl: &syn::ItemImpl) {
        let Some((trait_path, _)) = &item_impl.trait_ else {
            return;
        };
        let Some(trait_segment) = trait_path.segments.last() else {
            return;
        };
        if trait_segment.ident != "From" {
            return;
        }
        let syn::PathArguments::AngleBracketed(arguments) = &trait_segment.arguments else {
            return;
        };
        let Some(syn::GenericArgument::Type(syn::Type::Path(source_type))) = arguments.args.first()
        else {
            return;
        };
        let is_type_param = item_impl
            .generics
            .type_params()
            .any(|type_param| source_type.path.is_ident(&type_param.ident));
        let (Some(source), Some(target)) = (
            SymbolPath::from_syn_path(&source_type.path),
            type_last_segment(&item_impl.self_ty),
        ) else {
            return;
        };
        if is_type_param {
            return;
        }
        let from_block = item_impl
            .items
            .iter()
            .find_map(|impl_item| match impl_item {
                syn::ImplItem::Fn(impl_item_fn) if impl_item_fn.sig.ident == "from" => {
                    Some(&impl_item_fn.block)
                }
                _ => None,
            });
        let variants = from_block.and_then(|block| built_variants(block, &target));
        self.conversions.push(Conversion {
            source,
            target,
            variants,
        });
    }

    /// Records the enum's name, and a conversion for every variant with a `#[from]` field.
    fn add_enum(&mut self, item_enum: &syn::ItemEnum) {
        let target = item_enum.ident.to_string();
        for variant in &item_enum.variants {
            let from_field = variant
                .fields
                .iter()
                .find(|field| field.attrs.iter().any(|attr| attr.path().is_ident("from")));
            let Some(field) = from_field else {
                continue;
            };
            let syn::Type::Path(field_type) = &field.ty else {
                continue;
            };
            let (Some(source), Some(variant_path)) = (
                SymbolPath::from_syn_path(&field_type.path),
                SymbolPath::parse(&format!("{target}::{}", variant.ident)),
            ) else {
                continue;
            };
            self.conversions.push(Conversion {
                source,
                target: target.clone(),
                variants: Some(vec![variant_path]),
            });
        }
        self.enum_names.insert(target);
    }

    /// Records a fn body under its name, owned by the innermost impl or trait.
    fn add_helper_fn(&mut self, sig: &'ast syn::Signature, block: &'ast syn::Block) {
        let owner = self.owner_stack.last().copied().flatten();
        self.helper_fns
            .entry(sig.ident.to_string())
            .or_default()
            .push(HelperFn { owner, sig, block });
    }

    /// Counts every identifier of `tokens`, including the ones in nested groups. An identifier
    /// followed by a parenthesized group is also a called name.
    fn add_token_identifiers(&mut self, tokens: &TokenStream) {
        let mut previous_ident: Option<proc_macro2::Ident> = None;
        for token_tree in tokens.clone() {
            match token_tree {
                TokenTree::Ident(ident) => {
                    self.add_identifier(&ident);
                    previous_ident = Some(ident);
                    continue;
                }
                TokenTree::Group(group) => {
                    if group.delimiter() == Delimiter::Parenthesis
                        && let Some(ident) = previous_ident.take()
                    {
                        self.called_names.insert(ident.to_string());
                    }
                    self.add_token_identifiers(&group.stream());
                }
                TokenTree::Punct(_) | TokenTree::Literal(_) => {}
            }
            previous_ident = None;
        }
    }

    /// Counts one occurrence of `ident`.
    fn add_identifier(&mut self, ident: &proc_macro2::Ident) {
        *self.identifier_counts.entry(ident.to_string()).or_default() += 1;
    }

    /// Runs `visit` with `owner` as the innermost owner of the fns it finds.
    fn with_owner(&mut self, owner: Option<&'ast syn::Ident>, visit: impl FnOnce(&mut Self)) {
        self.owner_stack.push(owner);
        visit(self);
        self.owner_stack.pop();
    }
}

impl<'ast> Visit<'ast> for FileIndex<'ast> {
    fn visit_item_impl(&mut self, item_impl: &'ast syn::ItemImpl) {
        self.add_from_impl(item_impl);
        if let Some((trait_path, _)) = &item_impl.trait_
            && let Some(trait_segment) = trait_path.segments.last()
        {
            self.trait_names.insert(trait_segment.ident.to_string());
        }
        let owner = match &*item_impl.self_ty {
            syn::Type::Path(type_path) => {
                type_path.path.segments.last().map(|segment| &segment.ident)
            }
            _ => None,
        };
        self.with_owner(owner, |file_index| {
            syn::visit::visit_item_impl(file_index, item_impl);
        });
    }

    fn visit_item_trait(&mut self, item_trait: &'ast syn::ItemTrait) {
        self.trait_names.insert(item_trait.ident.to_string());
        self.with_owner(Some(&item_trait.ident), |file_index| {
            syn::visit::visit_item_trait(file_index, item_trait);
        });
    }

    fn visit_item_fn(&mut self, item_fn: &'ast syn::ItemFn) {
        self.add_helper_fn(&item_fn.sig, &item_fn.block);
        self.with_owner(None, |file_index| {
            syn::visit::visit_item_fn(file_index, item_fn);
        });
    }

    fn visit_impl_item_fn(&mut self, impl_item_fn: &'ast syn::ImplItemFn) {
        self.add_helper_fn(&impl_item_fn.sig, &impl_item_fn.block);
        self.with_owner(None, |file_index| {
            syn::visit::visit_impl_item_fn(file_index, impl_item_fn);
        });
    }

    fn visit_trait_item_fn(&mut self, trait_item_fn: &'ast syn::TraitItemFn) {
        if let Some(block) = &trait_item_fn.default {
            self.add_helper_fn(&trait_item_fn.sig, block);
        }
        self.with_owner(None, |file_index| {
            syn::visit::visit_trait_item_fn(file_index, trait_item_fn);
        });
    }

    fn visit_item_enum(&mut self, item_enum: &'ast syn::ItemEnum) {
        self.add_enum(item_enum);
        syn::visit::visit_item_enum(self, item_enum);
    }

    fn visit_item_type(&mut self, item_type: &'ast syn::ItemType) {
        if let syn::Type::Path(type_path) = &*item_type.ty
            && let Some(original) = type_path.path.segments.last()
        {
            self.aliases
                .push((original.ident.to_string(), item_type.ident.to_string()));
        }
        syn::visit::visit_item_type(self, item_type);
    }

    fn visit_use_name(&mut self, use_name: &'ast syn::UseName) {
        self.imported_names.insert(use_name.ident.to_string());
        syn::visit::visit_use_name(self, use_name);
    }

    fn visit_use_rename(&mut self, use_rename: &'ast syn::UseRename) {
        self.aliases
            .push((use_rename.ident.to_string(), use_rename.rename.to_string()));
        syn::visit::visit_use_rename(self, use_rename);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(expr_path) = &*call.func
            && let Some(segment) = expr_path.path.segments.last()
        {
            self.called_names.insert(segment.ident.to_string());
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, method_call: &'ast syn::ExprMethodCall) {
        self.called_names.insert(method_call.method.to_string());
        syn::visit::visit_expr_method_call(self, method_call);
    }

    fn visit_ident(&mut self, ident: &'ast syn::Ident) {
        self.add_identifier(ident);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.add_token_identifiers(&mac.tokens);
        syn::visit::visit_macro(self, mac);
    }
}

/// A fn body of the file, with its signature and the type or trait it belongs to.
///
/// # Type Parameters
///
/// * `'ast` - The lifetime of the file's syntax tree.
#[derive(Clone, Copy)]
pub struct HelperFn<'ast> {
    /// The last segment of the impl's self type, or the trait's name, or [`None`] for a free fn.
    pub owner: Option<&'ast syn::Ident>,

    pub sig: &'ast syn::Signature,
    pub block: &'ast syn::Block,
}

/// Collects the expressions through which an error may leave a fn body or a closure: the values of
/// its `return` expressions and the operands of its `?` expressions, outside nested closures, async
/// blocks and items.
///
/// # Type Parameters
///
/// * `'ast` - The lifetime of the syntax tree being walked.
#[derive(Default)]
pub struct ExitCollector<'ast> {
    pub returned: Vec<&'ast syn::Expr>,
    pub tried: Vec<&'ast syn::Expr>,
}

impl<'ast> Visit<'ast> for ExitCollector<'ast> {
    fn visit_expr_closure(&mut self, _closure: &'ast syn::ExprClosure) {}

    fn visit_expr_async(&mut self, _expr_async: &'ast syn::ExprAsync) {}

    fn visit_item(&mut self, _item: &'ast syn::Item) {}

    fn visit_expr_return(&mut self, expr_return: &'ast syn::ExprReturn) {
        if let Some(returned) = &expr_return.expr {
            self.returned.push(returned);
        }
        syn::visit::visit_expr_return(self, expr_return);
    }

    fn visit_expr_try(&mut self, expr_try: &'ast syn::ExprTry) {
        self.tried.push(&expr_try.expr);
        syn::visit::visit_expr_try(self, expr_try);
    }
}

/// A `From` conversion between two error types.
#[derive(Debug, Clone)]
struct Conversion {
    source: SymbolPath,

    /// The last segment of the target type.
    target: String,

    /// The variants of the target that the conversion builds, or [`None`] if the index can't name
    /// them all.
    variants: Option<Vec<SymbolPath>>,
}

/// Collects the variants that a `from` fn of an impl for `target` builds in its tail and `return`
/// expressions, looking through blocks, `if` and `match`. `Self::V` is spelled `target::V`.
///
/// # Returns
///
/// The variants, or [`None`] if some returned expression isn't a variant of `target`.
fn built_variants(block: &syn::Block, target: &str) -> Option<Vec<SymbolPath>> {
    let mut exit_collector = ExitCollector::default();
    exit_collector.visit_block(block);
    let mut variants = Vec::new();
    if let Some(syn::Stmt::Expr(tail, None)) = block.stmts.last() {
        collect_built_variants(tail, target, &mut variants)?;
    }
    for returned in exit_collector.returned {
        collect_built_variants(returned, target, &mut variants)?;
    }
    (!variants.is_empty()).then_some(variants)
}

/// Adds the variants of `target` that `expr` builds to `variants`.
///
/// # Returns
///
/// [`None`] if some branch of `expr` isn't a variant of `target`.
fn collect_built_variants(
    expr: &syn::Expr,
    target: &str,
    variants: &mut Vec<SymbolPath>,
) -> Option<()> {
    let path = match expr {
        syn::Expr::Paren(expr_paren) => {
            return collect_built_variants(&expr_paren.expr, target, variants);
        }
        syn::Expr::Block(expr_block) => {
            return collect_block_variants(&expr_block.block, target, variants);
        }
        syn::Expr::If(expr_if) => {
            collect_block_variants(&expr_if.then_branch, target, variants)?;
            let (_, else_branch) = expr_if.else_branch.as_ref()?;
            return collect_built_variants(else_branch, target, variants);
        }
        syn::Expr::Match(expr_match) => {
            for arm in &expr_match.arms {
                collect_built_variants(&arm.body, target, variants)?;
            }
            return Some(());
        }
        syn::Expr::Call(call) => match &*call.func {
            syn::Expr::Path(expr_path) => &expr_path.path,
            _ => return None,
        },
        syn::Expr::Path(expr_path) => &expr_path.path,
        syn::Expr::Struct(expr_struct) => &expr_struct.path,
        _ => return None,
    };
    let mut segments: Vec<String> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let [.., qualifier, last] = segments.as_mut_slice() else {
        return None;
    };
    if qualifier == "Self" {
        target.clone_into(qualifier);
    }
    if qualifier != target || !is_upper_camel_case(last) {
        return None;
    }
    let variant = SymbolPath::parse(&format!("{qualifier}::{last}"))?;
    if !variants.contains(&variant) {
        variants.push(variant);
    }
    Some(())
}

/// Adds the variants of `target` that the tail expression of `block` builds to `variants`.
///
/// # Returns
///
/// [`None`] if `block` has no tail expression or some branch of it isn't a variant of `target`.
fn collect_block_variants(
    block: &syn::Block,
    target: &str,
    variants: &mut Vec<SymbolPath>,
) -> Option<()> {
    let Some(syn::Stmt::Expr(tail, None)) = block.stmts.last() else {
        return None;
    };
    collect_built_variants(tail, target, variants)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_fns_are_indexed_with_their_owners() {
        let syntax_tree = syn::parse_file(
            "fn free() {}\nimpl S { fn method(&self) {} }\ntrait T { fn provided() {} fn \
             required(); }\nmod m { fn nested() {} }",
        )
        .expect("valid file");
        let file_index = FileIndex::build(&syntax_tree);
        let owners = |name| -> Vec<Option<String>> {
            file_index
                .helper_fns(name)
                .iter()
                .map(|helper_fn| helper_fn.owner.map(ToString::to_string))
                .collect()
        };
        assert_eq!(owners("free"), [None]);
        assert_eq!(owners("method"), [Some("S".to_owned())]);
        assert_eq!(owners("provided"), [Some("T".to_owned())]);
        assert_eq!(owners("nested"), [None]);
        assert_eq!(owners("required"), []);
    }

    #[test]
    fn identifiers_in_macro_tokens_count_as_uses_of_an_import() {
        let syntax_tree = syn::parse_file("use store::Store;\nfn f() { spawn!(Store::open()); }")
            .expect("valid file");
        let qualifier_links = FileIndex::build(&syntax_tree).qualifier_links();
        assert!(!qualifier_links.are_linked("Cache", "Store"));
    }

    #[test]
    fn called_names_come_from_calls_method_calls_and_macro_tokens() {
        let syntax_tree = syn::parse_file(
            "fn run() {}\nfn f(client: Client) {\n    spawn(move || run_indexer(1));\n    \
             client.poll_ready();\n    assert!(check(2), \"{}\", label);\n}",
        )
        .expect("valid file");
        let file_index = FileIndex::build(&syntax_tree);
        let called: Vec<bool> = [
            "spawn",
            "run_indexer",
            "poll_ready",
            "check",
            "run",
            "label",
        ]
        .iter()
        .map(|name| file_index.is_called(name))
        .collect();
        assert_eq!(called, [true, true, true, true, false, false]);
    }

    #[test]
    fn qualifier_links_hold_traits_imported_only_names_and_aliases() {
        let syntax_tree = syn::parse_file(
            "use config::YamlConfig;\nuse store::Store;\nuse crate::Error as CacheError;\ntype \
             DbError = sqlx::Error;\ntrait Local {}\nimpl Display for S {}\nfn f() { \
             Store::open(); }",
        )
        .expect("valid file");
        let qualifier_links = FileIndex::build(&syntax_tree).qualifier_links();
        for (left, right) in [
            ("ServerConfig", "YamlConfig"),
            ("S", "Local"),
            ("S", "Display"),
            ("Error", "CacheError"),
            ("DbError", "Error"),
        ] {
            assert!(
                qualifier_links.are_linked(left, right),
                "`{left}` and `{right}`"
            );
        }
        assert!(!qualifier_links.are_linked("Cache", "Store"));
        assert!(!qualifier_links.are_linked("Cache", "Storage"));
    }

    #[test]
    fn conversions_come_from_from_impls_and_from_fields() {
        let syntax_tree = syn::parse_file(concat!(
            "enum DecodeError { Incomplete, Invalid(ProtocolError) }\n",
            "impl From<ProtocolError> for DecodeError {\n",
            "    fn from(error: ProtocolError) -> Self { Self::Invalid(error) }\n",
            "}\n",
            "impl From<ReadError> for DecodeError {\n",
            "    fn from(error: ReadError) -> Self {\n",
            "        match error { ReadError::Eof => Self::Incomplete, _ => \
             DecodeError::Invalid(x) }\n",
            "    }\n",
            "}\n",
            "impl From<OpaqueError> for DecodeError {\n",
            "    fn from(error: OpaqueError) -> Self { convert(error) }\n",
            "}\n",
            "impl<E> From<E> for Wrapper { fn from(error: E) -> Self { Self::Any } }\n",
            "enum InternalError { Creation(#[from] std::io::Error), Other(String) }\n",
        ))
        .expect("valid file");
        let file_index = FileIndex::build(&syntax_tree);
        let path = |text: &str| SymbolPath::parse(text).expect("valid path");
        let converted = |source: &str, target: &str| {
            file_index
                .convert(&path(source), target)
                .map(|variants| variants.iter().map(ToString::to_string).collect::<Vec<_>>())
        };
        assert_eq!(
            converted("ProtocolError", "DecodeError"),
            Some(vec!["DecodeError::Invalid".to_owned()])
        );
        assert_eq!(
            converted("rmp::ReadError", "DecodeError"),
            Some(vec![
                "DecodeError::Incomplete".to_owned(),
                "DecodeError::Invalid".to_owned()
            ])
        );
        assert_eq!(converted("OpaqueError", "DecodeError"), None);
        assert_eq!(converted("Anything", "Wrapper"), None);
        assert_eq!(
            converted("io::Error", "InternalError"),
            Some(vec!["InternalError::Creation".to_owned()])
        );
        assert_eq!(converted("fmt::Error", "InternalError"), None);
        assert_eq!(file_index.conversion_variants_into("DecodeError"), None);
        assert_eq!(
            file_index.conversion_variants_into("InternalError"),
            Some(vec![path("InternalError::Creation")])
        );
        assert_eq!(file_index.conversion_variants_into("Undeclared"), None);
    }
}
