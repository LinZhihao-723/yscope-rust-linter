//! Parsing a unified diff (`git diff -U0`) into the changed lines of each `.rs` file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::Error;
use crate::error::InvalidDiffReason;
use crate::line_ranges::LineRanges;
use crate::line_ranges::LineSpan;

/// Parses a unified diff into the lines it touches on the new side of each `.rs` file.
///
/// The lines each hunk selects:
///
/// * A hunk that adds `d > 0` lines at new line `c` selects `c..=c+d-1`. With context lines (a diff
///   made without `-U0`), these include the context.
/// * A pure deletion after new line `c` selects `max(c, 1)..=c+1`, the lines on both sides of the
///   deleted text.
///
/// Files deleted by the diff (`+++ /dev/null`) and files not ending in `.rs` are skipped.
///
/// # Returns
///
/// A map from each changed `.rs` file's path, relative to the diff's root, to its selected lines on
/// success.
///
/// # Errors
///
/// Returns an error if:
///
/// * [`Error::InvalidDiff`] if:
///   * A `+++` path is quoted.
///   * A `+++` path has neither the `b/` prefix nor the `/dev/null` form.
///   * A hunk header is malformed, or appears before any `+++` line.
///   * A hunk header's line number or count overflows.
pub fn parse(diff_text: &str) -> Result<BTreeMap<PathBuf, LineRanges>, Error> {
    let mut changed_lines: BTreeMap<PathBuf, LineRanges> = BTreeMap::new();
    let mut current_file: Option<NewFile> = None;
    let mut hunk: Option<Hunk> = None;

    for (index, line) in diff_text.lines().enumerate() {
        let line_number = index + 1;
        let invalid = |reason| Error::InvalidDiff {
            line_number,
            reason,
        };

        if let Some(active_hunk) = hunk.as_mut() {
            if active_hunk.consume(line) {
                if active_hunk.is_done() {
                    hunk = None;
                }
                continue;
            }
            hunk = None;
        }

        if let Some(path_text) = line.strip_prefix("+++ ") {
            current_file = Some(NewFile::parse(path_text).map_err(invalid)?);
        } else if line.starts_with("@@") {
            let Some(file) = current_file.as_ref() else {
                return Err(invalid(InvalidDiffReason::HunkOutsideFile));
            };
            let header = HunkHeader::parse(line).map_err(invalid)?;
            if let NewFile::Path { path } = file
                && path.extension().is_some_and(|extension| extension == "rs")
            {
                changed_lines
                    .entry(path.clone())
                    .or_default()
                    .insert(header.selected_span);
            }
            let new_hunk = Hunk {
                remaining_old_lines: header.num_old_lines,
                remaining_new_lines: header.num_new_lines,
            };
            hunk = (!new_hunk.is_done()).then_some(new_hunk);
        }
    }
    Ok(changed_lines)
}

/// The hunk being read, counting the content lines it still has on each side.
#[derive(Debug, Clone, Copy)]
struct Hunk {
    remaining_old_lines: usize,
    remaining_new_lines: usize,
}

impl Hunk {
    /// Consumes `line` if it is one of the hunk's content lines: a `-` line, a `+` line, a context
    /// line (a leading space) or a `\ No newline at end of file` marker.
    ///
    /// # Returns
    ///
    /// Whether `line` belongs to the hunk.
    const fn consume(&mut self, line: &str) -> bool {
        match line.as_bytes().first() {
            Some(b'-') if self.remaining_old_lines > 0 => self.remaining_old_lines -= 1,
            Some(b'+') if self.remaining_new_lines > 0 => self.remaining_new_lines -= 1,
            Some(b' ') if self.remaining_old_lines > 0 && self.remaining_new_lines > 0 => {
                self.remaining_old_lines -= 1;
                self.remaining_new_lines -= 1;
            }
            Some(b'\\') => {}
            _ => return false,
        }
        true
    }

    /// # Returns
    ///
    /// Whether the hunk has no content lines left on either side.
    const fn is_done(self) -> bool {
        self.remaining_old_lines == 0 && self.remaining_new_lines == 0
    }
}

