//! Turning targets, diff ranges and excludes into the list of files to lint.

use std::collections::BTreeMap;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use crate::error::Error;
use crate::line_ranges::LineRanges;
use crate::line_ranges::LineRangesParseError;

/// A positional target: a file or a directory, optionally with line ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSpec {
    pub path: PathBuf,
    pub ranges: Option<LineRanges>,
}

impl FromStr for TargetSpec {
    type Err = Error;

    /// Parses a positional target argument.
    ///
    /// The argument is `FILE:RANGES` when the text after its last `:` has the `N[-M](,N[-M])*`
    /// form. Otherwise, the whole argument is a path.
    ///
    /// # Returns
    ///
    /// The parsed target on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`Error::InvalidLineRange`] if the ranges have the right form, but one starts at 0 or ends
    ///   before it starts.
    /// * [`Error::LineNumberOverflow`] if the ranges have the right form, but a line number
    ///   overflows `usize`.
    fn from_str(argument: &str) -> Result<Self, Self::Err> {
        if let Some((path, ranges_text)) = argument.rsplit_once(':') {
            match ranges_text.parse::<LineRanges>() {
                Ok(ranges) => {
                    return Ok(Self {
                        path: PathBuf::from(path),
                        ranges: Some(ranges),
                    });
                }
                Err(LineRangesParseError::InvalidRange { first, last }) => {
                    return Err(Error::InvalidLineRange {
                        argument: argument.to_owned(),
                        first,
                        last,
                    });
                }
                Err(LineRangesParseError::Overflow) => {
                    return Err(Error::LineNumberOverflow {
                        argument: argument.to_owned(),
                    });
                }
                Err(LineRangesParseError::Syntax) => {}
            }
        }
        Ok(Self {
            path: PathBuf::from(argument),
            ranges: None,
        })
    }
}

/// The lines of a file whose diagnostics are reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineFilter {
    /// Every line.
    All,

    /// Only diagnostics whose anchor span intersects these lines.
    Ranges(LineRanges),
}

impl LineFilter {
    /// Widens the filter to also keep everything `other` keeps.
    pub fn union(&mut self, other: Self) {
        match (&mut *self, other) {
            (Self::All, _) => {}
            (Self::Ranges(_), Self::All) => *self = Self::All,
            (Self::Ranges(ranges), Self::Ranges(other_ranges)) => ranges.union(other_ranges),
        }
    }
}

/// A file to lint and the lines to report in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub path: PathBuf,
    pub line_filter: LineFilter,
}

