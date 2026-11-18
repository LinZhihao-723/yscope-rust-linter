//! Finding the outcomes of a helper fn that pass on an error the helper is given, such as the
//! `return error;` and `DbError::Sql(error)` of a classifier that destructures its `error`
//! parameter with `let DbError::Sql(error) = error else { return error; };`.

use crate::diagnostic::Position;
use crate::syntax::span;

/// What a helper fn's signature and top-level `let` statements say about the errors it is given.
pub struct PassThrough {
    /// The name of each input of the helper's signature, by input index, or [`None`] for the
    /// receiver and for a pattern that isn't a plain identifier.
    param_names: Vec<Option<String>>,

    destructurings: Vec<Destructuring>,
}

impl PassThrough {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The pass-through facts of the helper with `sig` and `block`.
    pub fn new(sig: &syn::Signature, block: &syn::Block) -> Self {
        let param_names: Vec<Option<String>> = sig
            .inputs
            .iter()
            .map(|input| match input {
                syn::FnArg::Typed(pat_type) => match &*pat_type.pat {
                    syn::Pat::Ident(pat_ident) => Some(pat_ident.ident.to_string()),
                    _ => None,
                },
                syn::FnArg::Receiver(_) => None,
            })
            .collect();
        let destructurings = block
            .stmts
            .iter()
            .filter_map(|stmt| match stmt {
                syn::Stmt::Local(local) => Destructuring::of(local, &param_names),
                _ => None,
            })
            .collect();
        Self {
            param_names,
            destructurings,
        }
    }

    /// Decides whether `outcome`, a value the helper evaluates to, passes on one of its parameters:
    ///
    /// * The parameter itself passes it on, unless a destructuring that rebinds its name is in
    ///   scope.
    /// * A tuple variant built around the binding of a destructuring passes the parameter on when
    ///   the variant is the one the destructuring matched, such as `DbError::Sql(error)` after `let
    ///   DbError::Sql(error) = error else { .. };`.
    ///
    /// # Returns
    ///
    /// The input index of the parameter that `outcome` passes on, or [`None`] if it passes on none.
    pub fn passed_param(&self, outcome: &syn::Expr) -> Option<usize> {
        match outcome {
            syn::Expr::Path(expr_path) => {
                let ident = expr_path.path.get_ident()?;
                let name = ident.to_string();
                let position = span::start_position(ident.span());
                let is_rebound = self.destructurings.iter().any(|destructuring| {
                    destructuring.binding == name && destructuring.scope_start <= position
                });
                if is_rebound {
                    return None;
                }
                self.param_names
                    .iter()
                    .position(|param_name| param_name.as_deref() == Some(name.as_str()))
            }
            syn::Expr::Call(call) => {
                let syn::Expr::Path(func) = &*call.func else {
                    return None;
                };
                let [syn::Expr::Path(arg)] = call.args.iter().collect::<Vec<_>>().as_slice() else {
                    return None;
                };
                let ident = arg.path.get_ident()?;
                let name = ident.to_string();
                let position = span::start_position(ident.span());
                let variant = segment_names(&func.path);
                self.destructurings
                    .iter()
                    .rev()
                    .find(|destructuring| {
                        destructuring.binding == name && destructuring.scope_start <= position
                    })
                    .filter(|destructuring| destructuring.variant == variant)
                    .map(|destructuring| destructuring.param_index)
            }
            _ => None,
        }
    }
}

/// # Returns
///
/// The values that `expr` may evaluate to, looking through parentheses, blocks, `if` and `match`. A
/// `return` expression is left out, since its value is an outcome of its own.
pub fn outcome_leaves(expr: &syn::Expr) -> Vec<&syn::Expr> {
    let mut leaf_collector = LeafCollector { leaves: Vec::new() };
    leaf_collector.collect(expr);
    leaf_collector.leaves
}

/// A top-level `let Variant(binding) = param` statement of a helper, with or without `else`.
struct Destructuring {
    /// The segment names of the matched tuple variant's path, such as `["DbError", "Sql"]`.
    variant: Vec<String>,

    binding: String,

    /// The input index of the destructured parameter.
    param_index: usize,

    /// The position just past the statement, where the binding comes into scope.
    scope_start: Position,
}