/// The new-side file named by a `+++` line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NewFile {
    /// `+++ /dev/null`: the diff deletes the file.
    DevNull,

    /// `+++ b/<path>`.
    Path { path: PathBuf },
}

impl NewFile {
    /// Parses the path of a `+++` line.
    ///
    /// # Returns
    ///
    /// The file the path names on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`InvalidDiffReason::QuotedPath`] if the path is quoted.
    /// * [`InvalidDiffReason::MissingPathPrefix`] if the path has neither the `b/` prefix nor the
    ///   `/dev/null` form.
    fn parse(path_text: &str) -> Result<Self, InvalidDiffReason> {
        if path_text.starts_with('"') {
            return Err(InvalidDiffReason::QuotedPath);
        }
        if path_text == "/dev/null" {
            return Ok(Self::DevNull);
        }
        path_text
            .strip_prefix("b/")
            .map(|path| Self::Path {
                path: PathBuf::from(path),
            })
            .ok_or(InvalidDiffReason::MissingPathPrefix)
    }
}

/// A parsed hunk header: the number of lines on each side, and the new-side lines it selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HunkHeader {
    num_old_lines: usize,
    num_new_lines: usize,
    selected_span: LineSpan,
}

impl HunkHeader {
    /// Parses a `@@ -a[,b] +c[,d] @@` hunk header, where a missing count means 1.
    ///
    /// # Returns
    ///
    /// The parsed header on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`InvalidDiffReason::MalformedHunkHeader`] if:
    ///   * `line` doesn't have the `@@ -a[,b] +c[,d] @@` form.
    ///   * The new side has lines but starts at line 0.
    /// * [`InvalidDiffReason::LineNumberOverflow`] if the new side's last line overflows `usize`.
    /// * Forwards [`HunkRange::parse`]'s return values on failure.
    fn parse(line: &str) -> Result<Self, InvalidDiffReason> {
        let malformed = InvalidDiffReason::MalformedHunkHeader;
        let mut fields = line.strip_prefix("@@ ").ok_or(malformed)?.split(' ');
        let old_range_text = fields
            .next()
            .and_then(|field| field.strip_prefix('-'))
            .ok_or(malformed)?;
        let new_range_text = fields
            .next()
            .and_then(|field| field.strip_prefix('+'))
            .ok_or(malformed)?;
        if fields.next() != Some("@@") {
            return Err(malformed);
        }
        let old_range = HunkRange::parse(old_range_text)?;
        let HunkRange { start, count } = HunkRange::parse(new_range_text)?;
        let (first, last) = match count.checked_sub(1) {
            None => (start.max(1), start.checked_add(1)),
            Some(_) if start == 0 => return Err(malformed),
            Some(num_later_lines) => (start, start.checked_add(num_later_lines)),
        };
        let last = last.ok_or(InvalidDiffReason::LineNumberOverflow)?;
        Ok(Self {
            num_old_lines: old_range.count,
            num_new_lines: count,
            selected_span: LineSpan::new(first, last).ok_or(malformed)?,
        })
    }
}

/// One side of a hunk header: `start[,count]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HunkRange {
    start: usize,
    count: usize,
}

impl HunkRange {
    /// Parses the `start[,count]` part of a hunk header, where a missing count means 1.
    ///
    /// # Returns
    ///
    /// The parsed range on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * Forwards [`parse_number`]'s return values on failure.
    fn parse(text: &str) -> Result<Self, InvalidDiffReason> {
        let (start, count) = match text.split_once(',') {
            Some((start, count)) => (parse_number(start)?, parse_number(count)?),
            None => (parse_number(text)?, 1),
        };
        Ok(Self { start, count })
    }
}

