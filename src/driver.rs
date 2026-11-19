//! The lint run: discovering files, running each enabled rule on them, applying suppressions and
//! line filters, and sorting the result.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use crate::config::DiffInput;
use crate::config::LintConfig;
use crate::config::RuleSet;
use crate::diagnostic::Diagnostic;
use crate::diagnostic::FileDiagnostic;
use crate::diff;
use crate::error::Error;
use crate::file_context::FileContext;
use crate::line_ranges::LineRanges;
use crate::rules::Rule;
use crate::rules::RuleId;
use crate::source;
use crate::source::LineFilter;
use crate::syntax::test_code::TestModuleCache;

/// The outcome of a lint run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The diagnostics of every file, in output order and without duplicates.
    pub diagnostics: Vec<FileDiagnostic>,

    /// The number of files checked.
    pub num_files_checked: usize,

    /// The number of files skipped because they are marked `@generated`.
    pub num_generated_files: usize,
}

/// Runs the lint over every file the configuration selects, skipping the files marked `@generated`
/// in their first five lines.
///
/// # Returns
///
/// The sorted diagnostics of every file on success.
///
/// # Errors
///
/// Returns an error if:
///
/// * [`Error::Io`] if a source file can't be read.
/// * Forwards [`read_diff_input`]'s return values on failure.
/// * Forwards [`diff::parse`]'s return values on failure.
/// * Forwards [`source::discover`]'s return values on failure.
pub fn run(config: LintConfig) -> Result<Report, Error> {
    let diff_ranges: BTreeMap<PathBuf, LineRanges> = match &config.diff_input {
        Some(diff_input) => diff::parse(&read_diff_input(diff_input)?)?,
        None => BTreeMap::new(),
    };
    let mut file_checker = FileChecker::new(&config);
    let source_files = source::discover(config.targets, diff_ranges, &config.excludes)?;

    let mut diagnostics = Vec::new();
    let mut num_generated_files = 0;
    for source_file in &source_files {
        let source_text = fs::read_to_string(&source_file.path).map_err(|source| Error::Io {
            path: source_file.path.clone(),
            source,
        })?;
        if source::is_generated(&source_text) {
            num_generated_files += 1;
            continue;
        }
        diagnostics.extend(
            file_checker
                .check(&source_file.path, &source_text, &source_file.line_filter)
                .into_iter()
                .map(|diagnostic| FileDiagnostic {
                    path: source_file.path.clone(),
                    diagnostic,
                }),
        );
    }
    FileDiagnostic::sort_and_dedup(&mut diagnostics);
    Ok(Report {
        diagnostics,
        num_files_checked: source_files.len() - num_generated_files,
        num_generated_files,
    })
}

/// The enabled rules of a run, with the state they keep across files.
struct FileChecker {
    rule_set: RuleSet,
    rules: Vec<Rule>,

    /// The out-of-line test module verdicts, shared by the rules through each file's
    /// [`FileContext`].
    test_module_cache: TestModuleCache,
}

impl FileChecker {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// A checker with the rules and settings of `config`.
    fn new(config: &LintConfig) -> Self {
        Self {
            rule_set: config.rule_set.clone(),
            rules: RuleId::ALL
                .into_iter()
                .filter(|rule_id| config.rule_set.contains(*rule_id))
                .map(|rule_id| Rule::new(rule_id, config))
                .collect(),
            test_module_cache: TestModuleCache::default(),
        }
    }

    /// Runs every enabled rule on one file and applies its suppressions and line filter.
    ///
    /// The steps, in order:
    ///
    /// 1. The file's [`FileContext`] is built: it is lexed, and suppression comments are split from
    ///    the other comments.
    /// 2. Each enabled rule checks the file through the context. A rule that needs the `syn` syntax
    ///    tree gets it parsed once per file, and is skipped if the file failed to lex or to parse.
    /// 3. A lex failure becomes a `parse-error` diagnostic, suppressions drop the diagnostics they
    ///    cover, and unused and malformed suppressions are reported.
    /// 4. Diagnostics whose anchor span misses `line_filter` are dropped.
    ///
    /// # Returns
    ///
    /// The file's diagnostics, unsorted.
    fn check(
        &mut self,
        path: &Path,
        source_text: &str,
        line_filter: &LineFilter,
    ) -> Vec<Diagnostic> {
        let context = FileContext::new(path, source_text, &self.test_module_cache);
        let rule_diagnostics = self
            .rules
            .iter_mut()
            .flat_map(|rule| rule.check(&context))
            .collect();
        let mut diagnostics = context.finish(rule_diagnostics, &self.rule_set);
        if let LineFilter::Ranges(line_ranges) = line_filter {
            diagnostics.retain(|diagnostic| line_ranges.intersects(diagnostic.anchor()));
        }
        diagnostics
    }
}