impl Destructuring {
    /// # Returns
    ///
    /// The destructuring of `local`, or [`None`] if it doesn't destructure a parameter named in
    /// `param_names` with a one-field tuple variant pattern.
    fn of(local: &syn::Local, param_names: &[Option<String>]) -> Option<Self> {
        let syn::Pat::TupleStruct(pat_tuple_struct) = &local.pat else {
            return None;
        };
        let [syn::Pat::Ident(pat_ident)] = pat_tuple_struct.elems.iter().collect::<Vec<_>>()[..]
        else {
            return None;
        };
        let syn::Expr::Path(init) = &*local.init.as_ref()?.expr else {
            return None;
        };
        let param_name = init.path.get_ident()?.to_string();
        let param_index = param_names
            .iter()
            .position(|name| name.as_deref() == Some(param_name.as_str()))?;
        Some(Self {
            variant: segment_names(&pat_tuple_struct.path),
            binding: pat_ident.ident.to_string(),
            param_index,
            scope_start: span::start_position(local.semi_token.span),
        })
    }
}

/// Collects the leaves of [`outcome_leaves`].
///
/// # Type Parameters
///
/// * `'ast` - The lifetime of the syntax tree the leaves are in.
struct LeafCollector<'ast> {
    leaves: Vec<&'ast syn::Expr>,
}

impl<'ast> LeafCollector<'ast> {
    /// Adds the leaves of `expr`.
    fn collect(&mut self, expr: &'ast syn::Expr) {
        match expr {
            syn::Expr::Paren(expr_paren) => self.collect(&expr_paren.expr),
            syn::Expr::Block(expr_block) => self.collect_block(&expr_block.block),
            syn::Expr::Unsafe(expr_unsafe) => self.collect_block(&expr_unsafe.block),
            syn::Expr::If(expr_if) => {
                self.collect_block(&expr_if.then_branch);
                if let Some((_, else_branch)) = &expr_if.else_branch {
                    self.collect(else_branch);
                }
            }
            syn::Expr::Match(expr_match) => {
                for arm in &expr_match.arms {
                    self.collect(&arm.body);
                }
            }
            syn::Expr::Return(_) => {}
            _ => self.leaves.push(expr),
        }
    }

    /// Adds the leaves of `block`'s tail expression, if it has one.
    fn collect_block(&mut self, block: &'ast syn::Block) {
        if let Some(syn::Stmt::Expr(tail, None)) = block.stmts.last() {
            self.collect(tail);
        }
    }
}

/// # Returns
///
/// The identifiers of `path`'s segments.
fn segment_names(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use syn::visit::Visit;

    use super::*;
    use crate::rules::errors_doc::file_index::ExitCollector;

    /// # Returns
    ///
    /// The input index that each outcome leaf of `source_text`'s only fn passes on, in the order of
    /// its tail's leaves and then its `return` values.
    fn passed_params(source_text: &str) -> Vec<Option<usize>> {
        let syn::Item::Fn(item_fn) = &syn::parse_file(source_text).expect("valid file").items[0]
        else {
            panic!("the first item isn't a fn");
        };
        let pass_through = PassThrough::new(&item_fn.sig, &item_fn.block);
        let mut exit_collector = ExitCollector::default();
        exit_collector.visit_block(&item_fn.block);
        let tail = match item_fn.block.stmts.last() {
            Some(syn::Stmt::Expr(tail, None)) => Some(tail),
            _ => None,
        };
        tail.into_iter()
            .chain(exit_collector.returned)
            .flat_map(outcome_leaves)
            .map(|leaf| pass_through.passed_param(leaf))
            .collect()
    }

    #[test]
    fn destructured_parameters_rewrapped_in_their_variant_are_passed_on() {
        let source_text =
            "fn classify(chunk: &[u8], error: DbError) -> DbError {\n    let DbError::Sql(error) \
             = error else {\n        return error;\n    };\n    if rejected(&error) {\n        \
             return DbError::PayloadRejected(error);\n    }\n    match probe() {\n        Ok(_) \
             => DbError::Other(error),\n        _ => DbError::Sql(error),\n    }\n}";
        assert_eq!(passed_params(source_text), [None, Some(1), Some(1), None]);
    }

    #[test]
    fn parameters_returned_unchanged_are_passed_on_until_rebound() {
        let source_text = "fn classify(&self, error: Error) -> Error {\n    if retryable(&error) \
                           {\n        return error;\n    }\n    let Error::Io(error) = error else \
                           { return Error::Fatal; };\n    error\n}";
        assert_eq!(passed_params(source_text), [None, Some(1), None]);
    }
}
