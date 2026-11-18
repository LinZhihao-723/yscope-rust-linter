//! The rules that decide when a symbol path named by a doc link and one named by a fn body name the
//! same symbol.

use std::collections::HashSet;
use std::hash::Hash;

use crate::syntax::symbol_path::SymbolPath;

/// The names a fn's paths are matched under: the names of `Self` and the generic type parameters in
/// scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolScope {
    self_names: HashSet<String>,
    generic_params: HashSet<String>,
}

impl SymbolScope {
    /// Factory function.
    ///
    /// `Self` is always one of the self names.
    ///
    /// # Returns
    ///
    /// A scope whose `Self` set holds `self_names` and whose generic type parameters are
    /// `generic_params`.
    pub fn new(
        self_names: impl IntoIterator<Item = String>,
        generic_params: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut self_names: HashSet<String> = self_names.into_iter().collect();
        self_names.insert("Self".to_owned());
        Self {
            self_names,
            generic_params: generic_params.into_iter().collect(),
        }
    }

    /// Drops the qualifier of a path whose qualifier is a generic type parameter, so `T::new`
    /// becomes `new`.
    ///
    /// # Returns
    ///
    /// The normalized path.
    pub fn normalize(&self, path: SymbolPath) -> SymbolPath {
        match path.qualifier() {
            Some(qualifier) if self.generic_params.contains(qualifier) => {
                SymbolPath::single(path.last())
            }
            _ => path,
        }
    }

    /// # Returns
    ///
    /// Whether `name` is `Self`, the impl's self type, or the implemented or enclosing trait.
    pub fn is_self_name(&self, name: &str) -> bool {
        self.self_names.contains(name)
    }

    /// Two paths match exactly when their last segments are equal and, if both are qualified, their
    /// qualifiers are equal. Two qualifiers are equal when they are the same identifier or both are
    /// names of `Self`.
    ///
    /// # Returns
    ///
    /// Whether `left` and `right` match exactly.
    pub fn matches_exactly(&self, left: &SymbolPath, right: &SymbolPath) -> bool {
        if left.last() != right.last() {
            return false;
        }
        match (left.qualifier(), right.qualifier()) {
            (Some(left_qualifier), Some(right_qualifier)) => {
                left_qualifier == right_qualifier
                    || (self.self_names.contains(left_qualifier)
                        && self.self_names.contains(right_qualifier))
            }
            _ => true,
        }
    }

    /// A mention matches a documented path when some prefix of the mention matches the documented
    /// path exactly, so the mention `syn::Error::new_spanned` matches the doc path `syn::Error`.
    ///
    /// # Returns
    ///
    /// Whether `mention` matches `documented`.
    pub fn matches_mention(&self, documented: &SymbolPath, mention: &SymbolPath) -> bool {
        mention
            .prefixes()
            .any(|prefix| self.matches_exactly(documented, &prefix))
    }

    /// Pairs body facts with documented paths in two passes:
    ///
    /// 1. Every fact and documented path that match exactly are paired.
    /// 2. The facts left without a pair are paired with the documented paths left without one when
    ///    their last segments are equal and `qualifier_links` links their qualifiers.
    ///
    /// Each fact and documented path carries a key, and several of them may share one.
    ///
    /// # Type Parameters
    ///
    /// * `FactKeyType` - The key of a body fact.
    /// * `DocKeyType` - The key of a documented path.
    ///
    /// # Returns
    ///
    /// The keys of the facts and of the documented paths that were paired.
    pub fn pair<FactKeyType: Copy + Eq + Hash, DocKeyType: Copy + Eq + Hash>(
        &self,
        facts: &[KeyedPath<'_, FactKeyType>],
        documented: &[KeyedPath<'_, DocKeyType>],
        qualifier_links: &QualifierLinks,
    ) -> Pairing<FactKeyType, DocKeyType> {
        let mut paired_facts = HashSet::new();
        let mut paired_docs = HashSet::new();
        for (fact_index, fact) in facts.iter().enumerate() {
            for (doc_index, doc) in documented.iter().enumerate() {
                if self.matches_exactly(fact.path, doc.path) {
                    paired_facts.insert(fact_index);
                    paired_docs.insert(doc_index);
                }
            }
        }
        let unpaired_docs: Vec<usize> = (0..documented.len())
            .filter(|doc_index| !paired_docs.contains(doc_index))
            .collect();
        for (fact_index, fact) in facts.iter().enumerate() {
            if paired_facts.contains(&fact_index) {
                continue;
            }
            for doc_index in &unpaired_docs {
                let doc_path = documented[*doc_index].path;
                if doc_path.last() == fact.path.last()
                    && let (Some(fact_qualifier), Some(doc_qualifier)) =
                        (fact.path.qualifier(), doc_path.qualifier())
                    && qualifier_links.are_linked(fact_qualifier, doc_qualifier)
                {
                    paired_facts.insert(fact_index);
                    paired_docs.insert(*doc_index);
                }
            }
        }
        Pairing {
            paired_facts: paired_facts
                .into_iter()
                .map(|index| facts[index].key)
                .collect(),
            paired_docs: paired_docs
                .into_iter()
                .map(|index| documented[index].key)
                .collect(),
        }
    }
}

/// The file-level names under which two different qualifiers may name the same symbol.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QualifierLinks {
    /// The names that may be a trait providing a method that another type's path names.
    trait_names: HashSet<String>,

    /// The `(original, alias)` pairs of renames and type aliases.
    aliases: HashSet<(String, String)>,
}

impl QualifierLinks {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The links of `trait_names` and of the `(original, alias)` pairs of `aliases`.
    pub fn new(
        trait_names: impl IntoIterator<Item = String>,
        aliases: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            trait_names: trait_names.into_iter().collect(),
            aliases: aliases.into_iter().collect(),
        }
    }