/// Reads the whole diff text.
///
/// # Returns
///
/// The diff text on success.
///
/// # Errors
///
/// Returns an error if:
///
/// * [`Error::ReadStdin`] if the standard input can't be read.
/// * [`Error::Io`] if the diff file can't be read.
fn read_diff_input(diff_input: &DiffInput) -> Result<String, Error> {
    match diff_input {
        DiffInput::Stdin => {
            let mut diff_text = String::new();
            std::io::stdin()
                .read_to_string(&mut diff_text)
                .map_err(|source| Error::ReadStdin { source })?;
            Ok(diff_text)
        }
        DiffInput::File { path } => fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OutputFormat;
    use crate::config::Width;
    use crate::diagnostic::DiagnosticCode;

    fn file_checker(rule_set: RuleSet) -> FileChecker {
        let config = LintConfig {
            rule_set,
            width: Width::new(std::num::NonZeroUsize::new(100).expect("non-zero")),
            include_tests: false,
            format: OutputFormat::Text,
            targets: Vec::new(),
            diff_input: None,
            excludes: Vec::new(),
        };
        FileChecker::new(&config)
    }

    fn rendered(diagnostics: Vec<Diagnostic>) -> Vec<String> {
        let mut file_diagnostics: Vec<_> = diagnostics
            .into_iter()
            .map(|diagnostic| FileDiagnostic {
                path: PathBuf::from("a.rs"),
                diagnostic,
            })
            .collect();
        FileDiagnostic::sort_and_dedup(&mut file_diagnostics);
        file_diagnostics
            .iter()
            .map(|file_diagnostic| file_diagnostic.render(OutputFormat::Text))
            .collect()
    }

    #[test]
    fn lex_failure_is_one_parse_error() {
        let diagnostics = file_checker(RuleSet::all()).check(
            Path::new("a.rs"),
            "fn f() {\n    \"open\n",
            &LineFilter::All,
        );
        assert_eq!(
            rendered(diagnostics),
            [
                "a.rs:2:5: parse-error: unterminated string literal; only the comments before it \
                 are checked"
            ]
        );
    }

    #[test]
    fn syn_failure_is_a_parse_error_only_when_errors_doc_runs() {
        let source_text = "fn f( {}\n";
        let diagnostics =
            file_checker(RuleSet::all()).check(Path::new("a.rs"), source_text, &LineFilter::All);
        let codes: Vec<_> = diagnostics.iter().map(Diagnostic::code).collect();
        assert_eq!(codes, [DiagnosticCode::ParseError]);

        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        assert_eq!(
            file_checker(rule_set).check(Path::new("a.rs"), source_text, &LineFilter::All),
            []
        );
    }

    #[test]
    fn syn_failure_drops_only_the_rustfmt_hint_of_comment_greedy_wrap() {
        let source_text = format!(
            "struct S {{\n    /// {}\n    /// cc dd\n    a: u8,\n",
            "a".repeat(89)
        );
        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        let diagnostics =
            file_checker(rule_set).check(Path::new("a.rs"), &source_text, &LineFilter::All);
        let messages: Vec<_> = diagnostics.iter().map(Diagnostic::message).collect();
        assert_eq!(
            messages,
            [
                "line is 97 columns wide and the next line's first word `cc` (2 columns) fits \
                 within 100; move it up"
            ]
        );
        let parsable = format!("{source_text}}}\n");
        let diagnostics =
            file_checker(RuleSet::all()).check(Path::new("a.rs"), &parsable, &LineFilter::All);
        let messages: Vec<_> = diagnostics.iter().map(Diagnostic::message).collect();
        assert_eq!(
            messages,
            [
                "line is 97 columns wide and the next line's first word `cc` (2 columns) fits \
                 within 100; move it up, but rustfmt wraps this comment one column early \
                 (rust-lang/rustfmt#6180), so reword the paragraph instead"
            ]
        );
    }

    #[test]
    fn errors_doc_finding_is_kept_when_any_line_of_its_fn_is_in_range() {
        let source_text = "/// Loads.\n///\n/// # Errors\n///\n/// Returns an error if:\n///\n/// \
                           * Forwards [`save`]'s return values on failure.\nfn f() -> Result<(), \
                           E> {\n    load()?;\n    save()?;\n    Ok(())\n}\n\nfn g() {}\n";
        let messages = |ranges: &str| -> Vec<String> {
            let line_filter = LineFilter::Ranges(ranges.parse().expect("valid ranges"));
            file_checker(RuleSet::all())
                .check(Path::new("a.rs"), source_text, &line_filter)
                .iter()
                .map(|diagnostic| diagnostic.message().to_owned())
                .collect()
        };
        let finding = "`f` forwards errors from `load`, but `# Errors` does not list it; add \"* \
                       Forwards [`load`]'s return values on failure.\"";
        assert_eq!(messages("4"), [finding]);
        assert_eq!(messages("12"), [finding]);
        assert_eq!(messages("13-14"), Vec::<String>::new());
    }

    #[test]
    fn line_filter_keeps_diagnostics_whose_anchor_intersects_it() {
        let source_text =
            "// yscope-rust-linter: allow(nothing)\n\n// yscope-rust-linter: allow(nothing)\n";
        let line_filter = LineFilter::Ranges("3".parse().expect("valid ranges"));
        let diagnostics =
            file_checker(RuleSet::all()).check(Path::new("a.rs"), source_text, &line_filter);
        let lines: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.position().line)
            .collect();
        assert_eq!(lines, [3]);
    }

    #[test]
    fn greedy_wrap_finding_is_kept_when_only_its_next_line_is_in_range() {
        let source_text = "// Short\n// line.\n\nfn f() {}\n";
        let lines = |ranges: &str| -> Vec<usize> {
            let line_filter = LineFilter::Ranges(ranges.parse().expect("valid ranges"));
            file_checker(RuleSet::all())
                .check(Path::new("a.rs"), source_text, &line_filter)
                .iter()
                .map(|diagnostic| diagnostic.position().line)
                .collect()
        };
        assert_eq!(lines("2"), [1]);
        assert_eq!(lines("3-4"), Vec::<usize>::new());
    }

    #[test]
    fn unused_suppression_is_reported_only_when_its_line_is_in_range() {
        let source_text =
            "// yscope-rust-linter: allow(errors-doc): reason\nfn f() {}\n\nfn g() {}\n";
        let codes = |ranges: &str| -> Vec<DiagnosticCode> {
            let line_filter = LineFilter::Ranges(ranges.parse().expect("valid ranges"));
            file_checker(RuleSet::all())
                .check(Path::new("a.rs"), source_text, &line_filter)
                .iter()
                .map(Diagnostic::code)
                .collect()
        };
        assert_eq!(codes("1"), [DiagnosticCode::UnusedSuppression]);
        assert_eq!(codes("2-4"), []);
    }

    #[test]
    fn suppression_comment_ends_the_greedy_wrap_paragraph() {
        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        let check = |source_text: &str| -> Vec<usize> {
            file_checker(rule_set.clone())
                .check(Path::new("a.rs"), source_text, &LineFilter::All)
                .iter()
                .filter(|diagnostic| diagnostic.code() == RuleId::CommentGreedyWrap.into())
                .map(|diagnostic| diagnostic.position().line)
                .collect()
        };
        assert_eq!(check("// Short\n// line.\n"), [1]);
        for suppression in [
            "allow(errors-doc): reason",
            "allow-item(errors-doc): reason",
            "allow-start(errors-doc): reason",
            "allow-end(errors-doc)",
            "allow-file(errors-doc): reason",
        ] {
            assert_eq!(
                check(&format!(
                    "// Short\n// yscope-rust-linter: {suppression}\n// line.\nfn f() {{}}\n"
                )),
                Vec::<usize>::new(),
                "{suppression}"
            );
        }
    }

    #[test]
    fn allow_item_suppresses_the_greedy_wrap_findings_inside_its_item_only() {
        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        let source_text = "// yscope-rust-linter: allow-item(comment-greedy-wrap): reason\n/// \
                           Short\n/// line.\nfn f() {}\n// Short\n// line.\n";
        let lines: Vec<_> = file_checker(rule_set)
            .check(Path::new("a.rs"), source_text, &LineFilter::All)
            .iter()
            .map(|diagnostic| (diagnostic.code(), diagnostic.position().line))
            .collect();
        assert_eq!(lines, [(RuleId::CommentGreedyWrap.into(), 5)]);
    }

    #[test]
    fn greedy_wrap_only_run_parses_only_files_with_an_allow_item() {
        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        let codes = |source_text: &str| -> Vec<DiagnosticCode> {
            file_checker(rule_set.clone())
                .check(Path::new("a.rs"), source_text, &LineFilter::All)
                .iter()
                .map(Diagnostic::code)
                .collect()
        };
        assert_eq!(codes("fn f( {}\n"), []);
        assert_eq!(
            codes("// yscope-rust-linter: allow-item(comment-greedy-wrap): reason\nfn f( {}\n"),
            [DiagnosticCode::InvalidSuppression]
        );
    }
}
