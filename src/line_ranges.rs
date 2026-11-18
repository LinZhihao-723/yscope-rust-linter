//! Inclusive 1-based line spans and the sets of them that restrict which lines a run reports.

use std::fmt;
use std::str::FromStr;

/// An inclusive range of 1-based line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineSpan {
    first: usize,
    last: usize,
}

impl LineSpan {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The span `first..=last` on success, or [`None`] if `first` is 0 or `last` is smaller than
    /// `first`.
    pub const fn new(first: usize, last: usize) -> Option<Self> {
        if first == 0 || last < first {
            return None;
        }
        Some(Self { first, last })
    }

    /// Factory function.
    ///
    /// # Returns
    ///
    /// The span holding only `line`.
    ///
    /// # Panics
    ///
    /// Panics if `line` is 0.
    pub const fn single(line: usize) -> Self {
        assert!(line > 0, "line numbers start at 1");
        Self {
            first: line,
            last: line,
        }
    }

    pub const fn first(self) -> usize {
        self.first
    }

    pub const fn last(self) -> usize {
        self.last
    }

    /// # Returns
    ///
    /// The span as prose: `line 3` for one line, or `lines 3-5`.
    pub fn phrase(self) -> String {
        if self.first == self.last {
            format!("line {self}")
        } else {
            format!("lines {self}")
        }
    }

    /// # Returns
    ///
    /// Whether `line` is in the span.
    pub const fn contains(self, line: usize) -> bool {
        self.first <= line && line <= self.last
    }

    /// # Returns
    ///
    /// Whether the span and `other` share at least one line.
    pub const fn intersects(self, other: Self) -> bool {
        self.first <= other.last && other.first <= self.last
    }
}

impl fmt::Display for LineSpan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.first == self.last {
            write!(formatter, "{}", self.first)
        } else {
            write!(formatter, "{}-{}", self.first, self.last)
        }
    }
}

/// A set of lines, stored as sorted spans that neither overlap nor touch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineRanges {
    spans: Vec<LineSpan>,
}

impl LineRanges {
    /// Adds every line of `span` to the set.
    pub fn insert(&mut self, span: LineSpan) {
        let start = self
            .spans
            .partition_point(|existing| existing.last.saturating_add(1) < span.first);
        let mut merged = span;
        let mut end = start;
        while let Some(existing) = self.spans.get(end) {
            if existing.first > merged.last.saturating_add(1) {
                break;
            }
            merged.first = merged.first.min(existing.first);
            merged.last = merged.last.max(existing.last);
            end += 1;
        }
        self.spans.splice(start..end, [merged]);
    }

    /// Adds every line of `other` to the set.
    pub fn union(&mut self, other: Self) {
        for span in other.spans {
            self.insert(span);
        }
    }

    /// # Returns
    ///
    /// Whether some line of `span` is in the set.
    pub fn intersects(&self, span: LineSpan) -> bool {
        let index = self
            .spans
            .partition_point(|existing| existing.last < span.first);
        self.spans
            .get(index)
            .is_some_and(|existing| existing.intersects(span))
    }
}

impl FromIterator<LineSpan> for LineRanges {
    fn from_iter<IteratorType: IntoIterator<Item = LineSpan>>(spans: IteratorType) -> Self {
        let mut line_ranges = Self::default();
        for span in spans {
            line_ranges.insert(span);
        }
        line_ranges
    }
}

impl FromStr for LineRanges {
    type Err = LineRangesParseError;

    /// Parses a comma-separated list of `N` and `N-M` items, such as `10-20,30,40-45`.
    ///
    /// # Returns
    ///
    /// The union of the listed spans on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`LineRangesParseError::InvalidRange`] if a range starts at 0 or ends before it starts.
    /// * Forwards [`parse_line_number`]'s return values on failure.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut bounds = Vec::new();
        for item in text.split(',') {
            let (first, last) = if let Some((first, last)) = item.split_once('-') {
                (parse_line_number(first)?, parse_line_number(last)?)
            } else {
                let line = parse_line_number(item)?;
                (line, line)
            };
            bounds.push((first, last));
        }
        let mut line_ranges = Self::default();
        for (first, last) in bounds {
            let span = LineSpan::new(first, last)
                .ok_or(LineRangesParseError::InvalidRange { first, last })?;
            line_ranges.insert(span);
        }
        Ok(line_ranges)
    }
}

