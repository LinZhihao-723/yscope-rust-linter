//! Suppression comments: parsing them, computing the lines they cover, dropping the diagnostics
//! they cover and reporting the ones that covered nothing.
//!
//! A suppression is a plain `//` comment in one of these forms:
//!
//! ```text
//! // yscope-rust-linter: allow(<rule-id>[, <rule-id>]...): <reason>
//! // yscope-rust-linter: allow-item(<rule-id>[, <rule-id>]...): <reason>
//! // yscope-rust-linter: allow-start(<rule-id>[, <rule-id>]...): <reason>
//! // yscope-rust-linter: allow-end(<rule-id>[, <rule-id>]...)
//! // yscope-rust-linter: allow-file(<rule-id>[, <rule-id>]...): <reason>
//! ```
//!
//! Each `<rule-id>` is a [`RuleSelector`]: a rule name, a code, or a set prefix.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fmt;

use crate::config::RuleSet;
use crate::diagnostic::Diagnostic;
use crate::diagnostic::DiagnosticCode;
use crate::diagnostic::Position;
use crate::lexer::CommentLine;
use crate::lexer::CommentMarker;
use crate::line_ranges::LineSpan;
use crate::rules::InvalidRuleSelector;
use crate::rules::RuleId;
use crate::rules::RuleSelector;
use crate::syntax::item_lines::ItemLines;

/// One valid suppression, with the lines it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    position: Position,
    rule_ids: Vec<RuleId>,
    scope: Scope,
}

impl Suppression {
    /// A suppression covers a diagnostic of a rule it names in these cases:
    ///
    /// * Its scope holds the diagnostic's line.
    /// * It is an `allow`, and the diagnostic belongs to a fn whose header intersects its lines.
    ///
    /// # Returns
    ///
    /// Whether this suppression covers `diagnostic`.
    fn covers(&self, diagnostic: &Diagnostic) -> bool {
        let Some(rule_id) = diagnostic.code().rule_id() else {
            return false;
        };
        if !self.rule_ids.contains(&rule_id) {
            return false;
        }
        let line = diagnostic.position().line;
        match self.scope {
            Scope::Unit { lines } => {
                lines.contains(line)
                    || diagnostic
                        .fn_header()
                        .is_some_and(|fn_header| fn_header.intersects(lines))
            }
            Scope::Item { lines } => lines.contains(line),
            Scope::Region {
                start_line,
                end_line,
            } => start_line < line && line < end_line,
            Scope::File => true,
        }
    }

    /// # Returns
    ///
    /// The `unused-suppression` diagnostic that reports this suppression.
    fn unused_diagnostic(&self) -> Diagnostic {
        let rule_ids = join_rule_ids(&self.rule_ids);
        let message = match self.scope {
            Scope::Unit { lines } => {
                format!(
                    "this suppression of {rule_ids} covers no finding on {}; remove it",
                    lines.phrase()
                )
            }
            Scope::Item { lines } => format!(
                "this `allow-item` suppression of {rule_ids} covers no finding in its item on {}; \
                 remove it",
                lines.phrase()
            ),
            Scope::Region {
                start_line,
                end_line,
            } => format!(
                "this region suppression of {rule_ids} covers no finding between lines \
                 {start_line} and {end_line}; remove it"
            ),
            Scope::File => format!(
                "this `allow-file` suppression of {rule_ids} covers no finding in the file; \
                 remove it"
            ),
        };
        Diagnostic::new(DiagnosticCode::UnusedSuppression, self.position, message)
    }
}

/// The suppressions of one file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Suppressions {
    suppressions: Vec<Suppression>,

    /// The `allow-item` suppressions whose item isn't located yet.
    unlocated_items: Vec<UnlocatedItem>,
}

impl Suppressions {
    /// # Returns
    ///
    /// Whether some `allow-item` suppression still needs [`Self::locate_items`] or
    /// [`Self::reject_items`].
    pub const fn has_unlocated_items(&self) -> bool {
        !self.unlocated_items.is_empty()
    }

    /// Locates the item of each `allow-item` suppression:
    ///
    /// * The suppression's target line is the first line after it that is not an own-line
    ///   suppression comment.
    /// * Its item is the outermost of `item_lines` whose header holds the target line.
    /// * The suppression then covers that item's whole span.
    ///
    /// # Returns
    ///
    /// An `invalid-suppression` diagnostic for each `allow-item` suppression that no item directly
    /// follows.
    pub fn locate_items(&mut self, item_lines: &[ItemLines]) -> Vec<Diagnostic> {
        let mut invalid_suppressions = Vec::new();
        for unlocated_item in std::mem::take(&mut self.unlocated_items) {
            let item = item_lines
                .iter()
                .filter(|item| item.header.contains(unlocated_item.target_line))
                .min_by_key(|item| (item.span.first(), Reverse(item.span.last())));
            match item {
                Some(item) => self.suppressions.push(Suppression {
                    position: unlocated_item.position,
                    rule_ids: unlocated_item.rule_ids,
                    scope: Scope::Item { lines: item.span },
                }),
                None => invalid_suppressions
                    .push(InvalidSuppression::NoItem.into_diagnostic(unlocated_item.position)),
            }
        }
        invalid_suppressions
    }

    /// Drops every `allow-item` suppression, since the file's items can't be located for `reason`.
    ///
    /// # Returns
    ///
    /// An `invalid-suppression` diagnostic for each dropped suppression.
    pub fn reject_items(&mut self, reason: &str) -> Vec<Diagnostic> {
        std::mem::take(&mut self.unlocated_items)
            .into_iter()
            .map(|unlocated_item| {
                InvalidSuppression::ItemNotLocated {
                    reason: reason.to_owned(),
                }
                .into_diagnostic(unlocated_item.position)
            })
            .collect()
    }

