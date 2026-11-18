//! Recognizing the macros the body scanner handles specially, and parsing macro tokens into the
//! expressions and statements it scans.

use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use syn::Token;
use syn::parse::ParseStream;
use syn::parse::Parser;
use syn::punctuated::Punctuated;

/// How the body scanner treats a macro invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroKind {
    /// `bail!`, `ensure!`, `anyhow!` and `format_err!`, which build an `anyhow::Error`.
    Anyhow,

    /// `select!`, whose branches are scanned as part of the enclosing fn.
    Select,

    /// `try_join!`, which returns the first error of the futures it awaits.
    TryJoin,

    /// `stream!` and `try_stream!`, whose body runs in the stream they build, like an async block.
    Generator,

    /// `matches!` and the `assert_matches!` family, whose pattern is not scanned.
    PatternTaking,

    /// Any other macro, scanned on a best-effort basis.
    Other,
}

/// The expressions of a macro invocation that the body scanner visits.
#[derive(Default)]
pub struct MacroBody {
    pub exprs: Vec<syn::Expr>,
    pub stmts: Vec<syn::Stmt>,

    /// The arm bodies of a `select!`, whose values are the macro's value.
    pub arm_bodies: Vec<syn::Expr>,
}

/// # Returns
///
/// The kind of `mac`, by its path's last segment.
pub fn kind(mac: &syn::Macro) -> MacroKind {
    let Some(last_segment) = mac.path.segments.last() else {
        return MacroKind::Other;
    };
    let name = last_segment.ident.to_string();
    let is_unqualified_or_anyhow = mac.path.segments.len() == 1
        || mac
            .path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == "anyhow");
    match name.as_str() {
        "bail" | "ensure" | "anyhow" | "format_err" if is_unqualified_or_anyhow => {
            MacroKind::Anyhow
        }
        "select" => MacroKind::Select,
        "try_join" => MacroKind::TryJoin,
        "stream" | "try_stream" => MacroKind::Generator,
        "matches" | "assert_matches" | "debug_assert_matches" => MacroKind::PatternTaking,
        _ => MacroKind::Other,
    }
}

/// Parses the tokens of `mac` according to its kind:
///
/// * [`MacroKind::Select`]: the grammar `[biased;] (pat = expr [, if cond] => body [,])* [else =>
///   body [,]]`. The future expressions and guards are kept as expressions, the bodies as arm
///   bodies, and the patterns are dropped.
/// * [`MacroKind::PatternTaking`]: an expression, a comma, a pattern and an optional `if` guard.
///   The expression and the guard are kept.
/// * Every other kind: a comma-separated list of expressions, or else a block's statements.
///
/// # Returns
///
/// The parsed body, or [`None`] if the tokens don't parse.
pub fn parse_body(mac: &syn::Macro, macro_kind: MacroKind) -> Option<MacroBody> {
    let tokens = mac.tokens.clone();
    match macro_kind {
        MacroKind::Select => {
            parse_select
                .parse2(tokens)
                .ok()
                .map(|(exprs, arm_bodies)| MacroBody {
                    exprs,
                    arm_bodies,
                    ..MacroBody::default()
                })
        }
        MacroKind::PatternTaking => {
            parse_pattern_taking
                .parse2(tokens)
                .ok()
                .map(|exprs| MacroBody {
                    exprs,
                    ..MacroBody::default()
                })
        }
        MacroKind::Anyhow | MacroKind::TryJoin | MacroKind::Generator | MacroKind::Other => {
            if let Ok(exprs) =
                Punctuated::<syn::Expr, Token![,]>::parse_terminated.parse2(tokens.clone())
            {
                return Some(MacroBody {
                    exprs: exprs.into_iter().collect(),
                    ..MacroBody::default()
                });
            }
            syn::Block::parse_within
                .parse2(tokens)
                .ok()
                .map(|stmts| MacroBody {
                    stmts,
                    ..MacroBody::default()
                })
        }
    }
}

