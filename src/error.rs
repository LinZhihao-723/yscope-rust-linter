//! The fatal errors of a lint run: invalid arguments, invalid diff input and I/O failures.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// A failure that stops the whole run before any diagnostic is printed.
#[derive(Debug)]
pub enum Error {
    /// A file, a directory or the diff input couldn't be read.
    Io { path: PathBuf, source: io::Error },

    /// The standard input couldn't be read as the diff.
    ReadStdin { source: io::Error },

    /// A target names a path that doesn't exist.
    TargetNotFound { path: PathBuf },

    /// A target attaches line ranges to a directory.
    RangesOnDirectory { path: PathBuf },

    /// A target has a line range that starts at line 0 or ends before it starts.
    InvalidLineRange {
        argument: String,
        first: usize,
        last: usize,
    },

    /// A target has a line number too large for `usize`.
    LineNumberOverflow { argument: String },

    /// A line of the diff input can't be parsed.
    InvalidDiff {
        line_number: usize,
        reason: InvalidDiffReason,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "failed to read `{}`: {source}", path.display())
            }
            Self::ReadStdin { source } => {
                write!(formatter, "failed to read the standard input: {source}")
            }
            Self::TargetNotFound { path } => {
                write!(formatter, "target `{}` does not exist", path.display())
            }
            Self::RangesOnDirectory { path } => write!(
                formatter,
                "target `{}` is a directory, so it can't take line ranges",
                path.display()
            ),
            Self::InvalidLineRange {
                argument,
                first,
                last,
            } => write!(
                formatter,
                "invalid line range `{first}-{last}` in `{argument}`: lines start at 1 and a \
                 range can't end before it starts"
            ),
            Self::LineNumberOverflow { argument } => {
                write!(formatter, "line number too large in `{argument}`")
            }
            Self::InvalidDiff {
                line_number,
                reason,
            } => write!(
                formatter,
                "invalid diff input at line {line_number}: {reason}"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::ReadStdin { source } => Some(source),
            Self::TargetNotFound { .. }
            | Self::RangesOnDirectory { .. }
            | Self::InvalidLineRange { .. }
            | Self::LineNumberOverflow { .. }
            | Self::InvalidDiff { .. } => None,
        }
    }
}

/// Why a line of the diff input was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidDiffReason {
    /// The `+++` path is quoted, which git does for names with special characters.
    QuotedPath,

    /// The `+++` path has neither the `b/` prefix nor the `/dev/null` form.
    MissingPathPrefix,

    /// The `@@` hunk header doesn't have the `@@ -a[,b] +c[,d] @@` form, or its new side has lines
    /// but starts at line 0.
    MalformedHunkHeader,

    /// A hunk header appears before any `+++` line.
    HunkOutsideFile,

    /// A hunk header's line number or count, or the last line it selects, overflows `usize`.
    LineNumberOverflow,
}

impl fmt::Display for InvalidDiffReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::QuotedPath => "quoted paths are not supported",
            Self::MissingPathPrefix => "the `+++` path must start with `b/` or be `/dev/null`",
            Self::MalformedHunkHeader => {
                "the hunk header must have the form `@@ -a[,b] +c[,d] @@`, with `c > 0` when `d > \
                 0`"
            }
            Self::HunkOutsideFile => "a hunk header appears before any `+++` line",
            Self::LineNumberOverflow => "a hunk header's line number is too large",
        };
        formatter.write_str(description)
    }
}
