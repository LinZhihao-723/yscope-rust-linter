//! Diagnostic kinds and their codes, the diagnostics rules produce, and their sorted text and
//! GitHub rendering.

use std::cmp::Ordering;
use std::fmt;
use std::path::PathBuf;

use crate::config::OutputFormat;
use crate::line_ranges::LineSpan;
use crate::rules::Code;
use crate::rules::CodeSet;
use crate::rules::RuleId;

/// What a diagnostic reports: a rule's finding, or a meta diagnostic that no rule owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    /// A finding of a lint rule.
    Rule(RuleId),

    /// A suppression comment that can't be parsed.
    InvalidSuppression,

    /// A suppression comment that covered no finding.
    UnusedSuppression,

    /// A file that can't be lexed, or that `syn` can't parse.
    ParseError,
}

impl DiagnosticKind {
    pub const META: [Self; 3] = [
        Self::InvalidSuppression,
        Self::UnusedSuppression,
        Self::ParseError,
    ];

    /// # Returns
    ///
    /// The name of the rule or meta diagnostic, such as `errors-doc` or `parse-error`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rule(rule_id) => rule_id.name(),
            Self::InvalidSuppression => "invalid-suppression",
            Self::UnusedSuppression => "unused-suppression",
            Self::ParseError => "parse-error",
        }
    }

    /// # Returns
    ///
    /// The stable code of the rule or meta diagnostic, such as `DOC002` or `LNT001`.
    pub const fn code(self) -> Code {
        match self {
            Self::Rule(rule_id) => rule_id.code(),
            Self::ParseError => Code::new(CodeSet::Lnt, 1),
            Self::InvalidSuppression => Code::new(CodeSet::Lnt, 2),
            Self::UnusedSuppression => Code::new(CodeSet::Lnt, 3),
        }
    }

    /// # Returns
    ///
    /// The rule that owns this kind, or [`None`] for a meta diagnostic.
    pub const fn rule_id(self) -> Option<RuleId> {
        match self {
            Self::Rule(rule_id) => Some(rule_id),
            Self::InvalidSuppression | Self::UnusedSuppression | Self::ParseError => None,
        }
    }
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.code(), self.name())
    }
}

impl From<RuleId> for DiagnosticKind {
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
    kind: DiagnosticKind,
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
    /// A diagnostic with the given kind, position and message.
    ///
    /// # Panics
    ///
    /// Panics if `position.line` is 0.
    pub fn new(kind: DiagnosticKind, position: Position, message: String) -> Self {
        assert!(position.line > 0, "diagnostic lines start at 1");
        Self {
            kind,
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

    pub const fn kind(&self) -> DiagnosticKind {
        self.kind
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
    /// * For [`OutputFormat::Text`]: `<path>:<line>:<column>: <code> <name>: <message>`.
    /// * For [`OutputFormat::Github`]: an `::error` workflow command with escaped values, titled
    ///   `<code> <name>`.
    pub fn render(&self, format: OutputFormat) -> String {
        let path = self.path.display().to_string();
        let Position { line, column } = self.diagnostic.position();
        let code_and_name = self.diagnostic.kind().to_string();
        let message = self.diagnostic.message();
        match format {
            OutputFormat::Text => format!("{path}:{line}:{column}: {code_and_name}: {message}"),
            OutputFormat::Github => format!(
                "::error file={},line={line},col={column},title={}::{}",
                escape_github_property(&path),
                escape_github_property(&code_and_name),
                escape_github_data(message)
            ),
        }
    }

    /// Compares two diagnostics by path, line, column, rule or meta name and message, which is the
    /// output order.
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
                    .kind
                    .name()
                    .cmp(other.diagnostic.kind.name())
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
        kind: DiagnosticKind,
        message: &str,
    ) -> FileDiagnostic {
        FileDiagnostic {
            path: PathBuf::from(path),
            diagnostic: Diagnostic::new(kind, Position { line, column }, message.to_owned()),
        }
    }

    #[test]
    fn text_rendering_has_path_position_code_and_message() {
        let diagnostic = file_diagnostic("src/a.rs", 3, 5, DiagnosticKind::ParseError, "bad input");
        assert_eq!(
            diagnostic.render(OutputFormat::Text),
            "src/a.rs:3:5: LNT001 parse-error: bad input"
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
            "::error file=dir%2Cx/a%3Ab.rs,line=1,col=2,title=DOC002 errors-doc::100%25 \
             sure:%0D%0Ayes, really"
        );
    }

    #[test]
    fn sort_and_dedup_orders_by_path_position_code_then_message() {
        let mut diagnostics = vec![
            file_diagnostic("b.rs", 1, 1, DiagnosticKind::ParseError, "m"),
            file_diagnostic("a.rs", 2, 1, DiagnosticKind::ParseError, "m"),
            file_diagnostic("a.rs", 1, 9, RuleId::ErrorsDoc.into(), "z"),
            file_diagnostic("a.rs", 1, 9, RuleId::ErrorsDoc.into(), "a"),
            file_diagnostic("a.rs", 1, 9, RuleId::CommentGreedyWrap.into(), "z"),
            file_diagnostic("a.rs", 1, 10, DiagnosticKind::ParseError, "m"),
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
                "a.rs:1:9: DOC001 comment-greedy-wrap: z",
                "a.rs:1:9: DOC002 errors-doc: a",
                "a.rs:1:9: DOC002 errors-doc: z",
                "a.rs:1:10: LNT001 parse-error: m",
                "a.rs:2:1: LNT001 parse-error: m",
                "b.rs:1:1: LNT001 parse-error: m",
            ]
        );
    }

    #[test]
    fn sort_and_dedup_orders_kinds_at_one_position_by_name_not_by_code() {
        let mut diagnostics = vec![
            file_diagnostic("a.rs", 1, 1, RuleId::TypeParamsDoc.into(), "m"),
            file_diagnostic("a.rs", 1, 1, RuleId::SymbolOrder.into(), "m"),
        ];
        FileDiagnostic::sort_and_dedup(&mut diagnostics);
        let rendered: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.render(OutputFormat::Text))
            .collect();
        assert_eq!(
            rendered,
            [
                "a.rs:1:1: ORG001 symbol-order: m",
                "a.rs:1:1: DOC003 type-params-doc: m",
            ]
        );
    }

