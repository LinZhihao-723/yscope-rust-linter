//! Classifying an item's visibility into the groups that the symbol-ordering convention lists.

/// The visibility group of an item, ordered as the convention lists the groups: `pub` first,
/// private last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VisibilityGroup {
    Pub,
    PubCrate,
    PubSuper,
    PubIn,
    Private,
}

impl VisibilityGroup {
    /// Classifies `visibility`:
    ///
    /// * `pub(crate)` and `pub(in crate)` are [`Self::PubCrate`].
    /// * `pub(super)` and `pub(in super)` are [`Self::PubSuper`].
    /// * `pub(self)`, `pub(in self)` and no visibility are [`Self::Private`].
    /// * Any other `pub(in path)` is [`Self::PubIn`].
    ///
    /// # Returns
    ///
    /// The group of `visibility`.
    pub fn of(visibility: &syn::Visibility) -> Self {
        match visibility {
            syn::Visibility::Public(_) => Self::Pub,
            syn::Visibility::Inherited => Self::Private,
            syn::Visibility::Restricted(restricted) => {
                if restricted.path.is_ident("crate") {
                    Self::PubCrate
                } else if restricted.path.is_ident("super") {
                    Self::PubSuper
                } else if restricted.path.is_ident("self") {
                    Self::Private
                } else {
                    Self::PubIn
                }
            }
        }
    }

    /// # Returns
    ///
    /// The group's name as the convention spells it: `pub`, `pub(crate)`, `pub(super)`, `pub(in
    /// path)` or `private`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pub => "pub",
            Self::PubCrate => "pub(crate)",
            Self::PubSuper => "pub(super)",
            Self::PubIn => "pub(in path)",
            Self::Private => "private",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(source_text: &str) -> VisibilityGroup {
        let item_fn: syn::ItemFn = syn::parse_str(source_text).expect("valid fn");
        VisibilityGroup::of(&item_fn.vis)
    }

    #[test]
    fn visibilities_are_classified_into_ordered_groups() {
        let groups: Vec<VisibilityGroup> = [
            "pub fn f() {}",
            "pub(crate) fn f() {}",
            "pub(in crate) fn f() {}",
            "pub(super) fn f() {}",
            "pub(in super) fn f() {}",
            "pub(in crate::a) fn f() {}",
            "pub(in super::super) fn f() {}",
            "pub(self) fn f() {}",
            "fn f() {}",
        ]
        .into_iter()
        .map(group)
        .collect();
        assert_eq!(
            groups,
            [
                VisibilityGroup::Pub,
                VisibilityGroup::PubCrate,
                VisibilityGroup::PubCrate,
                VisibilityGroup::PubSuper,
                VisibilityGroup::PubSuper,
                VisibilityGroup::PubIn,
                VisibilityGroup::PubIn,
                VisibilityGroup::Private,
                VisibilityGroup::Private,
            ]
        );
        assert!(groups.is_sorted());
        assert_eq!(
            groups
                .iter()
                .map(|group| group.as_str())
                .collect::<Vec<_>>(),
            [
                "pub",
                "pub(crate)",
                "pub(crate)",
                "pub(super)",
                "pub(super)",
                "pub(in path)",
                "pub(in path)",
                "private",
                "private"
            ]
        );
    }
}
