//! Deciding whether code is test-only: `#[test]` and `#[cfg(test)]` attributes, and out-of-line
//! module files whose declaring file marks them `#[cfg(test)]`.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use proc_macro2::TokenStream;
use proc_macro2::TokenTree;

/// Decides, with caching across a run, whether a file is an out-of-line test module.
#[derive(Debug, Default)]
pub struct TestModuleCache {
    /// Each candidate declaring file that was looked at, by path.
    declaring_files: HashMap<PathBuf, DeclaringFile>,

    verdicts: HashMap<PathBuf, TestModuleVerdict>,
}

impl TestModuleCache {
    /// A file is a test module when its declaring file has a top-level `mod name;` item with
    /// `#[cfg(test)]`, or when the declaring file is itself a test module.
    ///
    /// The declaring file is the first existing candidate that declares the module:
    ///
    /// * For `D/S.rs`: `D/mod.rs`, `D/lib.rs`, `D/main.rs` and `D.rs`, declaring `S`.
    /// * For `D/mod.rs`: the same candidates one directory up, declaring `D`'s last component.
    /// * `lib.rs`, `main.rs` and the files directly under `src/bin/` are crate roots.
    ///
    /// A candidate that doesn't exist is skipped. A candidate that exists but can't be read or
    /// parsed is skipped too, and the verdict reports it as a problem.
    ///
    /// # Returns
    ///
    /// Whether `path` is an out-of-line test module, and the problem that may make that wrong.
    pub fn verdict(&mut self, path: &Path) -> TestModuleVerdict {
        if let Some(verdict) = self.verdicts.get(path) {
            return verdict.clone();
        }
        self.verdicts
            .insert(path.to_path_buf(), TestModuleVerdict::default());
        let verdict = self.compute_verdict(path);
        self.verdicts.insert(path.to_path_buf(), verdict.clone());
        verdict
    }

    /// Decides the verdict of `path` from its candidate declaring files, as [`Self::verdict`]
    /// describes.
    ///
    /// # Returns
    ///
    /// The verdict of `path`.
    fn compute_verdict(&mut self, path: &Path) -> TestModuleVerdict {
        let Some(candidates) = declaring_candidates(path) else {
            return TestModuleVerdict::default();
        };
        let mut problem: Option<String> = None;
        for candidate in candidates.paths {
            let is_cfg_test = match self.declaring_file(&candidate) {
                DeclaringFile::Missing => continue,
                DeclaringFile::Unusable { reason } => {
                    problem.get_or_insert_with(|| reason.clone());
                    continue;
                }
                DeclaringFile::Declarations { modules } => {
                    match modules.get(&candidates.module_name) {
                        Some(is_cfg_test) => *is_cfg_test,
                        None => continue,
                    }
                }
            };
            if is_cfg_test {
                return TestModuleVerdict {
                    is_test_module: true,
                    problem,
                };
            }
            let parent_verdict = self.verdict(&candidate);
            return TestModuleVerdict {
                is_test_module: parent_verdict.is_test_module,
                problem: problem.or(parent_verdict.problem),
            };
        }
        TestModuleVerdict {
            is_test_module: false,
            problem,
        }
    }

    /// Reads and parses `candidate` once per run.
    ///
    /// # Returns
    ///
    /// What the candidate holds.
    fn declaring_file(&mut self, candidate: &Path) -> &DeclaringFile {
        self.declaring_files
            .entry(candidate.to_path_buf())
            .or_insert_with(|| DeclaringFile::read(candidate))
    }
}

/// Whether a file is an out-of-line test module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestModuleVerdict {
    pub is_test_module: bool,

    /// Why a candidate declaring file couldn't be used, if one couldn't.
    pub problem: Option<String>,
}

/// # Returns
///
/// Whether `attrs` hold a `#[cfg(test)]` or a `#[cfg(all(.., test, ..))]` attribute.
pub fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        syn::Meta::List(meta_list) if meta_list.path.is_ident("cfg") => {
            cfg_requires_test(&meta_list.tokens)
        }
        _ => false,
    })
}