/// Why a `N[-M](,N[-M])*` text couldn't be turned into [`LineRanges`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRangesParseError {
    /// The text doesn't have the `N[-M](,N[-M])*` form.
    Syntax,

    /// The text has the right form, but a line number overflows `usize`.
    Overflow,

    /// The text has the right form, but a range starts at 0 or ends before it starts.
    InvalidRange { first: usize, last: usize },
}

/// Parses one line number of a `N[-M]` item.
///
/// # Returns
///
/// The line number on success.
///
/// # Errors
///
/// Returns an error if:
///
/// * [`LineRangesParseError::Syntax`] if `text` is empty or has a character other than an ASCII
///   digit.
/// * [`LineRangesParseError::Overflow`] if the number overflows `usize`.
fn parse_line_number(text: &str) -> Result<usize, LineRangesParseError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(LineRangesParseError::Syntax);
    }
    text.parse().map_err(|_| LineRangesParseError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_ranges(spans: &[(usize, usize)]) -> LineRanges {
        spans
            .iter()
            .map(|(first, last)| LineSpan::new(*first, *last).expect("test spans must be valid"))
            .collect()
    }

    fn parse_ranges(text: &str) -> Result<LineRanges, LineRangesParseError> {
        text.parse()
    }

    fn span(first: usize, last: usize) -> LineSpan {
        LineSpan::new(first, last).expect("test spans must be valid")
    }

    #[test]
    fn phrase_is_singular_for_one_line() {
        assert_eq!(LineSpan::single(3).phrase(), "line 3");
        assert_eq!(
            LineSpan::new(3, 5).expect("valid span").phrase(),
            "lines 3-5"
        );
    }

    #[test]
    fn parse_accepts_single_lines_ranges_and_lists() {
        assert_eq!(
            parse_ranges("10-20,30,40-45"),
            Ok(line_ranges(&[(10, 20), (30, 30), (40, 45)]))
        );
        assert_eq!(parse_ranges("7"), Ok(line_ranges(&[(7, 7)])));
    }

    #[test]
    fn parse_merges_overlapping_and_adjacent_ranges() {
        let parsed = parse_ranges("30-40,1-3,4,35-50,52").expect("valid ranges");
        assert_eq!(parsed.spans, [span(1, 4), span(30, 50), span(52, 52)]);
    }

    #[test]
    fn parse_rejects_zero_and_reversed_ranges() {
        assert_eq!(
            parse_ranges("0"),
            Err(LineRangesParseError::InvalidRange { first: 0, last: 0 })
        );
        assert_eq!(
            parse_ranges("5,20-10"),
            Err(LineRangesParseError::InvalidRange {
                first: 20,
                last: 10
            })
        );
    }

    #[test]
    fn parse_rejects_text_outside_the_range_syntax() {
        for text in ["", "a", "1-", "-1", "1,,2", "1-2-3", " 1", "1;2", "+1"] {
            assert_eq!(
                parse_ranges(text),
                Err(LineRangesParseError::Syntax),
                "{text:?}"
            );
        }
    }

    #[test]
    fn parse_rejects_line_numbers_that_overflow() {
        assert_eq!(
            parse_ranges("1-99999999999999999999999"),
            Err(LineRangesParseError::Overflow)
        );
    }

    #[test]
    fn intersects_checks_every_span() {
        let parsed = parse_ranges("10-20,30").expect("valid ranges");
        assert!(parsed.intersects(span(20, 25)));
        assert!(parsed.intersects(span(1, 10)));
        assert!(parsed.intersects(span(25, 35)));
        assert!(parsed.intersects(span(30, 30)));
        assert!(!parsed.intersects(span(21, 29)));
        assert!(!parsed.intersects(span(31, 40)));
        assert!(!parsed.intersects(span(9, 9)));
    }

    #[test]
    fn union_combines_both_sets() {
        let mut parsed = parse_ranges("1-5,20").expect("valid ranges");
        parsed.union(parse_ranges("6-10,15-19").expect("valid ranges"));
        assert_eq!(parsed.spans, [span(1, 10), span(15, 20)]);
    }
}