    fn every_kind() -> Vec<DiagnosticKind> {
        RuleId::ALL
            .into_iter()
            .map(DiagnosticKind::from)
            .chain(DiagnosticKind::META)
            .collect()
    }

    #[test]
    fn codes_are_frozen() {
        let mut table: Vec<_> = every_kind()
            .into_iter()
            .map(|kind| (kind.code().to_string(), kind.name()))
            .collect();
        table.sort_unstable();
        assert_eq!(
            table,
            [
                ("DOC001".to_owned(), "comment-greedy-wrap"),
                ("DOC002".to_owned(), "errors-doc"),
                ("DOC003".to_owned(), "type-params-doc"),
                ("DOC004".to_owned(), "trait-params-doc"),
                ("LNT001".to_owned(), "parse-error"),
                ("LNT002".to_owned(), "invalid-suppression"),
                ("LNT003".to_owned(), "unused-suppression"),
                ("ORG001".to_owned(), "symbol-order"),
            ]
        );
    }

    #[test]
    fn codes_are_well_formed_and_numbered_without_gaps_in_each_set() {
        for kind in every_kind() {
            let text = kind.code().to_string();
            let (prefix, digits) = text.split_at(3);
            assert_eq!(prefix, kind.code().set.prefix(), "{text}");
            assert!(
                prefix.len() == 3 && prefix.bytes().all(|byte| byte.is_ascii_uppercase()),
                "{text}"
            );
            assert!(
                digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit()),
                "{text}"
            );
            assert_eq!(
                kind.code().set == CodeSet::Lnt,
                kind.rule_id().is_none(),
                "{text}"
            );
        }
        for set in CodeSet::ALL {
            let mut numbers: Vec<_> = every_kind()
                .into_iter()
                .map(DiagnosticKind::code)
                .filter(|code| code.set == set)
                .map(|code| code.number)
                .collect();
            numbers.sort_unstable();
            let expected: Vec<u16> = (1..).take(numbers.len()).collect();
            assert_eq!(numbers, expected, "{}", set.prefix());
        }
    }

    #[test]
    fn codes_and_names_are_unique() {
        let kinds = every_kind();
        for (index, kind) in kinds.iter().enumerate() {
            for other in &kinds[index + 1..] {
                assert_ne!(kind.code(), other.code());
                assert_ne!(kind.name(), other.name());
            }
        }
    }

    #[test]
    fn counted_nouns_are_singular_only_for_one() {
        assert_eq!(counted(0, "file"), "0 files");
        assert_eq!(counted(1, "file"), "1 file");
        assert_eq!(counted(2, "generated file"), "2 generated files");
    }
}