/// # Returns
///
/// Every identifier in `tokens`, including the ones in nested groups.
pub fn identifiers(tokens: &TokenStream) -> Vec<syn::Ident> {
    let mut found = Vec::new();
    for token_tree in tokens.clone() {
        match token_tree {
            TokenTree::Ident(ident) => found.push(ident),
            TokenTree::Group(group) => found.extend(identifiers(&group.stream())),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
    found
}

/// # Returns
///
/// Whether `tokens` hold a postfix `?`, one that follows an identifier or a parenthesized or
/// bracketed group, including in nested groups. A `?` after `=` or `,`, such as in
/// `tracing::info!(id = ?id)`, is a formatting sigil and doesn't count.
pub fn has_try_operator(tokens: &TokenStream) -> bool {
    let mut previous: Option<TokenTree> = None;
    for token_tree in tokens.clone() {
        let is_try = match (&token_tree, &previous) {
            (TokenTree::Punct(punct), Some(TokenTree::Ident(_))) => punct.as_char() == '?',
            (TokenTree::Punct(punct), Some(TokenTree::Group(group))) => {
                punct.as_char() == '?' && group.delimiter() != proc_macro2::Delimiter::Brace
            }
            (TokenTree::Group(group), _) => has_try_operator(&group.stream()),
            _ => false,
        };
        if is_try {
            return true;
        }
        previous = Some(token_tree);
    }
    false
}

/// Parses the branches of a `select!` invocation.
///
/// # Returns
///
/// The future expressions and guards, in order, and the arm bodies, in order.
///
/// # Errors
///
/// Returns an error if:
///
/// * Forwards [`syn::Pat::parse_multi_with_leading_vert`]'s return values on failure.
/// * Forwards [`syn::parse::ParseBuffer::parse`]'s return values on failure.
fn parse_select(input: ParseStream<'_>) -> syn::Result<(Vec<syn::Expr>, Vec<syn::Expr>)> {
    let mut exprs = Vec::new();
    let mut arm_bodies = Vec::new();
    if input.peek(syn::Ident) && input.peek2(Token![;]) {
        let _: syn::Ident = input.parse()?;
        let _: Token![;] = input.parse()?;
    }
    while !input.is_empty() {
        if input.peek(Token![else]) {
            let _: Token![else] = input.parse()?;
        } else {
            let _ = syn::Pat::parse_multi_with_leading_vert(input)?;
            let _: Token![=] = input.parse()?;
            exprs.push(input.parse()?);
            if input.peek(Token![,]) && input.peek2(Token![if]) {
                let _: Token![,] = input.parse()?;
                let _: Token![if] = input.parse()?;
                exprs.push(input.parse()?);
            }
        }
        let _: Token![=>] = input.parse()?;
        arm_bodies.push(input.parse()?);
        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        }
    }
    Ok((exprs, arm_bodies))
}

/// Parses the arguments of a pattern-taking macro such as `matches!`.
///
/// # Returns
///
/// The scrutinee expression and the guard, if any.
///
/// # Errors
///
/// Returns an error if:
///
/// * Forwards [`syn::Pat::parse_multi_with_leading_vert`]'s return values on failure.
/// * Forwards [`syn::parse::ParseBuffer::parse`]'s return values on failure.
fn parse_pattern_taking(input: ParseStream<'_>) -> syn::Result<Vec<syn::Expr>> {
    let mut exprs = vec![input.parse()?];
    let _: Token![,] = input.parse()?;
    let _ = syn::Pat::parse_multi_with_leading_vert(input)?;
    if input.peek(Token![if]) {
        let _: Token![if] = input.parse()?;
        exprs.push(input.parse()?);
    }
    let _: TokenStream = input.parse()?;
    Ok(exprs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn macro_of(source_text: &str) -> syn::Macro {
        syn::parse_str(source_text).expect("valid macro invocation")
    }

    /// # Returns
    ///
    /// The number of expressions, statements and arm bodies of the parsed macro, or [`None`] if it
    /// doesn't parse.
    fn num_exprs(source_text: &str) -> Option<usize> {
        let mac = macro_of(source_text);
        parse_body(&mac, kind(&mac))
            .map(|body| body.exprs.len() + body.stmts.len() + body.arm_bodies.len())
    }

    #[test]
    fn macros_are_classified_by_their_last_segment() {
        let cases = [
            ("bail!(\"x\")", MacroKind::Anyhow),
            ("anyhow::ensure!(x, \"y\")", MacroKind::Anyhow),
            ("other::bail!(x)", MacroKind::Other),
            ("tokio::select! {}", MacroKind::Select),
            ("matches!(x, Y)", MacroKind::PatternTaking),
            ("debug_assert_ne!(a, b)", MacroKind::Other),
            ("tokio::try_join!(a(), b())", MacroKind::TryJoin),
            (
                "async_stream::try_stream! { yield x?; }",
                MacroKind::Generator,
            ),
            ("vec![a]", MacroKind::Other),
        ];
        for (source_text, expected) in cases {
            assert_eq!(
                kind(&macro_of(source_text)),
                expected,
                "kind of `{source_text}`"
            );
        }
    }

    #[test]
    fn select_branches_keep_futures_guards_and_bodies() {
        let source_text = "tokio::select! { biased; Some(m) = rx.recv(), if open => { \
                           self.handle(m).await?; } _ = token.cancelled() => break, else => \
                           return Ok(()), }";
        assert_eq!(num_exprs(source_text), Some(6));
    }

    #[test]
    fn pattern_taking_macros_drop_the_pattern() {
        assert_eq!(num_exprs("matches!(r, Err(X::Y(_)) if ready)"), Some(2));
        assert_eq!(
            num_exprs("assert_matches!(r, Ok(_), \"message {}\", x)"),
            Some(1)
        );
    }

    #[test]
    fn other_macros_parse_as_expressions_or_statements() {
        assert_eq!(num_exprs("vec![foo()?, bar]"), Some(2));
        assert_eq!(num_exprs("vec![foo(); 3]"), Some(2));
        assert_eq!(num_exprs("tracing::info!(id = ?id, \"text\")"), None);
        assert_eq!(num_exprs("json!({ \"key\": value })"), None);
        let mac = macro_of("json!({ \"key\": value })");
        assert_eq!(
            identifiers(&mac.tokens)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["value"]
        );
    }

    #[test]
    fn only_postfix_question_marks_are_try_operators() {
        for (source_text, has_try) in [
            ("m!(load()?)", true),
            ("m!(value?)", true),
            ("m!(x[0]?)", true),
            ("m!({ inner()? })", true),
            ("tracing::info!(id = ?id, \"x\")", false),
            ("tracing::info!(?id, \"x\")", false),
            ("m!(a, b)", false),
        ] {
            let mac = macro_of(source_text);
            assert_eq!(has_try_operator(&mac.tokens), has_try, "{source_text}");
        }
    }
}
