//! Telling, from one file's syntax, whether a path names an item of the crate being linted rather
//! than an item of another crate.

use std::collections::HashSet;

use syn::visit::Visit;

/// The names through which one file refers to items of its own crate.
#[derive(Debug)]
pub struct CrateNames {
    names: HashSet<String>,
}

impl CrateNames {
    /// Collects the crate-local names of `syntax_tree`, in any module of the file:
    ///
    /// * The names of the traits and modules the file defines or declares.
    /// * The names that a `use` imports from a path that starts with `crate`, `self` or `super`, or
    ///   with one of the file's own modules. A renamed import contributes its new name, and `self`
    ///   in a `use` group contributes the module it names. Glob imports contribute nothing.
    ///
    /// # Returns
    ///
    /// The file's crate-local names.
    pub fn collect(syntax_tree: &syn::File) -> Self {
        let mut definition_collector = DefinitionCollector {
            names: HashSet::new(),
        };
        definition_collector.visit_file(syntax_tree);
        let mut use_collector = UseCollector {
            defined_names: &definition_collector.names,
            names: HashSet::new(),
        };
        use_collector.visit_file(syntax_tree);
        let mut names = use_collector.names;
        names.extend(definition_collector.names);
        Self { names }
    }

    /// Decides whether `path` names an item of the crate: its first segment is `crate`, `self` or
    /// `super`, or one of the file's crate-local names. A path with a leading `::` names another
    /// crate.
    ///
    /// # Returns
    ///
    /// Whether `path` is crate-local.
    pub fn is_crate_path(&self, path: &syn::Path) -> bool {
        if path.leading_colon.is_some() {
            return false;
        }
        path.segments.first().is_some_and(|segment| {
            let first = segment.ident.to_string();
            is_crate_root(&first) || self.names.contains(&first)
        })
    }
}

/// The `syn` visitor that collects the names of the traits and modules a file defines.
struct DefinitionCollector {
    names: HashSet<String>,
}

impl<'ast> Visit<'ast> for DefinitionCollector {
    fn visit_item_trait(&mut self, item_trait: &'ast syn::ItemTrait) {
        self.names.insert(item_trait.ident.to_string());
        syn::visit::visit_item_trait(self, item_trait);
    }

    fn visit_item_trait_alias(&mut self, item_trait_alias: &'ast syn::ItemTraitAlias) {
        self.names.insert(item_trait_alias.ident.to_string());
        syn::visit::visit_item_trait_alias(self, item_trait_alias);
    }

    fn visit_item_mod(&mut self, item_mod: &'ast syn::ItemMod) {
        self.names.insert(item_mod.ident.to_string());
        syn::visit::visit_item_mod(self, item_mod);
    }
}

/// The `syn` visitor that collects the names a file imports from its own crate.
///
/// # Type Parameters
///
/// * `'names` - The lifetime of the names of the file's traits and modules.
struct UseCollector<'names> {
    /// The names of the traits and modules the file defines, which a crate-local import may start
    /// with.
    defined_names: &'names HashSet<String>,

    names: HashSet<String>,
}

impl UseCollector<'_> {
    /// Adds the names that `use_tree` imports from the crate.
    ///
    /// * `locality` says whether the path before `use_tree` is crate-local. It is
    ///   [`PathLocality::Undecided`] at the root of the `use`, where the first segment of the path
    ///   decides it.
    /// * `parent` is the last segment before `use_tree`, which a `self` in a group names.
    fn add(&mut self, use_tree: &syn::UseTree, locality: PathLocality, parent: Option<&str>) {
        match use_tree {
            syn::UseTree::Path(use_path) => {
                let segment = use_path.ident.to_string();
                let locality = match locality {
                    PathLocality::Undecided => {
                        if is_crate_root(&segment) || self.defined_names.contains(&segment) {
                            PathLocality::CrateLocal
                        } else {
                            PathLocality::External
                        }
                    }
                    PathLocality::CrateLocal | PathLocality::External => locality,
                };
                self.add(&use_path.tree, locality, Some(&segment));
            }
            syn::UseTree::Name(use_name) => {
                let name = use_name.ident.to_string();
                let imported = if name == "self" {
                    parent.map(str::to_owned)
                } else {
                    Some(name)
                };
                if locality == PathLocality::CrateLocal
                    && let Some(imported) = imported
                {
                    self.names.insert(imported);
                }
            }
            syn::UseTree::Rename(use_rename) => {
                if locality == PathLocality::CrateLocal {
                    self.names.insert(use_rename.rename.to_string());
                }
            }
            syn::UseTree::Group(use_group) => {
                for item in &use_group.items {
                    self.add(item, locality, parent);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
}

impl<'ast> Visit<'ast> for UseCollector<'_> {
    fn visit_item_use(&mut self, item_use: &'ast syn::ItemUse) {
        let locality = if item_use.leading_colon.is_some() {
            PathLocality::External
        } else {
            PathLocality::Undecided
        };
        self.add(&item_use.tree, locality, None);
    }
}

/// Whether the path before a use tree names an item of the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathLocality {
    /// The use tree is the root of its `use`, so its first segment decides.
    Undecided,

    CrateLocal,
    External,
}

/// # Returns
///
/// Whether `segment` is a path root that names the current crate or one of its modules.
fn is_crate_root(segment: &str) -> bool {
    matches!(segment, "crate" | "self" | "super")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_paths(source_text: &str, paths: &[&str]) -> Vec<bool> {
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        let crate_names = CrateNames::collect(&syntax_tree);
        paths
            .iter()
            .map(|path| crate_names.is_crate_path(&syn::parse_str(path).expect("valid path")))
            .collect()
    }

    #[test]
    fn paths_rooted_at_the_crate_are_crate_local() {
        assert_eq!(
            crate_paths(
                "",
                &[
                    "crate::cache::Cache",
                    "self::Local",
                    "super::Parent",
                    "::std::fmt::Display",
                    "Display",
                    "std::fmt::Display",
                    "serde::Serialize",
                ]
            ),
            [true, true, true, false, false, false, false]
        );
    }

    #[test]
    fn defined_traits_and_modules_are_crate_local() {
        let source_text = "trait Defined {}\nmod declared;\nmod inline { pub trait Nested {} \
                           }\nstruct NotATrait;";
        assert_eq!(
            crate_paths(
                source_text,
                &[
                    "Defined",
                    "declared::Trait",
                    "Nested",
                    "inline::Nested",
                    "NotATrait"
                ]
            ),
            [true, true, true, true, false]
        );
    }

    #[test]
    fn imports_from_the_crate_are_crate_local() {
        let source_text = "use crate::cache::Cache;\nuse super::{storage::{self, Storage as \
                           Store}, Other};\nuse std::fmt::Display;\nuse \
                           ::crate_name::Foreign;\nuse crate::glob::*;\nmod local;\nuse \
                           local::Imported;\nfn f() { use self::Nested; }";
        assert_eq!(
            crate_paths(
                source_text,
                &[
                    "Cache",
                    "storage::Storage",
                    "Store",
                    "Storage",
                    "Other",
                    "Display",
                    "Foreign",
                    "Globbed",
                    "Imported",
                    "Nested",
                ]
            ),
            [
                true, true, true, false, true, false, false, false, true, true
            ]
        );
    }
}
