//! Looking through `syn` types to the paths they name.

/// # Returns
///
/// `ty` without parentheses and invisible groups.
pub fn peel_type_parens(mut ty: &syn::Type) -> &syn::Type {
    loop {
        ty = match ty {
            syn::Type::Paren(type_paren) => &type_paren.elem,
            syn::Type::Group(type_group) => &type_group.elem,
            _ => return ty,
        };
    }
}

/// # Returns
///
/// The last segment of `ty` if it is a path, or [`None`] otherwise.
pub fn type_last_segment(ty: &syn::Type) -> Option<String> {
    match peel_type_parens(ty) {
        syn::Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}