    /// Drops every diagnostic that a suppression covers, as [`Suppression::covers`] decides, then
    /// reports the suppressions that covered nothing.
    ///
    /// A suppression is reported as unused only when every rule it names is in `rule_set`.
    ///
    /// # Returns
    ///
    /// The diagnostics no suppression covers, followed by an `unused-suppression` diagnostic for
    /// each unused suppression.
    ///
    /// # Panics
    ///
    /// Panics if some `allow-item` suppression is neither located nor rejected.
    pub fn apply(self, diagnostics: Vec<Diagnostic>, rule_set: &RuleSet) -> Vec<Diagnostic> {
        assert!(
            self.unlocated_items.is_empty(),
            "`allow-item` suppressions are located or rejected before they apply"
        );
        let mut used_suppressions: HashSet<usize> = HashSet::new();
        let mut kept: Vec<Diagnostic> = diagnostics
            .into_iter()
            .filter(|diagnostic| {
                let mut is_covered = false;
                for (suppression_index, suppression) in self.suppressions.iter().enumerate() {
                    if suppression.covers(diagnostic) {
                        used_suppressions.insert(suppression_index);
                        is_covered = true;
                    }
                }
                !is_covered
            })
            .collect();
        for (suppression_index, suppression) in self.suppressions.iter().enumerate() {
            if used_suppressions.contains(&suppression_index)
                || !suppression
                    .rule_ids
                    .iter()
                    .all(|rule_id| rule_set.contains(*rule_id))
            {
                continue;
            }
            kept.push(suppression.unused_diagnostic());
        }
        kept
    }
}

/// The comments of one file, split into suppressions and the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedComments {
    pub suppressions: Suppressions,

    /// The comment lines that are not suppression candidates, in source order.
    pub comment_lines: Vec<CommentLine>,

    /// An `invalid-suppression` diagnostic for each malformed or misplaced suppression candidate.
    pub invalid_suppressions: Vec<Diagnostic>,

    /// An `unused-suppression` diagnostic for each deprecated rule that a valid suppression names,
    /// since a deprecated rule never reports a finding.
    pub deprecated_rules: Vec<Diagnostic>,
}

/// Splits a file's comment lines into suppressions and the rest.
///
/// A suppression candidate is a plain `//` comment whose trimmed text starts with
/// `yscope-rust-linter:`. The lines a valid suppression covers depend on its form:
///
/// * `allow`, trailing (with code before it): its own line.
/// * `allow`, own-line: the next unit, which starts at the first following line that is not an
///   own-line suppression candidate:
///   * If that line is an own-line comment, the unit is the whole comment block starting there.
///   * Otherwise, the unit is that single line.
/// * `allow-item`: the item it directly precedes, once [`Suppressions::locate_items`] finds it. A
///   trailing one is invalid.
/// * `allow-start` and `allow-end`: for each rule, the lines strictly between an `allow-start` and
///   the next `allow-end` that names the rule. A start without an end, an end without an open
///   start, and a start for a rule whose region is still open are invalid.
/// * `allow-file`: the whole file. One after `header_end`, the end of the file header, is invalid.
///
/// A deprecated rule that a valid suppression names is reported and covers nothing, so a
/// suppression that names only deprecated rules covers no lines.
///
/// # Returns
///
/// The suppressions, the remaining comment lines and the diagnostics of invalid candidates.
///
/// # Panics
///
/// Panics if a covered unit ends before it starts, which [`CommentLine::continues_block`] rules
/// out.
pub fn parse(comment_lines: Vec<CommentLine>, header_end: Option<Position>) -> ParsedComments {
    let mut candidates = Vec::new();
    let mut remaining_comment_lines = Vec::new();
    for comment_line in comment_lines {
        if is_candidate(&comment_line) {
            candidates.push(comment_line);
        } else {
            remaining_comment_lines.push(comment_line);
        }
    }

    let own_line_candidate_lines: Vec<usize> = candidates
        .iter()
        .filter(|candidate| candidate.is_own_line)
        .map(|candidate| candidate.line)
        .collect();
    let own_line_comment_indices: BTreeMap<usize, usize> = remaining_comment_lines
        .iter()
        .enumerate()
        .filter(|(_, comment_line)| comment_line.is_own_line)
        .map(|(index, comment_line)| (comment_line.line, index))
        .collect();
    let next_unit_start = |line: usize| {
        let mut first = line + 1;
        while own_line_candidate_lines.binary_search(&first).is_ok() {
            first += 1;
        }
        first
    };

    let mut parser = Parser::default();
    for candidate in candidates {
        let position = Position {
            line: candidate.line,
            column: candidate.marker_column,
        };
        let Directive {
            kind,
            rule_ids,
            deprecated_rule_ids,
        } = match Directive::parse(&candidate.text) {
            Ok(directive) => directive,
            Err(invalid) => {
                parser.reject(invalid, position);
                continue;
            }
        };
        parser.deprecated_rules.extend(
            deprecated_rule_ids
                .into_iter()
                .map(|rule_id| deprecated_rule_diagnostic(rule_id, position)),
        );
        if rule_ids.is_empty() {
            continue;
        }
        match kind {
            SuppressionKind::Allow => {
                let lines = if candidate.is_own_line {
                    let first = next_unit_start(candidate.line);
                    let last = own_line_comment_indices.get(&first).map_or(first, |index| {
                        block_last_line(&remaining_comment_lines, *index)
                    });
                    LineSpan::new(first, last)
                        .expect("a covered unit starts after line 0 and ends at or after its start")
                } else {
                    LineSpan::single(candidate.line)
                };
                parser.suppressions.push(Suppression {
                    position,
                    rule_ids,
                    scope: Scope::Unit { lines },
                });
            }
            SuppressionKind::AllowItem if candidate.is_own_line => {
                parser.unlocated_items.push(UnlocatedItem {
                    position,
                    rule_ids,
                    target_line: next_unit_start(candidate.line),
                });
            }
            SuppressionKind::AllowItem => parser.reject(InvalidSuppression::TrailingItem, position),
            SuppressionKind::AllowStart => parser.open_regions(position, rule_ids),
            SuppressionKind::AllowEnd => parser.close_regions(position, rule_ids),
            SuppressionKind::AllowFile
                if header_end.is_some_and(|header_end| position > header_end) =>
            {
                parser.reject(InvalidSuppression::MisplacedFile, position);
            }
            SuppressionKind::AllowFile => parser.suppressions.push(Suppression {
                position,
                rule_ids,
                scope: Scope::File,
            }),
        }
    }
    parser.finish(remaining_comment_lines)
}

/// The prefix that makes a plain comment a suppression candidate.
const SUPPRESSION_PREFIX: &str = "yscope-rust-linter:";

/// The lines a suppression covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// An `allow`: its lines, and the findings of the fns whose header intersects them.
    Unit { lines: LineSpan },

    /// An `allow-item`: the lines of its item.
    Item { lines: LineSpan },

    /// An `allow-start` and its `allow-end`: the lines strictly between them.
    Region { start_line: usize, end_line: usize },

    /// An `allow-file`: the whole file.
    File,
}