/// Parses a decimal number of a hunk header.
///
/// # Returns
///
/// The number on success.
///
/// # Errors
///
/// Returns an error if:
///
/// * [`InvalidDiffReason::MalformedHunkHeader`] if `text` is empty or has a character other than an
///   ASCII digit.
/// * [`InvalidDiffReason::LineNumberOverflow`] if the number overflows `usize`.
fn parse_number(text: &str) -> Result<usize, InvalidDiffReason> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(InvalidDiffReason::MalformedHunkHeader);
    }
    text.parse()
        .map_err(|_| InvalidDiffReason::LineNumberOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(text: &str) -> LineRanges {
        text.parse().expect("valid ranges")
    }

    #[test]
    fn hunks_select_added_lines_and_both_sides_of_deletions() {
        let diff_text = "\
diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -3,0 +4,2 @@ fn f() {
+    let a = 1;
+    let b = 2;
@@ -10 +12 @@
-old
+new
@@ -20,2 +23,0 @@
-gone
-gone too
@@ -1 +0,0 @@
-first line
";
        let changed_lines = parse(diff_text).expect("valid diff");
        assert_eq!(
            changed_lines,
            BTreeMap::from([(PathBuf::from("src/a.rs"), ranges("1,4-5,12,23-24"))])
        );
    }

    #[test]
    fn content_lines_that_look_like_headers_stay_in_their_hunk() {
        let diff_text = "\
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,2 @@
--- not a header
-@@ not a hunk
+++ b/not-a-file.rs
+@@ -1 +1 @@
";
        let changed_lines = parse(diff_text).expect("valid diff");
        assert_eq!(
            changed_lines,
            BTreeMap::from([(PathBuf::from("a.rs"), ranges("1-2"))])
        );
    }

    #[test]
    fn deleted_and_non_rust_files_are_skipped() {
        let diff_text = "\
--- a/gone.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-a
-b
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-x
+y
--- a/src/b.rs
+++ b/src/b.rs
@@ -5 +5,3 @@
-x
+y
+z
+w
\\ No newline at end of file
";
        let changed_lines = parse(diff_text).expect("valid diff");
        assert_eq!(
            changed_lines,
            BTreeMap::from([(PathBuf::from("src/b.rs"), ranges("5-7"))])
        );
    }

    #[test]
    fn context_lines_stay_in_their_hunk_and_are_selected() {
        let diff_text = "\
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,3 @@
 context
+++ added line that looks like a header
-removed
+added
@@ -9 +10 @@
-x
+y
";
        let changed_lines = parse(diff_text).expect("valid diff");
        assert_eq!(
            changed_lines,
            BTreeMap::from([(PathBuf::from("a.rs"), ranges("1-3,10"))])
        );
    }

    #[test]
    fn empty_diff_has_no_files() {
        assert_eq!(parse("").expect("valid diff"), BTreeMap::new());
    }

    #[test]
    fn invalid_diffs_are_rejected_with_their_line_number() {
        let cases = [
            ("+++ \"b/a b.rs\"\n", 1, InvalidDiffReason::QuotedPath),
            ("+++ a.rs\n", 1, InvalidDiffReason::MissingPathPrefix),
            (
                "+++ b/a.rs\n@@ -1 +x @@\n",
                2,
                InvalidDiffReason::MalformedHunkHeader,
            ),
            (
                "+++ b/a.rs\n@@ -1 +1\n",
                2,
                InvalidDiffReason::MalformedHunkHeader,
            ),
            ("@@ -1 +1 @@\n", 1, InvalidDiffReason::HunkOutsideFile),
            (
                "+++ b/a.rs\n@@ -1 +0 @@\n",
                2,
                InvalidDiffReason::MalformedHunkHeader,
            ),
            (
                "+++ b/a.rs\n@@ -1 +0,1 @@\n",
                2,
                InvalidDiffReason::MalformedHunkHeader,
            ),
            (
                "+++ b/a.rs\n@@ -1 +18446744073709551615,2 @@\n",
                2,
                InvalidDiffReason::LineNumberOverflow,
            ),
            (
                "+++ b/a.rs\n@@ -1 +18446744073709551615,0 @@\n",
                2,
                InvalidDiffReason::LineNumberOverflow,
            ),
            (
                "+++ b/a.rs\n@@ -1 +99999999999999999999999 @@\n",
                2,
                InvalidDiffReason::LineNumberOverflow,
            ),
        ];
        for (diff_text, expected_line_number, expected_reason) in cases {
            let error = parse(diff_text).expect_err("invalid diff");
            assert!(
                matches!(
                    error,
                    Error::InvalidDiff { line_number, reason }
                        if line_number == expected_line_number && reason == expected_reason
                ),
                "{diff_text:?}: {error:?}"
            );
        }
    }
}