    /// Two different qualifiers are linked in these cases:
    ///
    /// * One of them is a trait name, since `Type::method` may call the trait's method.
    /// * One is an alias of the other.
    ///
    /// # Returns
    ///
    /// Whether `left` and `right` are linked.
    pub fn are_linked(&self, left: &str, right: &str) -> bool {
        self.trait_names.contains(left)
            || self.trait_names.contains(right)
            || [(left, right), (right, left)]
                .into_iter()
                .any(|(original, alias)| {
                    self.aliases
                        .contains(&(original.to_owned(), alias.to_owned()))
                })
    }
}

/// A body fact or a documented path to pair, with the key that the pairing reports it by.
///
/// # Type Parameters
///
/// * `'path` - The lifetime of the path.
/// * `KeyType` - The type of the key.
#[derive(Debug, Clone, Copy)]
pub struct KeyedPath<'path, KeyType> {
    pub key: KeyType,
    pub path: &'path SymbolPath,
}

/// The result of pairing body facts with documented paths.
///
/// # Type Parameters
///
/// * `FactKeyType` - The type of the body facts' keys.
/// * `DocKeyType` - The type of the documented paths' keys.
#[derive(Debug, Clone)]
pub struct Pairing<FactKeyType, DocKeyType> {
    /// The keys of the facts paired with a documented path.
    pub paired_facts: HashSet<FactKeyType>,

    /// The keys of the documented paths paired with a fact.
    pub paired_docs: HashSet<DocKeyType>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> SymbolPath {
        SymbolPath::parse(text).expect("valid symbol path")
    }

    fn scope() -> SymbolScope {
        SymbolScope::new(
            ["Server".to_owned(), "Service".to_owned()],
            ["StorageType".to_owned()],
        )
    }

    #[test]
    fn qualifier_is_checked_only_when_both_sides_are_qualified() {
        let scope = scope();
        assert!(scope.matches_exactly(&path("send"), &path("DeleteBuilder::send")));
        assert!(scope.matches_exactly(&path("Job::start"), &path("job::Job::start")));
        assert!(!scope.matches_exactly(&path("Job::start"), &path("Task::start")));
        assert!(!scope.matches_exactly(&path("start"), &path("stop")));
    }

    #[test]
    fn self_set_holds_self_impl_type_and_trait_names() {
        let scope = scope();
        assert!(scope.matches_exactly(&path("Self::load"), &path("Server::load")));
        assert!(scope.matches_exactly(&path("Service::load"), &path("Self::load")));
        assert!(scope.matches_exactly(&path("Server::load"), &path("Service::load")));
        assert!(!scope.matches_exactly(&path("Self::load"), &path("Client::load")));
    }

    #[test]
    fn generic_type_parameter_qualifiers_are_dropped() {
        let scope = scope();
        assert_eq!(
            scope.normalize(path("StorageType::connect")),
            path("connect")
        );
        assert_eq!(
            scope.normalize(path("Server::connect")),
            path("Server::connect")
        );
    }

    fn keyed_by_index(paths: &[SymbolPath]) -> Vec<KeyedPath<'_, usize>> {
        paths
            .iter()
            .enumerate()
            .map(|(key, path)| KeyedPath { key, path })
            .collect()
    }

    /// # Returns
    ///
    /// The indices of the facts and documented paths that `scope` pairs, sorted.
    fn paired(
        facts: &[&str],
        documented: &[&str],
        qualifier_links: &QualifierLinks,
    ) -> (Vec<usize>, Vec<usize>) {
        let fact_paths: Vec<SymbolPath> = facts.iter().map(|text| path(text)).collect();
        let doc_paths: Vec<SymbolPath> = documented.iter().map(|text| path(text)).collect();
        let pairing = scope().pair(
            &keyed_by_index(&fact_paths),
            &keyed_by_index(&doc_paths),
            qualifier_links,
        );
        let mut paired_facts: Vec<usize> = pairing.paired_facts.into_iter().collect();
        let mut paired_docs: Vec<usize> = pairing.paired_docs.into_iter().collect();
        paired_facts.sort_unstable();
        paired_docs.sort_unstable();
        (paired_facts, paired_docs)
    }

    #[test]
    fn second_pass_pairs_linked_qualifiers_without_cross_pairing() {
        let trait_links = QualifierLinks::new(["YamlConfig".to_owned()], []);
        assert_eq!(
            paired(
                &["ServerConfig::from_yaml_file"],
                &["YamlConfig::from_yaml_file"],
                &trait_links
            ),
            (vec![0], vec![0])
        );
        let alias_links = QualifierLinks::new([], [("Error".to_owned(), "CacheError".to_owned())]);
        assert_eq!(
            paired(
                &["Error::NotFound"],
                &["CacheError::NotFound"],
                &alias_links
            ),
            (vec![0], vec![0])
        );
        assert_eq!(
            paired(
                &["A::new", "B::new"],
                &["A::new", "B::new", "C::new"],
                &trait_links
            ),
            (vec![0, 1], vec![0, 1])
        );
    }

    #[test]
    fn second_pass_leaves_unlinked_qualifiers_unpaired() {
        assert_eq!(
            paired(
                &["Storage::load"],
                &["Cache::load"],
                &QualifierLinks::default()
            ),
            (vec![], vec![])
        );
    }

    #[test]
    fn prefix_matching_is_for_mentions_only() {
        let scope = scope();
        let mention = path("syn::Error::new_spanned");
        assert!(scope.matches_mention(&path("syn::Error"), &mention));
        assert!(!scope.matches_exactly(&path("syn::Error"), &mention));
        assert!(!scope.matches_mention(&path("io::Error"), &mention));
    }
}