/// # Returns
///
/// Whether `attr` marks a test fn: its path's last segment is `test`.
pub fn is_test_attribute(attr: &syn::Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "test")
}

/// What a candidate declaring file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeclaringFile {
    /// The file doesn't exist.
    Missing,

    /// The file exists but can't be read or parsed, for `reason`.
    Unusable { reason: String },

    /// The file's top-level `mod name;` declarations, mapped to whether each has `#[cfg(test)]`.
    Declarations { modules: HashMap<String, bool> },
}

impl DeclaringFile {
    /// Reads and parses `candidate`.
    ///
    /// # Returns
    ///
    /// What the candidate holds.
    fn read(candidate: &Path) -> Self {
        let source_text = match fs::read_to_string(candidate) {
            Ok(source_text) => source_text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Self::Missing,
            Err(error) => {
                return Self::Unusable {
                    reason: format!("failed to read `{}`: {error}", candidate.display()),
                };
            }
        };
        let syntax_tree = match syn::parse_file(&source_text) {
            Ok(syntax_tree) => syntax_tree,
            Err(error) => {
                return Self::Unusable {
                    reason: format!("syn failed to parse `{}` ({error})", candidate.display()),
                };
            }
        };
        let modules = syntax_tree
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Mod(item_mod) if item_mod.content.is_none() => {
                    Some((item_mod.ident.to_string(), has_cfg_test(&item_mod.attrs)))
                }
                _ => None,
            })
            .collect();
        Self::Declarations { modules }
    }
}

/// The files that may declare a module file, and the module name they would declare.
struct DeclaringCandidates {
    paths: Vec<PathBuf>,
    module_name: String,
}

/// # Returns
///
/// Whether the cfg predicate `tokens` can only hold under `cfg(test)`: it is `test`, or an `all`
/// with such a predicate.
fn cfg_requires_test(tokens: &TokenStream) -> bool {
    let token_trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
    match token_trees.as_slice() {
        [TokenTree::Ident(ident)] => ident == "test",
        [TokenTree::Ident(ident), TokenTree::Group(group)] if ident == "all" => {
            let mut predicate = TokenStream::new();
            for token_tree in group.stream() {
                if matches!(&token_tree, TokenTree::Punct(punct) if punct.as_char() == ',') {
                    if cfg_requires_test(&predicate) {
                        return true;
                    }
                    predicate = TokenStream::new();
                } else {
                    predicate.extend([token_tree]);
                }
            }
            cfg_requires_test(&predicate)
        }
        _ => false,
    }
}

/// # Returns
///
/// The candidate declaring files of the module file `path`, or [`None`] if `path` is a crate root.
fn declaring_candidates(path: &Path) -> Option<DeclaringCandidates> {
    let file_stem = path.file_stem()?.to_str()?;
    let directory = path.parent()?;
    if matches!(file_stem, "lib" | "main") || is_bin_directory(directory) {
        return None;
    }
    let (directory, module_name) = if file_stem == "mod" {
        (directory.parent()?, directory.file_name()?.to_str()?)
    } else {
        (directory, file_stem)
    };
    let mut paths: Vec<PathBuf> = ["mod.rs", "lib.rs", "main.rs"]
        .into_iter()
        .map(|file_name| directory.join(file_name))
        .collect();
    if let Some(directory_name) = directory.file_name() {
        paths.push(directory.with_file_name(format!("{}.rs", directory_name.to_str()?)));
    }
    Some(DeclaringCandidates {
        paths,
        module_name: module_name.to_owned(),
    })
}

