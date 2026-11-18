//! The sort keys that order the items of a scope, and the reasons two keys are in order.

use crate::syntax::visibility::VisibilityGroup;

/// The sort key of an item in a scope. The scope is in order when the keys of its items don't
/// decrease.
pub trait SortKey: Copy + Ord {
    /// Explains why an item comes before another one, by the first key component that differs.
    ///
    /// # Parameters
    ///
    /// * `smaller` - The key of the item that comes first.
    /// * `larger` - The key of the item that comes second, which is larger than `smaller`.
    ///
    /// # Returns
    ///
    /// The reason, such as `pub items come before private items`.
    fn reason(smaller: Self, larger: Self) -> String;
}

/// The kind of a module item, ordered as the convention lists the kinds within a visibility group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Const,
    Static,

    /// Structs, enums, unions, traits, trait aliases and type aliases, and the impl blocks of
    /// structs, enums and unions.
    Type {
        /// The index of the type in the module's items, shared by its impl blocks.
        anchor: usize,

        impl_rank: ImplRank,
    },

    Fn,
}

impl Category {
    /// # Returns
    ///
    /// The category's name in the plural, as reasons spell it.
    const fn plural(self) -> &'static str {
        match self {
            Self::Const => "consts",
            Self::Static => "statics",
            Self::Type { .. } => "types and their impls",
            Self::Fn => "fns",
        }
    }
}

/// The place of a type or one of its impl blocks among the type and its impl blocks, ordered as the
/// convention lists them: the type, its inherent impls, the impls of crate traits, then the impls
/// of external traits. The derived `Ord` relies on this declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImplRank {
    /// The type itself.
    Type,

    Inherent,
    CrateTrait,
    ExternalTrait,
}

impl ImplRank {
    /// # Returns
    ///
    /// The impl blocks of the rank in the plural, as reasons spell them.
    const fn plural(self) -> &'static str {
        match self {
            Self::Type => "types",
            Self::Inherent => "inherent impls",
            Self::CrateTrait => "impls of crate traits",
            Self::ExternalTrait => "impls of external traits",
        }
    }
}

/// The sort key of an item in a module. The derived `Ord` compares the fields in declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleKey {
    pub group: VisibilityGroup,
    pub category: Category,
}

impl SortKey for ModuleKey {
    fn reason(smaller: Self, larger: Self) -> String {
        if smaller.group != larger.group {
            return format!(
                "{} items come before {} items",
                smaller.group.as_str(),
                larger.group.as_str()
            );
        }
        match (smaller.category, larger.category) {
            (
                Category::Type {
                    anchor: smaller_anchor,
                    impl_rank: smaller_impl_rank,
                },
                Category::Type {
                    anchor: larger_anchor,
                    impl_rank: larger_impl_rank,
                },
            ) => {
                if smaller_anchor != larger_anchor || smaller_impl_rank == ImplRank::Type {
                    "a type's impl blocks directly follow it".to_owned()
                } else {
                    format!(
                        "{} come before {}",
                        smaller_impl_rank.plural(),
                        larger_impl_rank.plural()
                    )
                }
            }
            (smaller_category, larger_category) => format!(
                "{} come before {}",
                smaller_category.plural(),
                larger_category.plural()
            ),
        }
    }
}

/// The sort key of an item in an inherent impl block. The derived `Ord` compares the fields in
/// declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ImplKey {
    category: ImplItemCategory,
    group: VisibilityGroup,
}

impl ImplKey {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The key of an associated const, which doesn't depend on its visibility.
    pub const fn associated_const() -> Self {
        Self {
            category: ImplItemCategory::Const,
            group: VisibilityGroup::Pub,
        }
    }

    /// Factory function.
    ///
    /// # Returns
    ///
    /// The key of an associated fn in the visibility group `group`.
    pub const fn associated_fn(group: VisibilityGroup) -> Self {
        Self {
            category: ImplItemCategory::Fn,
            group,
        }
    }
}

impl SortKey for ImplKey {
    fn reason(smaller: Self, larger: Self) -> String {
        if smaller.category == larger.category {
            format!(
                "{} fns come before {} fns",
                smaller.group.as_str(),
                larger.group.as_str()
            )
        } else {
            "associated consts come before fns".to_owned()
        }
    }
}

/// The kind of an item in an inherent impl block, ordered as the convention lists the kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ImplItemCategory {
    Const,
    Fn,
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_KEY: ModuleKey = ModuleKey {
        group: VisibilityGroup::Pub,
        category: Category::Type {
            anchor: 1,
            impl_rank: ImplRank::Type,
        },
    };

    #[test]
    fn module_reasons_name_the_first_differing_component() {
        let reasons: Vec<String> = [
            ModuleKey {
                group: VisibilityGroup::Private,
                category: Category::Const,
            },
            ModuleKey {
                category: Category::Fn,
                ..BASE_KEY
            },
            ModuleKey {
                category: Category::Type {
                    anchor: 2,
                    impl_rank: ImplRank::Inherent,
                },
                ..BASE_KEY
            },
            ModuleKey {
                category: Category::Type {
                    anchor: 1,
                    impl_rank: ImplRank::ExternalTrait,
                },
                ..BASE_KEY
            },
        ]
        .into_iter()
        .map(|larger| ModuleKey::reason(BASE_KEY, larger))
        .collect();
        assert_eq!(
            reasons,
            [
                "pub items come before private items",
                "types and their impls come before fns",
                "a type's impl blocks directly follow it",
                "a type's impl blocks directly follow it",
            ]
        );

        let [inherent, crate_trait, external_trait] = [
            ImplRank::Inherent,
            ImplRank::CrateTrait,
            ImplRank::ExternalTrait,
        ]
        .map(|impl_rank| ModuleKey {
            category: Category::Type {
                anchor: 1,
                impl_rank,
            },
            ..BASE_KEY
        });
        assert_eq!(
            [
                ModuleKey::reason(inherent, crate_trait),
                ModuleKey::reason(crate_trait, external_trait),
                ModuleKey::reason(inherent, external_trait),
            ],
            [
                "inherent impls come before impls of crate traits",
                "impls of crate traits come before impls of external traits",
                "inherent impls come before impls of external traits",
            ]
        );
    }

    #[test]
    fn impl_keys_order_consts_first_then_fns_by_visibility() {
        let private_const = ImplKey::associated_const();
        let pub_fn = ImplKey::associated_fn(VisibilityGroup::Pub);
        let private_fn = ImplKey::associated_fn(VisibilityGroup::Private);
        assert!(private_const < pub_fn && pub_fn < private_fn);
        assert_eq!(
            [
                ImplKey::reason(private_const, private_fn),
                ImplKey::reason(pub_fn, private_fn),
            ],
            [
                "associated consts come before fns",
                "pub fns come before private fns"
            ]
        );
    }
}
