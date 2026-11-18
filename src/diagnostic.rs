//! Diagnostic codes, the diagnostics rules produce, and their sorted text and GitHub rendering.

use std::cmp::Ordering;
use std::fmt;
use std::path::PathBuf;

use crate::config::OutputFormat;
use crate::line_ranges::LineSpan;
use crate::rules::RuleId;

/// The code printed with a diagnostic: a rule ID, or a meta code that no rule owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    /// A finding of a lint rule.
    Rule(RuleId),

    /// A suppression comment that can't be parsed.
    InvalidSuppression,

    /// A suppression comment that covered no finding.
    UnusedSuppression,

    /// A file that can't be lexed, or that `syn` can't parse.
    ParseError,
}

impl DiagnosticCode {
    pub const META: [Self; 3] = [
        Self::InvalidSuppression,
        Self::UnusedSuppression,
        Self::ParseError,
    ];

    /// # Returns
    ///
    /// The code as diagnostics print it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rule(rule_id) => rule_id.as_str(),
            Self::InvalidSuppression => "invalid-suppression",
            Self::UnusedSuppression => "unused-suppression",
            Self::ParseError => "parse-error",
        }
    }

    /// # Returns
    ///
    /// The rule that owns this code, or [`None`] for a meta code.
    pub const fn rule_id(self) -> Option<RuleId> {
        match self {
            Self::Rule(rule_id) => Some(rule_id),
            Self::InvalidSuppression | Self::UnusedSuppression | Self::ParseError => None,
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<RuleId> for DiagnosticCode {
    fn from(rule_id: RuleId) -> Self {
        Self::Rule(rule_id)
    }
}

/// A 1-based line and column, with columns counted in Unicode scalar values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// One finding in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    code: DiagnosticCode,
    position: Position,
    message: String,
    anchor: LineSpan,
    fn_header: Option<LineSpan>,
}

impl Diagnostic {
    /// Factory function.
    ///
    /// The anchor span starts as the position's own line, and the diagnostic has no fn header.
    ///
    /// # Returns
    ///
    /// A diagnostic with the given code, position and message.
    ///
    /// # Panics
    ///
    /// Panics if `position.line` is 0.
    pub fn new(code: DiagnosticCode, position: Position, message: String) -> Self {
        assert!(position.line > 0, "diagnostic lines start at 1");
        Self {
            code,
            position,
            message,
            anchor: LineSpan::single(position.line),
            fn_header: None,
        }
    }

    /// Sets the anchor span: the lines the range filter checks when it decides whether to keep the
    /// diagnostic.
    ///
    /// # Returns
    ///
    /// The diagnostic with the new anchor span.
    pub const fn with_anchor(mut self, anchor: LineSpan) -> Self {
        self.anchor = anchor;
        self
    }

    /// Sets the header of the fn the diagnostic belongs to: the lines from its first attribute or
    /// doc line through its `fn` keyword. A suppression that covers any of these lines also covers
    /// the diagnostic.
    ///
    /// # Returns
    ///
    /// The diagnostic with the new fn header.
    pub const fn with_fn_header(mut self, fn_header: LineSpan) -> Self {
        self.fn_header = Some(fn_header);
        self
    }

    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    pub const fn position(&self) -> Position {
        self.position
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub const fn anchor(&self) -> LineSpan {
        self.anchor
    }

    pub const fn fn_header(&self) -> Option<LineSpan> {
        self.fn_header
    }
}

/// A diagnostic together with the file it was found in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiagnostic {
    pub path: PathBuf,
    pub diagnostic: Diagnostic,
}

impl FileDiagnostic {
    /// Renders the diagnostic as one output line, without the trailing newline.
    ///
    /// # Returns
    ///
    /// * For [`OutputFormat::Text`]: `<path>:<line>:<column>: <code>: <message>`.
    /// * For [`OutputFormat::Github`]: an `::error` workflow command with escaped values.
    pub fn render(&self, format: OutputFormat) -> String {
        let path = self.path.display().to_string();
        let Position { line, column } = self.diagnostic.position();
        let code = self.diagnostic.code().as_str();
        let message = self.diagnostic.message();
        match format {
            OutputFormat::Text => format!("{path}:{line}:{column}: {code}: {message}"),
            OutputFormat::Github => format!(
                "::error file={},line={line},col={column},title={}::{}",
                escape_github_property(&path),
                escape_github_property(code),
                escape_github_data(message)
            ),
        }
    }

    /// Compares two diagnostics by path, line, column, code and message, which is the output order.
    ///
    /// # Returns
    ///
    /// The order of `self` relative to `other`.
    pub fn cmp_output_order(&self, other: &Self) -> Ordering {
        self.path
            .cmp(&other.path)
            .then_with(|| self.diagnostic.position.cmp(&other.diagnostic.position))
            .then_with(|| {
                self.diagnostic
                    .code
                    .as_str()
                    .cmp(other.diagnostic.code.as_str())
            })
            .then_with(|| self.diagnostic.message.cmp(&other.diagnostic.message))
    }

    /// Sorts diagnostics into output order and removes the ones that would print the same line.
    pub fn sort_and_dedup(diagnostics: &mut Vec<Self>) {
        diagnostics.sort_by(Self::cmp_output_order);
        diagnostics.dedup_by(|later, earlier| later.cmp_output_order(earlier).is_eq());
    }
}

/// # Returns
///
/// `count` followed by `noun`, with an `s` appended unless `count` is 1, such as `3 columns`.
pub fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Escapes the message of a GitHub workflow command.
///
/// # Returns
///
/// `text` with `%`, `\r` and `\n` percent-encoded.
fn escape_github_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escapes a property value of a GitHub workflow command.
///
/// # Returns
///
/// `text` with `%`, `\r`, `\n`, `:` and `,` percent-encoded.
fn escape_github_property(text: &str) -> String {
    escape_github_data(text)
        .replace(':', "%3A")
        .replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_diagnostic(
        path: &str,
        line: usize,
        column: usize,
        code: DiagnosticCode,
        message: &str,
    ) -> FileDiagnostic {
        FileDiagnostic {
            path: PathBuf::from(path),
            diagnostic: Diagnostic::new(code, Position { line, column }, message.to_owned()),
        }
    }

    #[test]
    fn text_rendering_has_path_position_code_and_message() {
        let diagnostic = file_diagnostic("src/a.rs", 3, 5, DiagnosticCode::ParseError, "bad input");
        assert_eq!(
            diagnostic.render(OutputFormat::Text),
            "src/a.rs:3:5: parse-error: bad input"
        );
    }

    #[test]
    fn github_rendering_escapes_data_and_properties() {
        let diagnostic = file_diagnostic(
            "dir,x/a:b.rs",
            1,
            2,
            RuleId::ErrorsDoc.into(),
            "100% sure:\r\nyes, really",
        );
        assert_eq!(
            diagnostic.render(OutputFormat::Github),
            "::error file=dir%2Cx/a%3Ab.rs,line=1,col=2,title=errors-doc::100%25 sure:%0D%0Ayes, \
             really"
        );
    }

    #[test]
    fn sort_and_dedup_orders_by_path_position_code_then_message() {
        let mut diagnostics = vec![
            file_diagnostic("b.rs", 1, 1, DiagnosticCode::ParseError, "m"),
            file_diagnostic("a.rs", 2, 1, DiagnosticCode::ParseError, "m"),
            file_diagnostic("a.rs", 1, 9, RuleId::ErrorsDoc.into(), "z"),
            file_diagnostic("a.rs", 1, 9, RuleId::ErrorsDoc.into(), "a"),
            file_diagnostic("a.rs", 1, 9, RuleId::CommentGreedyWrap.into(), "z"),
            file_diagnostic("a.rs", 1, 10, DiagnosticCode::ParseError, "m"),
            file_diagnostic("a.rs", 1, 9, RuleId::ErrorsDoc.into(), "a"),
        ];
        FileDiagnostic::sort_and_dedup(&mut diagnostics);
        let rendered: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.render(OutputFormat::Text))
            .collect();
        assert_eq!(
            rendered,
            [
                "a.rs:1:9: comment-greedy-wrap: z",
                "a.rs:1:9: errors-doc: a",
                "a.rs:1:9: errors-doc: z",
                "a.rs:1:10: parse-error: m",
                "a.rs:2:1: parse-error: m",
                "b.rs:1:1: parse-error: m",
            ]
        );
    }

    #[test]
    fn counted_nouns_are_singular_only_for_one() {
        assert_eq!(counted(0, "file"), "0 files");
        assert_eq!(counted(1, "file"), "1 file");
        assert_eq!(counted(2, "generated file"), "2 generated files");
    }
}