/// An `allow-item` suppression whose item isn't located yet.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UnlocatedItem {
    position: Position,
    rule_ids: Vec<RuleId>,

    /// The first line after the suppression that is not an own-line suppression candidate.
    target_line: usize,
}

/// The form of a suppression, named by the keyword after the prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuppressionKind {
    Allow,
    AllowItem,
    AllowStart,
    AllowEnd,
    AllowFile,
}

impl SuppressionKind {
    const ALL: [Self; 5] = [
        Self::Allow,
        Self::AllowItem,
        Self::AllowStart,
        Self::AllowEnd,
        Self::AllowFile,
    ];

    /// # Returns
    ///
    /// The form with the keyword `keyword`, or [`None`] if no form has it.
    fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.keyword() == keyword)
    }

    /// # Returns
    ///
    /// The keyword that names the form after the prefix.
    const fn keyword(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::AllowItem => "allow-item",
            Self::AllowStart => "allow-start",
            Self::AllowEnd => "allow-end",
            Self::AllowFile => "allow-file",
        }
    }

    /// # Returns
    ///
    /// Whether the form needs a reason, which every form but `allow-end` does.
    const fn needs_reason(self) -> bool {
        !matches!(self, Self::AllowEnd)
    }

    /// # Returns
    ///
    /// The syntax of the form, as a code span for messages.
    fn syntax(self) -> String {
        let reason = if self.needs_reason() {
            ": <reason>"
        } else {
            ""
        };
        format!(
            "`// {SUPPRESSION_PREFIX} {}(<rule-id>[, <rule-id>]...){reason}`",
            self.keyword()
        )
    }
}

/// The parsed text of a suppression candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Directive {
    kind: SuppressionKind,

    /// The rules the suppression names that aren't deprecated, without duplicates.
    rule_ids: Vec<RuleId>,

    /// The deprecated rules the suppression names by name or code, without duplicates.
    deprecated_rule_ids: Vec<RuleId>,
}

impl Directive {
    /// Parses the text of a suppression candidate: the prefix, the keyword of a form, the `(...)`
    /// list of [`RuleSelector`]s, and `: <reason>`, which only `allow-end` may leave out.
    ///
    /// # Returns
    ///
    /// The directive on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`InvalidSuppression::Syntax`] if the text doesn't have the form's syntax.
    /// * [`InvalidSuppression::UnknownKind`] if the keyword names no form.
    /// * [`InvalidSuppression::NoRuleIds`] if the `(...)` list is empty.
    /// * [`InvalidSuppression::Selector`] if the list names a meta code or an unknown rule.
    /// * [`InvalidSuppression::MissingReason`] if the form needs a reason and the reason is empty.
    fn parse(text: &str) -> Result<Self, InvalidSuppression> {
        let directive = text
            .trim_start()
            .strip_prefix(SUPPRESSION_PREFIX)
            .ok_or(InvalidSuppression::Syntax {
                kind: SuppressionKind::Allow,
            })?
            .trim_start();
        let keyword_end = directive
            .find(|character: char| !(character.is_alphanumeric() || "-_".contains(character)))
            .unwrap_or(directive.len());
        let (keyword, arguments) = directive.split_at(keyword_end);
        if keyword.is_empty() {
            return Err(InvalidSuppression::Syntax {
                kind: SuppressionKind::Allow,
            });
        }
        let kind = SuppressionKind::from_keyword(keyword).ok_or_else(|| {
            InvalidSuppression::UnknownKind {
                keyword: keyword.to_owned(),
            }
        })?;
        let syntax_error = InvalidSuppression::Syntax { kind };
        let (id_list, rest) = arguments
            .strip_prefix('(')
            .and_then(|arguments| arguments.split_once(')'))
            .ok_or_else(|| syntax_error.clone())?;
        let reason = match rest.trim_start().strip_prefix(':') {
            Some(reason) => Some(reason),
            None if !kind.needs_reason() && rest.trim().is_empty() => None,
            None => return Err(syntax_error),
        };

        let mut rule_ids = Vec::new();
        let mut deprecated_rule_ids = Vec::new();
        for id in id_list.split(',').map(str::trim) {
            if id.is_empty() {
                return Err(if id_list.trim().is_empty() {
                    InvalidSuppression::NoRuleIds { kind }
                } else {
                    syntax_error
                });
            }
            let selector = id
                .parse::<RuleSelector>()
                .map_err(|error| InvalidSuppression::Selector { error })?;
            for rule_id in selector.rule_ids() {
                let target = if rule_id.is_deprecated() {
                    &mut deprecated_rule_ids
                } else {
                    &mut rule_ids
                };
                if !target.contains(&rule_id) {
                    target.push(rule_id);
                }
            }
        }
        if kind.needs_reason() && reason.is_none_or(|reason| reason.trim().is_empty()) {
            return Err(InvalidSuppression::MissingReason { kind });
        }
        Ok(Self {
            kind,
            rule_ids,
            deprecated_rule_ids,
        })
    }
}

/// Why a suppression candidate is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InvalidSuppression {
    /// The text after the keyword doesn't have the `(...)` list and reason of form `kind`, or there
    /// is no keyword.
    Syntax { kind: SuppressionKind },

    /// The keyword after the prefix names no form.
    UnknownKind { keyword: String },

    /// The `(...)` list is empty.
    NoRuleIds { kind: SuppressionKind },

    /// An ID in the `(...)` list names a meta code, which can't be suppressed, or no rule.
    Selector { error: InvalidRuleSelector },

    /// The reason after `):` is empty, and form `kind` needs one.
    MissingReason { kind: SuppressionKind },

    /// An `allow-item` follows code on its line.
    TrailingItem,

    /// No item directly follows an `allow-item`.
    NoItem,

    /// The items of the file can't be located for `reason`, so no `allow-item` can be resolved.
    ItemNotLocated { reason: String },

    /// An `allow-start` names a rule whose region, opened on `open_line`, is still open.
    NestedRegion { rule_id: RuleId, open_line: usize },

    /// An `allow-end` names rules that have no open region.
    UnopenedRegion { rule_ids: Vec<RuleId> },

    /// An `allow-start` opens regions for rules that no `allow-end` closes.
    UnclosedRegion { rule_ids: Vec<RuleId> },

    /// An `allow-file` comes after the file header.
    MisplacedFile,
}