/// Collects the files to lint.
///
/// The rules for each kind of input:
///
/// * A directory target is walked recursively for `.rs` files. Directories named `target`, hidden
///   directories and directory symlinks are skipped.
/// * A file target is linted even when its name doesn't end in `.rs` or it sits inside a directory
///   the walk would skip.
/// * Each file of `diff_ranges` is linted on its changed lines.
/// * A file listed by several inputs gets the union of their line filters.
/// * A file under any of `excludes`, compared by whole path components, is dropped.
///
/// # Returns
///
/// The files to lint, sorted by their normalized path, on success.
///
/// # Errors
///
/// Returns an error if:
///
/// * [`Error::TargetNotFound`] if a target doesn't exist.
/// * [`Error::RangesOnDirectory`] if a directory target has line ranges.
/// * [`Error::Io`] if a target's metadata can't be read.
/// * Forwards [`walk_directory`]'s return values on failure.
pub fn discover(
    targets: Vec<TargetSpec>,
    diff_ranges: BTreeMap<PathBuf, LineRanges>,
    excludes: &[PathBuf],
) -> Result<Vec<SourceFile>, Error> {
    let excludes: Vec<PathBuf> = excludes.iter().map(|exclude| normalize(exclude)).collect();
    let mut line_filters: BTreeMap<PathBuf, LineFilter> = BTreeMap::new();
    let mut add_file = |path: PathBuf, line_filter: LineFilter| {
        let path = normalize(&path);
        if excludes.iter().any(|exclude| path.starts_with(exclude)) {
            return;
        }
        match line_filters.get_mut(&path) {
            Some(existing) => existing.union(line_filter),
            None => {
                line_filters.insert(path, line_filter);
            }
        }
    };

    for TargetSpec { path, ranges } in targets {
        let metadata = fs::metadata(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::TargetNotFound { path: path.clone() }
            } else {
                Error::Io {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        if metadata.is_dir() {
            if ranges.is_some() {
                return Err(Error::RangesOnDirectory { path });
            }
            let mut walked_files = Vec::new();
            walk_directory(&path, &mut walked_files)?;
            for walked_file in walked_files {
                add_file(walked_file, LineFilter::All);
            }
        } else {
            add_file(path, ranges.map_or(LineFilter::All, LineFilter::Ranges));
        }
    }
    for (path, ranges) in diff_ranges {
        add_file(path, LineFilter::Ranges(ranges));
    }

    Ok(line_filters
        .into_iter()
        .map(|(path, line_filter)| SourceFile { path, line_filter })
        .collect())
}

/// # Returns
///
/// Whether one of the first five lines of `source_text` is a plain `//` comment with the word
/// `@generated`, such as `// This file is @generated by prost-build.`.
pub fn is_generated(source_text: &str) -> bool {
    source_text
        .lines()
        .take(GENERATED_MARKER_LINES)
        .filter_map(|line| line.trim_start().strip_prefix("//"))
        .filter(|comment_text| {
            !comment_text.starts_with(['/', '!']) || comment_text.starts_with("//")
        })
        .any(|comment_text| {
            comment_text.split_whitespace().any(|word| {
                word.trim_end_matches(|current: char| current.is_ascii_punctuation())
                    == "@generated"
            })
        })
}

/// The number of leading lines searched for the `@generated` marker.
const GENERATED_MARKER_LINES: usize = 5;

/// Appends every `.rs` file under `directory` to `files`, in sorted order within each directory.
///
/// # Errors
///
/// Returns an error if:
///
/// * [`Error::Io`] if the directory's entries, or an entry's file type, can't be read.
fn walk_directory(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), Error> {
    let io_error = |source| Error::Io {
        path: directory.to_path_buf(),
        source,
    };
    let mut entries = fs::read_dir(directory)
        .map_err(io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_error)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "target" || name.starts_with('.') {
                continue;
            }
            walk_directory(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && (file_type.is_file() || path.is_file())
        {
            files.push(path);
        }
    }
    Ok(())
}

/// Normalizes a path lexically by dropping its `.` components, so `./src/a.rs` becomes `src/a.rs`.
///
/// # Returns
///
/// The normalized path, or `.` if nothing is left.
fn normalize(path: &Path) -> PathBuf {
    let normalized: PathBuf = path
        .components()
        .filter(|component| *component != Component::CurDir)
        .collect();
    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_with_range_syntax_after_the_last_colon_has_ranges() {
        let target = TargetSpec::from_str("src/a.rs:10-20,30").expect("valid target");
        assert_eq!(target.path, PathBuf::from("src/a.rs"));
        assert_eq!(
            target.ranges,
            Some("10-20,30".parse().expect("valid ranges"))
        );
    }

    #[test]
    fn target_without_range_syntax_after_the_colon_is_a_path() {
        for argument in ["src/a.rs", "dir:name/a.rs", "a.rs:", "a.rs:x", "a.rs:1-2:b"] {
            let target = TargetSpec::from_str(argument).expect("valid target");
            assert_eq!(target.path, PathBuf::from(argument));
            assert_eq!(target.ranges, None);
        }
    }

    #[test]
    fn target_with_invalid_range_is_rejected() {
        let error = TargetSpec::from_str("a.rs:5-3").expect_err("reversed range");
        assert!(
            matches!(
                &error,
                Error::InvalidLineRange { argument, first: 5, last: 3 } if argument == "a.rs:5-3"
            ),
            "{error:?}"
        );
        let error = TargetSpec::from_str("a.rs:0").expect_err("zero line");
        assert!(
            matches!(
                error,
                Error::InvalidLineRange {
                    first: 0,
                    last: 0,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn line_filter_union_prefers_all_and_merges_ranges() {
        let ranges = |text: &str| LineFilter::Ranges(text.parse().expect("valid ranges"));
        let mut line_filter = ranges("1-3");
        line_filter.union(ranges("10"));
        assert_eq!(line_filter, ranges("1-3,10"));
        line_filter.union(LineFilter::All);
        assert_eq!(line_filter, LineFilter::All);
        line_filter.union(ranges("4"));
        assert_eq!(line_filter, LineFilter::All);
    }

    #[test]
    fn generated_marker_is_a_word_of_a_plain_comment_in_the_first_five_lines() {
        assert!(is_generated(
            "// This file is @generated by prost-build.\nfn main() {}\n"
        ));
        assert!(is_generated("\n\n\n\n// @generated\n"));
        assert!(is_generated("//// @generated.\n"));
        assert!(!is_generated("\n\n\n\n\n// @generated\n"));
        assert!(!is_generated("//! Lints files marked @generated.\n"));
        assert!(!is_generated("/// Skips @generated files.\n"));
        assert!(!is_generated("let marker = \"// @generated\";\n"));
        assert!(!is_generated("// Not@generated here.\n"));
    }

    #[test]
    fn target_with_overflowing_line_number_is_rejected() {
        let error = TargetSpec::from_str("a.rs:99999999999999999999999").expect_err("overflow");
        assert!(
            matches!(
                &error,
                Error::LineNumberOverflow { argument } if argument.starts_with("a.rs:")
            ),
            "{error:?}"
        );
    }

    #[test]
    fn normalize_drops_current_directory_components() {
        assert_eq!(
            normalize(Path::new("./src/./a.rs")),
            PathBuf::from("src/a.rs")
        );
        assert_eq!(normalize(Path::new(".")), PathBuf::from("."));
        assert_eq!(normalize(Path::new("../x")), PathBuf::from("../x"));
    }

    /// Creates `relative_paths` as empty files under a fresh directory named after `test_name`.
    ///
    /// # Returns
    ///
    /// The directory's path.
    ///
    /// # Panics
    ///
    /// Panics if the directory or a file can't be created.
    fn fixture_directory(test_name: &str, relative_paths: &[&str]) -> PathBuf {
        let directory = std::env::temp_dir()
            .join(format!("yscope-rust-linter-source-{}", std::process::id()))
            .join(test_name);
        if directory.exists() {
            fs::remove_dir_all(&directory).expect("failed to clear the test directory");
        }
        for relative_path in relative_paths {
            let path = directory.join(relative_path);
            fs::create_dir_all(path.parent().expect("a file path has a parent"))
                .expect("failed to create the parent directory");
            fs::write(path, "").expect("failed to write the test file");
        }
        directory
    }

    fn target(path: PathBuf, ranges: Option<&str>) -> TargetSpec {
        TargetSpec {
            path,
            ranges: ranges.map(|text| text.parse().expect("valid ranges")),
        }
    }

    #[test]
    fn discover_walks_directories_and_excludes_by_whole_components() {
        let directory = fixture_directory(
            "walk",
            &[
                "src/b.rs",
                "src/a.rs",
                "src/ab.rs",
                "src/a/x.rs",
                "src/a/notes.txt",
                "src/target/t.rs",
                "src/.hidden/h.rs",
            ],
        );
        let source_files = discover(
            vec![target(directory.join("src"), None)],
            BTreeMap::new(),
            &[directory.join("./src/a")],
        )
        .expect("valid targets");
        let paths: Vec<_> = source_files
            .iter()
            .map(|source_file| source_file.path.clone())
            .collect();
        assert_eq!(
            paths,
            ["src/a.rs", "src/ab.rs", "src/b.rs"].map(|path| directory.join(path))
        );
        fs::remove_dir_all(&directory).expect("failed to remove the test directory");
    }

    #[test]
    fn discover_unions_the_filters_of_a_file_listed_several_times() {
        let directory = fixture_directory("union", &["target/t.rs", "notes.txt", "src/a.rs"]);
        let skipped_file = directory.join("target/t.rs");
        let source_files = discover(
            vec![
                target(skipped_file.clone(), Some("1-2")),
                target(directory.join("notes.txt"), Some("3")),
                target(directory.join("./src/a.rs"), Some("1")),
                target(directory.join("src"), None),
            ],
            BTreeMap::from([(skipped_file, "5".parse().expect("valid ranges"))]),
            &[],
        )
        .expect("valid targets");
        let ranges = |text: &str| LineFilter::Ranges(text.parse().expect("valid ranges"));
        assert_eq!(
            source_files,
            [
                SourceFile {
                    path: directory.join("notes.txt"),
                    line_filter: ranges("3"),
                },
                SourceFile {
                    path: directory.join("src/a.rs"),
                    line_filter: LineFilter::All,
                },
                SourceFile {
                    path: directory.join("target/t.rs"),
                    line_filter: ranges("1-2,5"),
                },
            ]
        );
        fs::remove_dir_all(&directory).expect("failed to remove the test directory");
    }
}