/// # Returns
///
/// Whether `directory` is a crate's `src/bin` directory.
fn is_bin_directory(directory: &Path) -> bool {
    directory.file_name() == Some(OsStr::new("bin"))
        && directory
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "src")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(source_text: &str) -> Vec<syn::Attribute> {
        let item_fn: syn::ItemFn = syn::parse_str(source_text).expect("valid fn");
        item_fn.attrs
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("a file path has a parent"))
            .expect("failed to create the parent directory");
        fs::write(path, contents).expect("failed to write the test file");
    }

    fn test_directory(test_name: &str) -> PathBuf {
        let directory = std::env::temp_dir()
            .join(format!(
                "yscope-rust-linter-test-modules-{}",
                std::process::id()
            ))
            .join(test_name);
        if directory.exists() {
            fs::remove_dir_all(&directory).expect("failed to clear the test directory");
        }
        directory
    }

    #[test]
    fn cfg_test_attributes_are_recognized() {
        assert!(has_cfg_test(&attrs("#[cfg(test)] fn f() {}")));
        assert!(has_cfg_test(&attrs(
            "#[cfg(all(feature = \"x\", test))] fn f() {}"
        )));
        assert!(!has_cfg_test(&attrs(
            "#[cfg(any(test, feature = \"x\"))] fn f() {}"
        )));
        assert!(!has_cfg_test(&attrs("#[cfg(not(test))] fn f() {}")));
        assert!(!has_cfg_test(&attrs("#[test] fn f() {}")));
    }

    #[test]
    fn module_files_declared_with_cfg_test_are_test_modules() {
        let directory = test_directory("declared");
        let src = directory.join("src");
        write_file(
            &src.join("lib.rs"),
            "mod a;\nmod c;\n#[cfg(test)]\nmod d;\n#[cfg(test)]\nmod tests;\n",
        );
        write_file(&src.join("tests.rs"), "mod nested;\n");
        write_file(&src.join("tests/nested.rs"), "");
        write_file(&src.join("a.rs"), "#[cfg(test)]\nmod a_tests;\nmod b;\n");
        write_file(&src.join("a/a_tests.rs"), "");
        write_file(&src.join("a/b/mod.rs"), "");
        write_file(&src.join("bin/tool.rs"), "");
        write_file(
            &src.join("c/mod.rs"),
            "#[cfg(test)]\nmod c_tests;\nmod c_impl;\n",
        );
        write_file(&src.join("c/c_tests.rs"), "");
        write_file(&src.join("c/c_impl.rs"), "");
        write_file(&src.join("d/mod.rs"), "");
        write_file(
            &directory.join("app/main.rs"),
            "#[cfg(test)]\nmod app_tests;\n",
        );
        write_file(&directory.join("app/app_tests.rs"), "");

        let mut cache = TestModuleCache::default();
        let mut is_test_module = |relative_path: &str| {
            let verdict = cache.verdict(&src.join(relative_path));
            assert_eq!(verdict.problem, None, "{relative_path}");
            verdict.is_test_module
        };
        assert!(is_test_module("tests.rs"));
        assert!(is_test_module("tests/nested.rs"));
        assert!(is_test_module("a/a_tests.rs"));
        assert!(!is_test_module("a.rs"));
        assert!(!is_test_module("a/b/mod.rs"));
        assert!(!is_test_module("lib.rs"));
        assert!(!is_test_module("bin/tool.rs"));
        assert!(!is_test_module("undeclared.rs"));
        assert!(is_test_module("c/c_tests.rs"));
        assert!(!is_test_module("c/c_impl.rs"));
        assert!(!is_test_module("c/mod.rs"));
        assert!(is_test_module("d/mod.rs"));
        assert!(is_test_module("../app/app_tests.rs"));
        fs::remove_dir_all(&directory).expect("failed to remove the test directory");
    }

    #[test]
    fn unparsable_declaring_files_are_reported_as_problems() {
        let directory = test_directory("unparsable");
        let src = directory.join("src");
        write_file(
            &src.join("lib.rs"),
            "#[cfg(test)]\nmod tests;\nfn broken( {}\n",
        );
        write_file(&src.join("tests.rs"), "");

        let verdict = TestModuleCache::default().verdict(&src.join("tests.rs"));
        assert!(!verdict.is_test_module);
        assert!(
            verdict
                .problem
                .as_deref()
                .is_some_and(|problem| problem.starts_with("syn failed to parse")),
            "{verdict:?}"
        );
        fs::remove_dir_all(&directory).expect("failed to remove the test directory");
    }
}