impl InvalidSuppression {
    /// # Returns
    ///
    /// The `invalid-suppression` diagnostic at `position` that reports this problem.
    fn into_diagnostic(self, position: Position) -> Diagnostic {
        Diagnostic::new(
            DiagnosticCode::InvalidSuppression,
            position,
            self.to_string(),
        )
    }
}

impl fmt::Display for InvalidSuppression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const ITEM_PLACEMENT: &str = "put it on its own line directly above an item, or among the \
                                      item's attributes and doc comments";
        match self {
            Self::Syntax { kind } => {
                write!(
                    formatter,
                    "malformed suppression; expected {}",
                    kind.syntax()
                )
            }
            Self::UnknownKind { keyword } => {
                let keywords: Vec<_> = SuppressionKind::ALL
                    .iter()
                    .map(|kind| format!("`{}`", kind.keyword()))
                    .collect();
                write!(
                    formatter,
                    "unknown suppression form `{keyword}`; expected one of {}",
                    keywords.join(", ")
                )
            }
            Self::NoRuleIds { kind } => write!(
                formatter,
                "suppression names no rule; expected {}",
                kind.syntax()
            ),
            Self::Selector {
                error: InvalidRuleSelector::MetaCode { text },
            } => {
                write!(formatter, "`{text}` is not a rule and can't be suppressed")
            }
            Self::Selector { error } => write!(formatter, "{error}"),
            Self::MissingReason { kind } => write!(
                formatter,
                "suppression needs a reason after `):`; expected {}",
                kind.syntax()
            ),
            Self::TrailingItem => write!(
                formatter,
                "an `allow-item` suppression can't follow code on its line; {ITEM_PLACEMENT}"
            ),
            Self::NoItem => write!(
                formatter,
                "no item directly follows this `allow-item` suppression; {ITEM_PLACEMENT}"
            ),
            Self::ItemNotLocated { reason } => write!(
                formatter,
                "can't locate the item of this `allow-item` suppression: {reason}"
            ),
            Self::NestedRegion { rule_id, open_line } => write!(
                formatter,
                "`{rule_id}` already has a region open from line {open_line}; end it with \
                 `allow-end` first"
            ),
            Self::UnopenedRegion { rule_ids } => write!(
                formatter,
                "this `allow-end` of {} closes no open region; open it with `allow-start` first",
                join_rule_ids(rule_ids)
            ),
            Self::UnclosedRegion { rule_ids } => write!(
                formatter,
                "this `allow-start` of {} has no matching `allow-end`; close it with `// \
                 {SUPPRESSION_PREFIX} allow-end({})`",
                join_rule_ids(rule_ids),
                join_rule_ids(rule_ids)
            ),
            Self::MisplacedFile => write!(
                formatter,
                "an `allow-file` suppression must be in the file header, before the first item \
                 and its attributes and doc comments"
            ),
        }
    }
}

/// The state of parsing one file's suppression candidates in source order.
#[derive(Debug, Default)]
struct Parser {
    suppressions: Vec<Suppression>,
    unlocated_items: Vec<UnlocatedItem>,
    invalid_suppressions: Vec<Diagnostic>,
    deprecated_rules: Vec<Diagnostic>,

    /// The regions that are open, in the order they opened.
    open_regions: Vec<OpenRegion>,
}

impl Parser {
    /// Records an invalid suppression candidate at `position`.
    fn reject(&mut self, invalid: InvalidSuppression, position: Position) {
        self.invalid_suppressions
            .push(invalid.into_diagnostic(position));
    }

    /// Opens a region at the `allow-start` at `start` for each of `rule_ids`. A rule whose region
    /// is already open keeps that region, and gets an invalid suppression at `start`.
    fn open_regions(&mut self, start: Position, rule_ids: Vec<RuleId>) {
        for rule_id in rule_ids {
            match self
                .open_regions
                .iter()
                .find(|open_region| open_region.rule_id == rule_id)
            {
                Some(open_region) => {
                    let open_line = open_region.start.line;
                    self.reject(
                        InvalidSuppression::NestedRegion { rule_id, open_line },
                        start,
                    );
                }
                None => self.open_regions.push(OpenRegion { rule_id, start }),
            }
        }
    }

    /// Closes the open region of each of `rule_ids` at the `allow-end` at `end`:
    ///
    /// * The regions that opened at the same `allow-start` become one suppression, at that start.
    /// * The rules without an open region get one invalid suppression at `end`.
    fn close_regions(&mut self, end: Position, rule_ids: Vec<RuleId>) {
        let mut closed: Vec<Suppression> = Vec::new();
        let mut unopened = Vec::new();
        for rule_id in rule_ids {
            let Some(index) = self
                .open_regions
                .iter()
                .position(|open_region| open_region.rule_id == rule_id)
            else {
                unopened.push(rule_id);
                continue;
            };
            let start = self.open_regions.remove(index).start;
            match closed
                .iter_mut()
                .find(|suppression| suppression.position == start)
            {
                Some(suppression) => suppression.rule_ids.push(rule_id),
                None => closed.push(Suppression {
                    position: start,
                    rule_ids: vec![rule_id],
                    scope: Scope::Region {
                        start_line: start.line,
                        end_line: end.line,
                    },
                }),
            }
        }
        self.suppressions.extend(closed);
        if !unopened.is_empty() {
            self.reject(
                InvalidSuppression::UnopenedRegion { rule_ids: unopened },
                end,
            );
        }
    }

    /// Reports the regions left open, one invalid suppression per `allow-start`.
    ///
    /// # Returns
    ///
    /// The parsed comments, with `comment_lines` as the comment lines that are not suppression
    /// candidates.
    fn finish(mut self, comment_lines: Vec<CommentLine>) -> ParsedComments {
        let mut unclosed: BTreeMap<Position, Vec<RuleId>> = BTreeMap::new();
        for open_region in std::mem::take(&mut self.open_regions) {
            unclosed
                .entry(open_region.start)
                .or_default()
                .push(open_region.rule_id);
        }
        for (start, rule_ids) in unclosed {
            self.reject(InvalidSuppression::UnclosedRegion { rule_ids }, start);
        }
        ParsedComments {
            suppressions: Suppressions {
                suppressions: self.suppressions,
                unlocated_items: self.unlocated_items,
            },
            comment_lines,
            invalid_suppressions: self.invalid_suppressions,
            deprecated_rules: self.deprecated_rules,
        }
    }
}

/// A region that an `allow-start` opened for one rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenRegion {
    rule_id: RuleId,
    start: Position,
}

/// # Returns
///
/// Whether `comment_line` is a suppression candidate: a plain comment whose trimmed text starts
/// with `yscope-rust-linter:`.
fn is_candidate(comment_line: &CommentLine) -> bool {
    comment_line.marker == CommentMarker::Plain
        && comment_line
            .text
            .trim_start()
            .starts_with(SUPPRESSION_PREFIX)
}

/// # Returns
///
/// The last line of the comment block that starts at `comment_lines[start_index]`.
fn block_last_line(comment_lines: &[CommentLine], start_index: usize) -> usize {
    let mut last_index = start_index;
    while let Some(next) = comment_lines.get(last_index + 1)
        && next.continues_block(&comment_lines[last_index])
    {
        last_index += 1;
    }
    comment_lines[last_index].line
}

/// # Returns
///
/// The `unused-suppression` diagnostic at `position` that reports the deprecated rule `rule_id`.
fn deprecated_rule_diagnostic(rule_id: RuleId, position: Position) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::UnusedSuppression,
        position,
        format!(
            "`{rule_id}` ({}) is deprecated and never reports a finding; remove it from this \
             suppression",
            rule_id.code()
        ),
    )
}

/// # Returns
///
/// `rule_ids` joined with `, `.
fn join_rule_ids(rule_ids: &[RuleId]) -> String {
    rule_ids
        .iter()
        .map(|rule_id| rule_id.name())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer;
    use crate::syntax::item_lines;

    const ALLOW_ERRORS_DOC: &str = "// yscope-rust-linter: allow(errors-doc): reason\n";
    const ITEM_ERRORS_DOC: &str = "// yscope-rust-linter: allow-item(errors-doc): reason\n";
    const START_ERRORS_DOC: &str = "// yscope-rust-linter: allow-start(errors-doc): reason\n";
    const END_ERRORS_DOC: &str = "// yscope-rust-linter: allow-end(errors-doc)\n";
    const FILE_ERRORS_DOC: &str = "// yscope-rust-linter: allow-file(errors-doc): reason\n";

    /// Parses the suppressions of `source_text`, and locates the items of its `allow-item`
    /// suppressions in its syntax tree.
    ///
    /// # Panics
    ///
    /// Panics if `source_text` doesn't lex, or has an `allow-item` suppression and doesn't parse.
    fn parse_source(source_text: &str) -> ParsedComments {
        let lexed = lexer::lex(source_text);
        assert_eq!(lexed.error, None);
        let mut parsed = parse(lexed.comment_lines, lexed.header_end);
        if parsed.suppressions.has_unlocated_items() {
            let syntax_tree = syn::parse_file(source_text).expect("valid file");
            let invalid = parsed
                .suppressions
                .locate_items(&item_lines::collect(&syntax_tree));
            parsed.invalid_suppressions.extend(invalid);
        }
        parsed
    }

    fn rule_ids(text: &str) -> Result<Vec<RuleId>, InvalidSuppression> {
        Directive::parse(text).map(|directive| directive.rule_ids)
    }

    fn covered_spans(parsed: &ParsedComments) -> Vec<String> {
        parsed
            .suppressions
            .suppressions
            .iter()
            .map(|suppression| match suppression.scope {
                Scope::Unit { lines } | Scope::Item { lines } => lines.to_string(),
                Scope::Region {
                    start_line,
                    end_line,
                } => format!("({start_line}, {end_line})"),
                Scope::File => "file".to_owned(),
            })
            .collect()
    }

    /// # Returns
    ///
    /// The `(line, message)` of each invalid suppression of `source_text`.
    fn invalid(source_text: &str) -> Vec<(usize, String)> {
        let mut invalid: Vec<_> = parse_source(source_text)
            .invalid_suppressions
            .iter()
            .map(|diagnostic| (diagnostic.position().line, diagnostic.message().to_owned()))
            .collect();
        invalid.sort();
        invalid
    }

    /// # Returns
    ///
    /// The lines among the first `num_lines` whose `rule_id` finding the suppressions of
    /// `source_text` cover.
    fn covered_lines(source_text: &str, rule_id: RuleId, num_lines: usize) -> Vec<usize> {
        let suppressions = parse_source(source_text).suppressions;
        (1..=num_lines)
            .filter(|line| {
                suppressions
                    .suppressions
                    .iter()
                    .any(|suppression| suppression.covers(&finding(rule_id, *line)))
            })
            .collect()
    }

    /// # Returns
    ///
    /// The messages of the unused suppressions of `source_text` when nothing is found.
    fn unused_messages(source_text: &str) -> Vec<String> {
        parse_source(source_text)
            .suppressions
            .apply(Vec::new(), &RuleSet::all())
            .iter()
            .map(|diagnostic| diagnostic.message().to_owned())
            .collect()
    }

    fn finding(rule_id: RuleId, line: usize) -> Diagnostic {
        Diagnostic::new(
            rule_id.into(),
            Position { line, column: 1 },
            "finding".to_owned(),
        )
    }

    #[test]
    fn valid_suppressions_parse_their_rule_ids() {
        assert_eq!(
            rule_ids(
                " yscope-rust-linter: allow(errors-doc, comment-greedy-wrap, errors-doc): why"
            ),
            Ok(vec![RuleId::ErrorsDoc, RuleId::CommentGreedyWrap])
        );
        assert_eq!(
            rule_ids("yscope-rust-linter:allow( errors-doc ):reason"),
            Ok(vec![RuleId::ErrorsDoc])
        );
    }

    #[test]
    fn suppressions_name_rules_by_name_code_or_set() {
        assert_eq!(
            rule_ids(" yscope-rust-linter: allow(DOC002, ORG): why"),
            Ok(vec![RuleId::ErrorsDoc, RuleId::SymbolOrder])
        );
        assert_eq!(
            rule_ids(" yscope-rust-linter: allow-end(type-params-doc, DOC, DOC003)"),
            Ok(vec![
                RuleId::TypeParamsDoc,
                RuleId::CommentGreedyWrap,
                RuleId::ErrorsDoc,
                RuleId::TraitParamsDoc,
            ])
        );
    }

    #[test]
    fn deprecated_rule_named_in_a_suppression_is_reported_and_suppresses_nothing() {
        RuleId::ErrorsDoc.with_deprecated(|| {
            let source_text = concat!(
                "// yscope-rust-linter: allow-file(DOC): reason\n",
                "// yscope-rust-linter: allow(errors-doc, comment-greedy-wrap): reason\n",
                "// A block.\n",
                "// yscope-rust-linter: allow(DOC002): reason\n",
                "fn f() {}\n",
                "// yscope-rust-linter: allow-start(DOC002): reason\n",
                "// yscope-rust-linter: allow-end(errors-doc)\n",
            );
            let parsed = parse_source(source_text);
            assert_eq!(parsed.invalid_suppressions, []);
            assert_eq!(covered_spans(&parsed), ["file", "3"]);
            let deprecated_rules: Vec<_> = parsed
                .deprecated_rules
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.position().line))
                .collect();
            assert_eq!(
                deprecated_rules,
                [2, 4, 6, 7].map(|line| (DiagnosticCode::UnusedSuppression, line))
            );
            assert_eq!(
                parsed.deprecated_rules[0].message(),
                "`errors-doc` (DOC002) is deprecated and never reports a finding; remove it from \
                 this suppression"
            );
            let diagnostics = vec![
                finding(RuleId::CommentGreedyWrap, 3),
                finding(RuleId::ErrorsDoc, 5),
            ];
            assert_eq!(
                parsed
                    .suppressions
                    .apply(diagnostics.clone(), &RuleSet::all()),
                diagnostics[1..]
            );
        });
    }

    #[test]
    fn malformed_suppressions_are_rejected() {
        let cases = [
            (
                " yscope-rust-linter: allow(errors-doc)",
                InvalidSuppression::Syntax {
                    kind: SuppressionKind::Allow,
                },
            ),
            (
                " yscope-rust-linter: allow(errors-doc):   ",
                InvalidSuppression::MissingReason {
                    kind: SuppressionKind::Allow,
                },
            ),
            (
                " yscope-rust-linter: allow(): why",
                InvalidSuppression::NoRuleIds {
                    kind: SuppressionKind::Allow,
                },
            ),
            (
                " yscope-rust-linter: allow(errors-doc,): why",
                InvalidSuppression::Syntax {
                    kind: SuppressionKind::Allow,
                },
            ),
            (
                " yscope-rust-linter: deny(errors-doc): why",
                InvalidSuppression::UnknownKind {
                    keyword: "deny".to_owned(),
                },
            ),
            (
                " yscope-rust-linter: allow(parse-error): why",
                InvalidSuppression::Selector {
                    error: InvalidRuleSelector::MetaCode {
                        text: "parse-error".to_owned(),
                    },
                },
            ),
            (
                " yscope-rust-linter: allow(errors-doc, LNT): why",
                InvalidSuppression::Selector {
                    error: InvalidRuleSelector::MetaCode {
                        text: "LNT".to_owned(),
                    },
                },
            ),
            (
                " yscope-rust-linter: allow(errors_doc): why",
                InvalidSuppression::Selector {
                    error: InvalidRuleSelector::Unknown {
                        text: "errors_doc".to_owned(),
                    },
                },
            ),
            (
                " yscope-rust-linter: allow(doc002): why",
                InvalidSuppression::Selector {
                    error: InvalidRuleSelector::Unknown {
                        text: "doc002".to_owned(),
                    },
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(rule_ids(text), Err(expected), "{text:?}");
        }
    }

    #[test]
    fn only_plain_comments_are_candidates_and_they_leave_the_comment_stream() {
        let parsed = parse_source(concat!(
            "/// yscope-rust-linter: allow(errors-doc): doc text\n",
            "//! yscope-rust-linter: allow(errors-doc): doc text\n",
            "// yscope-rust-linter: allow(errors-doc): reason\n",
            "// yscope-rust-linter: nonsense\n",
            "fn f() {}\n",
        ));
        let remaining_lines: Vec<_> = parsed
            .comment_lines
            .iter()
            .map(|comment_line| comment_line.line)
            .collect();
        assert_eq!(remaining_lines, [1, 2]);
        assert_eq!(covered_spans(&parsed), ["5"]);
        let invalid_lines: Vec<_> = parsed
            .invalid_suppressions
            .iter()
            .map(|diagnostic| diagnostic.position().line)
            .collect();
        assert_eq!(invalid_lines, [4]);
    }

    #[test]
    fn coverage_is_the_own_line_the_next_line_or_the_next_comment_block() {
        let parsed = parse_source(concat!(
            "let a = b()?; // yscope-rust-linter: allow(errors-doc): trailing\n",
            "// yscope-rust-linter: allow(errors-doc): stacked\n",
            "// yscope-rust-linter: allow(comment-greedy-wrap): next block\n",
            "/// Block line one.\n",
            "/// Block line two.\n",
            "// Another block.\n",
            "// yscope-rust-linter: allow(errors-doc): next line\n",
            "fn f() {}\n",
        ));
        assert_eq!(covered_spans(&parsed), ["1", "4-5", "4-5", "8"]);
    }

    #[test]
    fn apply_drops_covered_findings_of_the_named_rules_only() {
        let parsed = parse_source(concat!(
            "// yscope-rust-linter: allow(comment-greedy-wrap): reason\n",
            "// A block.\n",
            "// More of it.\n",
        ));
        let diagnostics = vec![
            finding(RuleId::CommentGreedyWrap, 3),
            finding(RuleId::ErrorsDoc, 3),
            Diagnostic::new(
                DiagnosticCode::ParseError,
                Position { line: 2, column: 1 },
                "bad".to_owned(),
            ),
        ];
        let kept = parsed
            .suppressions
            .apply(diagnostics.clone(), &RuleSet::all());
        assert_eq!(kept, diagnostics[1..]);
    }

    #[test]
    fn apply_covers_fn_findings_through_the_fn_header() {
        let parsed =
            parse_source("// yscope-rust-linter: allow(errors-doc): reason\n/// Doc.\nfn f() {}\n");
        let fn_header = LineSpan::new(2, 3).expect("valid span");
        let diagnostics = vec![finding(RuleId::ErrorsDoc, 9).with_fn_header(fn_header)];
        assert_eq!(parsed.suppressions.apply(diagnostics, &RuleSet::all()), []);
    }

    #[test]
    fn trailing_suppression_on_the_question_mark_line_of_a_chain_covers_its_finding() {
        let source_text = concat!(
            "self.client\n",
            "    .load() // yscope-rust-linter: allow(errors-doc): not this line\n",
            "    .await?; // yscope-rust-linter: allow(errors-doc): the `?` line\n",
        );
        let diagnostics = vec![finding(RuleId::ErrorsDoc, 3)];
        let codes: Vec<_> = parse_source(source_text)
            .suppressions
            .apply(diagnostics, &RuleSet::all())
            .iter()
            .map(|diagnostic| (diagnostic.code(), diagnostic.position().line))
            .collect();
        assert_eq!(codes, [(DiagnosticCode::UnusedSuppression, 2)]);
    }

    #[test]
    fn apply_reports_unused_suppressions_only_when_their_rules_are_enabled() {
        let source_text = "// yscope-rust-linter: allow(errors-doc): reason\nfn f() {}\n";
        let unused = parse_source(source_text)
            .suppressions
            .apply(Vec::new(), &RuleSet::all());
        let codes: Vec<_> = unused.iter().map(Diagnostic::code).collect();
        assert_eq!(codes, [DiagnosticCode::UnusedSuppression]);

        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        assert_eq!(
            parse_source(source_text)
                .suppressions
                .apply(Vec::new(), &rule_set),
            []
        );
    }

    #[test]
    fn every_form_parses_its_rule_ids_and_only_allow_end_may_omit_the_reason() {
        for text in [
            " yscope-rust-linter: allow-item(errors-doc): why",
            " yscope-rust-linter: allow-start(errors-doc): why",
            " yscope-rust-linter: allow-end(errors-doc)",
            " yscope-rust-linter: allow-end(errors-doc): why",
            " yscope-rust-linter: allow-file(errors-doc): why",
        ] {
            assert_eq!(rule_ids(text), Ok(vec![RuleId::ErrorsDoc]), "{text:?}");
        }
        let cases = [
            (
                " yscope-rust-linter: allow-item(errors-doc)",
                InvalidSuppression::Syntax {
                    kind: SuppressionKind::AllowItem,
                },
            ),
            (
                " yscope-rust-linter: allow-start(errors-doc): ",
                InvalidSuppression::MissingReason {
                    kind: SuppressionKind::AllowStart,
                },
            ),
            (
                " yscope-rust-linter: allow-file(errors-doc):",
                InvalidSuppression::MissingReason {
                    kind: SuppressionKind::AllowFile,
                },
            ),
            (
                " yscope-rust-linter: allow-end(errors-doc) why",
                InvalidSuppression::Syntax {
                    kind: SuppressionKind::AllowEnd,
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(rule_ids(text), Err(expected), "{text:?}");
        }
    }

    #[test]
    fn unknown_forms_are_rejected_with_the_valid_forms() {
        for keyword in ["allow-block", "nonsense"] {
            let text = format!(" yscope-rust-linter: {keyword}(errors-doc): why");
            let invalid = rule_ids(&text).expect_err("unknown form");
            assert_eq!(
                invalid,
                InvalidSuppression::UnknownKind {
                    keyword: keyword.to_owned()
                }
            );
            assert_eq!(
                invalid.to_string(),
                format!(
                    "unknown suppression form `{keyword}`; expected one of `allow`, `allow-item`, \
                     `allow-start`, `allow-end`, `allow-file`"
                )
            );
        }
    }

    #[test]
    fn allow_item_covers_its_item_from_the_first_attribute_or_doc_line_through_the_last_line() {
        let source_text = format!(
            "{ITEM_ERRORS_DOC}{ALLOW_ERRORS_DOC}/// Doc.\n#[inline]\nfn f() {{\n    // \
             Inner.\n}}\nfn g() {{}}\n"
        );
        assert_eq!(
            covered_lines(&source_text, RuleId::ErrorsDoc, 8),
            [3, 4, 5, 6, 7]
        );
        assert_eq!(
            covered_lines(&source_text, RuleId::CommentGreedyWrap, 8),
            []
        );
    }

    #[test]
    fn allow_item_among_attributes_and_doc_comments_covers_its_item() {
        let source_text =
            format!("/// Doc.\n{ITEM_ERRORS_DOC}#[inline]\n// Note.\nfn f() {{}}\nfn g() {{}}\n");
        assert_eq!(
            covered_lines(&source_text, RuleId::ErrorsDoc, 6),
            [1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn allow_item_above_a_method_covers_only_that_method() {
        let source_text = format!(
            "impl S {{\n    {ITEM_ERRORS_DOC}    fn f() {{\n    }}\n\n    fn g() {{}}\n}}\n"
        );
        assert_eq!(covered_lines(&source_text, RuleId::ErrorsDoc, 7), [3, 4]);
    }

    #[test]
    fn allow_item_without_an_item_right_after_it_is_invalid() {
        let no_item = "no item directly follows this `allow-item` suppression; put it on its own \
                       line directly above an item, or among the item's attributes and doc \
                       comments";
        for source_text in [
            format!("{ITEM_ERRORS_DOC}\nfn f() {{}}\n"),
            format!("// Note.\n{ITEM_ERRORS_DOC}// Note.\nfn f() {{}}\n"),
            format!("fn f() {{\n    {ITEM_ERRORS_DOC}    g();\n}}\n"),
            format!("fn f() {{}}\n{ITEM_ERRORS_DOC}"),
        ] {
            let line = source_text
                .lines()
                .position(|line| line.contains("allow-item"))
                .expect("an allow-item line")
                + 1;
            assert_eq!(invalid(&source_text), [(line, no_item.to_owned())]);
        }
    }

    #[test]
    fn trailing_allow_item_is_invalid() {
        assert_eq!(
            invalid(&format!("fn f() {{}} {ITEM_ERRORS_DOC}")),
            [(
                1,
                "an `allow-item` suppression can't follow code on its line; put it on its own \
                 line directly above an item, or among the item's attributes and doc comments"
                    .to_owned()
            )]
        );
    }

    #[test]
    fn rejected_items_are_invalid_suppressions_that_name_the_reason() {
        let lexed = lexer::lex(&format!("{ITEM_ERRORS_DOC}fn f() {{}}\n"));
        let mut parsed = parse(lexed.comment_lines, lexed.header_end);
        let invalid: Vec<_> = parsed
            .suppressions
            .reject_items("the file failed to parse")
            .iter()
            .map(|diagnostic| (diagnostic.code(), diagnostic.message().to_owned()))
            .collect();
        assert_eq!(
            invalid,
            [(
                DiagnosticCode::InvalidSuppression,
                "can't locate the item of this `allow-item` suppression: the file failed to parse"
                    .to_owned()
            )]
        );
        assert_eq!(parsed.suppressions.apply(Vec::new(), &RuleSet::all()), []);
    }

    #[test]
    fn region_covers_the_lines_strictly_between_its_start_and_its_end_for_each_rule() {
        let source_text = concat!(
            "// yscope-rust-linter: allow-start(errors-doc, comment-greedy-wrap): reason\n",
            "a\n",
            "// yscope-rust-linter: allow-end(comment-greedy-wrap)\n",
            "b\n",
            "// yscope-rust-linter: allow-end(errors-doc)\n",
            "c\n",
        );
        assert_eq!(
            covered_lines(source_text, RuleId::CommentGreedyWrap, 6),
            [2]
        );
        assert_eq!(covered_lines(source_text, RuleId::ErrorsDoc, 6), [2, 3, 4]);
        assert_eq!(invalid(source_text), []);

        let source_text = concat!(
            "// yscope-rust-linter: allow-start(errors-doc): reason\n",
            "// yscope-rust-linter: allow-start(comment-greedy-wrap): reason\n",
            "a\n",
            "// yscope-rust-linter: allow-end(comment-greedy-wrap)\n",
            "b\n",
            "// yscope-rust-linter: allow-end(errors-doc)\n",
        );
        assert_eq!(
            covered_lines(source_text, RuleId::CommentGreedyWrap, 6),
            [3]
        );
        assert_eq!(
            covered_lines(source_text, RuleId::ErrorsDoc, 6),
            [2, 3, 4, 5]
        );
        assert_eq!(invalid(source_text), []);
    }

    #[test]
    fn regions_of_one_start_closed_by_one_end_are_one_suppression() {
        let source_text = concat!(
            "// yscope-rust-linter: allow-start(errors-doc, comment-greedy-wrap): reason\n",
            "a\n",
            "// yscope-rust-linter: allow-end(comment-greedy-wrap, errors-doc)\n",
        );
        assert_eq!(covered_spans(&parse_source(source_text)), ["(1, 3)"]);
        assert_eq!(
            unused_messages(source_text),
            [
                "this region suppression of comment-greedy-wrap, errors-doc covers no finding \
                 between lines 1 and 3; remove it"
            ]
        );
    }

    #[test]
    fn region_start_without_an_end_is_invalid_at_the_start() {
        let source_text = format!("a\n{START_ERRORS_DOC}b\n");
        assert_eq!(
            invalid(&source_text),
            [(
                2,
                "this `allow-start` of errors-doc has no matching `allow-end`; close it with `// \
                 yscope-rust-linter: allow-end(errors-doc)`"
                    .to_owned()
            )]
        );
        assert_eq!(covered_lines(&source_text, RuleId::ErrorsDoc, 3), []);
    }

    #[test]
    fn region_end_without_an_open_start_for_its_rule_is_invalid() {
        let source_text = concat!(
            "// yscope-rust-linter: allow-start(errors-doc): reason\n",
            "a\n",
            "// yscope-rust-linter: allow-end(errors-doc, comment-greedy-wrap)\n",
            "// yscope-rust-linter: allow-end(errors-doc)\n",
        );
        assert_eq!(
            invalid(source_text),
            [
                (
                    3,
                    "this `allow-end` of comment-greedy-wrap closes no open region; open it with \
                     `allow-start` first"
                        .to_owned()
                ),
                (
                    4,
                    "this `allow-end` of errors-doc closes no open region; open it with \
                     `allow-start` first"
                        .to_owned()
                ),
            ]
        );
        assert_eq!(covered_lines(source_text, RuleId::ErrorsDoc, 4), [2]);
    }

    #[test]
    fn second_start_of_an_open_rule_is_invalid_and_the_first_region_stays_open() {
        let source_text = format!("{START_ERRORS_DOC}a\n{START_ERRORS_DOC}b\n{END_ERRORS_DOC}c\n");
        assert_eq!(
            invalid(&source_text),
            [(
                3,
                "`errors-doc` already has a region open from line 1; end it with `allow-end` first"
                    .to_owned()
            )]
        );
        assert_eq!(covered_lines(&source_text, RuleId::ErrorsDoc, 6), [2, 3, 4]);
    }

    #[test]
    fn allow_file_in_the_header_covers_the_whole_file() {
        let source_text = format!(
            "//! Crate docs.\n#![allow(dead_code)]\n\n/* Block. */\n// \
             Note.\n{FILE_ERRORS_DOC}/// Doc.\nfn f() {{}}\n"
        );
        assert_eq!(invalid(&source_text), []);
        assert_eq!(
            covered_lines(&source_text, RuleId::ErrorsDoc, 100),
            (1..=100).collect::<Vec<_>>()
        );
        assert_eq!(
            covered_lines(&source_text, RuleId::CommentGreedyWrap, 3),
            []
        );
    }

    #[test]
    fn allow_file_after_the_header_is_invalid() {
        let misplaced = "an `allow-file` suppression must be in the file header, before the first \
                         item and its attributes and doc comments";
        for source_text in [
            format!("fn f() {{}}\n{FILE_ERRORS_DOC}"),
            format!("/// Doc.\n{FILE_ERRORS_DOC}fn f() {{}}\n"),
            format!("#[derive(Debug)]\n{FILE_ERRORS_DOC}struct S;\n"),
            format!("use a::b; {FILE_ERRORS_DOC}"),
        ] {
            let line = source_text.lines().count();
            let line = source_text
                .lines()
                .position(|text| text.contains("allow-file"))
                .map_or(line, |index| index + 1);
            assert_eq!(invalid(&source_text), [(line, misplaced.to_owned())]);
        }
    }

    #[test]
    fn unused_item_region_and_file_suppressions_are_reported() {
        assert_eq!(
            unused_messages(&format!("{ITEM_ERRORS_DOC}fn f() {{\n}}\n")),
            [
                "this `allow-item` suppression of errors-doc covers no finding in its item on \
                 lines 2-3; remove it"
            ]
        );
        assert_eq!(
            unused_messages(&format!("{START_ERRORS_DOC}a\n{END_ERRORS_DOC}")),
            [
                "this region suppression of errors-doc covers no finding between lines 1 and 3; \
                 remove it"
            ]
        );
        assert_eq!(
            unused_messages(FILE_ERRORS_DOC),
            [
                "this `allow-file` suppression of errors-doc covers no finding in the file; \
                 remove it"
            ]
        );
        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        assert_eq!(
            parse_source(FILE_ERRORS_DOC)
                .suppressions
                .apply(Vec::new(), &rule_set),
            []
        );
    }
}
