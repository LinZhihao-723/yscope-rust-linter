//! A Rust comment lexer: extracts every `//` line comment from source text, skipping string, char
//! and block-comment contents so that a `//` inside them is never taken for a comment.

use std::fmt;

use crate::diagnostic::Position;

/// The kind of a line comment, decided by its marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommentMarker {
    /// `//`, and runs of four or more slashes, which rustdoc treats as plain comments.
    Plain,

    /// `///`.
    OuterDoc,

    /// `//!`.
    InnerDoc,
}

/// One `//` line comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentLine {
    /// The 1-based line number.
    pub line: usize,

    /// The 1-based column of the marker's first `/`, counted in Unicode scalar values.
    pub marker_column: usize,

    pub marker: CommentMarker,

    /// The number of slashes the marker starts with: 2 for `//` and `//!`, 3 for `///`, and more
    /// for a longer run.
    pub num_slashes: usize,

    /// Everything after the marker up to the end of the line, without a trailing `\r`.
    pub text: String,

    /// Whether only whitespace precedes the marker on its line.
    pub is_own_line: bool,

    /// The scalar count of the whole source line with its trailing whitespace removed.
    pub width: usize,
}

impl CommentLine {
    /// # Returns
    ///
    /// Whether `self` continues the comment block that `previous` belongs to: both are own-line
    /// comments with the same marker and marker column, on consecutive lines.
    pub fn continues_block(&self, previous: &Self) -> bool {
        self.line == previous.line + 1
            && self.is_own_line
            && previous.is_own_line
            && self.marker_column == previous.marker_column
            && self.marker == previous.marker
            && self.num_slashes == previous.num_slashes
    }
}

/// A literal or block comment that the source text never closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexError {
    pub kind: LexErrorKind,

    /// The 1-based line where the unterminated token starts.
    pub line: usize,

    /// The 1-based column where the unterminated token starts.
    pub column: usize,
}

/// The kind of token that a [`LexError`] leaves unterminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexErrorKind {
    BlockComment,
    String,
    RawString,
    Char,
}

impl fmt::Display for LexErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::BlockComment => "block comment",
            Self::String => "string literal",
            Self::RawString => "raw string literal",
            Self::Char => "char literal",
        };
        formatter.write_str(description)
    }
}

/// The result of lexing one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexed {
    /// The comment lines in source order. After a lex error, only the ones before the error.
    pub comment_lines: Vec<CommentLine>,

    pub error: Option<LexError>,

    /// The position of the first token after the file header, or [`None`] if the whole lexed text
    /// is header. The header is the leading whitespace, non-doc comments, inner doc comments and
    /// inner attributes, so an outer doc comment, an outer attribute or any other token ends it.
    pub header_end: Option<Position>,
}

/// Lexes Rust source text into its line comments.
///
/// # Returns
///
/// The comment lines in source order, plus the error that stopped the lexer, if any.
pub fn lex(source_text: &str) -> Lexed {
    Lexer::new(source_text).run()
}

/// The cursor over the source text's scalar values.
struct Lexer {
    chars: Vec<char>,
    index: usize,
    line: usize,
    line_start: usize,
    comment_lines: Vec<CommentLine>,
    header: Header,
}

impl Lexer {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// A lexer at the start of `source_text`, past a byte order mark.
    fn new(source_text: &str) -> Self {
        let chars: Vec<char> = source_text.chars().collect();
        let index = usize::from(chars.first() == Some(&'\u{feff}'));
        Self {
            chars,
            index,
            line: 1,
            line_start: index,
            comment_lines: Vec::new(),
            header: Header::Open,
        }
    }

    /// Lexes the whole source text.
    ///
    /// # Returns
    ///
    /// The comment lines, and the error that stopped the lexer, if any.
    fn run(mut self) -> Lexed {
        self.skip_shebang();
        let error = self.lex_code().err();
        let header_end = match self.header {
            Header::Ended { end } => Some(end),
            Header::Open | Header::AttributeStart | Header::Attribute { .. } => None,
        };
        Lexed {
            comment_lines: self.comment_lines,
            error,
            header_end,
        }
    }

    /// Lexes code until the end of the source text.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * Forwards [`Self::lex_block_comment`]'s return values on failure.
    /// * Forwards [`Self::lex_string`]'s return values on failure.
    /// * Forwards [`Self::lex_quote`]'s return values on failure.
    /// * Forwards [`Self::lex_identifier_or_prefixed_literal`]'s return values on failure.
    fn lex_code(&mut self) -> Result<(), LexError> {
        while let Some(current) = self.peek(0) {
            self.track_header(current);
            match current {
                '/' if self.peek(1) == Some('/') => self.lex_line_comment(),
                '/' if self.peek(1) == Some('*') => self.lex_block_comment()?,
                '"' => self.lex_string(1, LexErrorKind::String)?,
                '\'' => self.lex_quote()?,
                _ if is_identifier_start(current) => self.lex_identifier_or_prefixed_literal()?,
                _ if current.is_ascii_digit() => self.skip_while(is_identifier_continue),
                _ => self.advance(),
            }
        }
        Ok(())
    }

    /// Moves the header state past the scalar at the cursor, which is `current`. The header ends at
    /// the first scalar that can't be part of it:
    ///
    /// * Whitespace keeps the header open.
    /// * A `//` or `/*` comment keeps it open, unless it is an outer doc comment (`///` or `/**`).
    /// * `#!` starts an inner attribute, which keeps the header open through its closing `]`.
    /// * Any other scalar ends it.
    fn track_header(&mut self, current: char) {
        self.header = match self.header {
            Header::Ended { .. } => return,
            Header::Attribute { depth } => match current {
                '[' => Header::Attribute { depth: depth + 1 },
                ']' if depth == 1 => Header::Open,
                ']' => Header::Attribute { depth: depth - 1 },
                _ => return,
            },
            Header::AttributeStart => match current {
                '[' => Header::Attribute { depth: 1 },
                '!' => return,
                _ if current.is_whitespace() => return,
                _ => self.header_ended_here(),
            },
            Header::Open => match (current, self.peek(1), self.peek(2), self.peek(3)) {
                (..) if current.is_whitespace() => return,
                ('/', Some('/'), Some('/'), next) if next != Some('/') => self.header_ended_here(),
                ('/', Some('*'), Some('*'), Some(next)) if next != '*' && next != '/' => {
                    self.header_ended_here()
                }
                ('/', Some('/' | '*'), ..) => return,
                ('#', ..) if self.next_non_whitespace() == Some('!') => Header::AttributeStart,
                _ => self.header_ended_here(),
            },
        };
    }

    /// # Returns
    ///
    /// The first scalar after the cursor that isn't whitespace, or [`None`] if there is none.
    fn next_non_whitespace(&self) -> Option<char> {
        self.chars
            .get(self.index + 1..)?
            .iter()
            .copied()
            .find(|current| !current.is_whitespace())
    }

    /// # Returns
    ///
    /// The header state of a header that ends at the cursor.
    const fn header_ended_here(&self) -> Header {
        Header::Ended {
            end: Position {
                line: self.line,
                column: self.column(),
            },
        }
    }

    /// Skips a first line that starts with `#!` but not `#![`.
    fn skip_shebang(&mut self) {
        if self.peek(0) == Some('#') && self.peek(1) == Some('!') && self.peek(2) != Some('[') {
            self.skip_while(|current| current != '\n');
        }
    }

    /// Records the line comment at the cursor and moves the cursor to the end of its line.
    fn lex_line_comment(&mut self) {
        let marker_start = self.index;
        let marker_column = self.column();
        let is_own_line = self.chars[self.line_start..marker_start]
            .iter()
            .all(|current| current.is_whitespace());
        self.skip_while(|current| current == '/');
        let num_slashes = self.index - marker_start;
        let marker = match num_slashes {
            2 if self.peek(0) == Some('!') => {
                self.advance();
                CommentMarker::InnerDoc
            }
            3 => CommentMarker::OuterDoc,
            _ => CommentMarker::Plain,
        };
        let text_start = self.index;
        self.skip_while(|current| current != '\n');
        let line_text = &self.chars[self.line_start..self.index];
        let text = &self.chars[text_start..self.index];
        let text = text.strip_suffix(&['\r']).unwrap_or(text);
        let width = line_text.len()
            - line_text
                .iter()
                .rev()
                .take_while(|current| current.is_whitespace())
                .count();
        self.comment_lines.push(CommentLine {
            line: self.line,
            marker_column,
            marker,
            num_slashes,
            text: text.iter().collect(),
            is_own_line,
            width,
        });
    }

    /// Skips a possibly nested block comment that starts at the cursor.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`LexError`] if the block comment is never closed.
    fn lex_block_comment(&mut self) -> Result<(), LexError> {
        let error = self.error_here(LexErrorKind::BlockComment);
        self.advance_by(2);
        let mut depth = 1_usize;
        while depth > 0 {
            match (self.peek(0), self.peek(1)) {
                (None, _) => return Err(error),
                (Some('/'), Some('*')) => {
                    depth += 1;
                    self.advance_by(2);
                }
                (Some('*'), Some('/')) => {
                    depth -= 1;
                    self.advance_by(2);
                }
                _ => self.advance(),
            }
        }
        Ok(())
    }

    /// Skips a string literal whose opening `"` is `prefix_length - 1` scalars after the cursor.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`LexError`] of `kind` if the literal is never closed.
    fn lex_string(&mut self, prefix_length: usize, kind: LexErrorKind) -> Result<(), LexError> {
        let error = self.error_here(kind);
        self.advance_by(prefix_length);
        loop {
            match self.peek(0) {
                None => return Err(error),
                Some('\\') => self.advance_by(2),
                Some('"') => {
                    self.advance();
                    return Ok(());
                }
                Some(_) => self.advance(),
            }
        }
    }

    /// Skips a raw string literal: `prefix_length` scalars of prefix (such as `r`, `br` or `cr`),
    /// then `num_hashes` `#`, then the opening `"`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`LexError`] if the literal is never closed.
    fn lex_raw_string(&mut self, prefix_length: usize, num_hashes: usize) -> Result<(), LexError> {
        let error = self.error_here(LexErrorKind::RawString);
        self.advance_by(prefix_length + num_hashes + 1);
        loop {
            match self.peek(0) {
                None => return Err(error),
                Some('"') if (1..=num_hashes).all(|offset| self.peek(offset) == Some('#')) => {
                    self.advance_by(num_hashes + 1);
                    return Ok(());
                }
                Some(_) => self.advance(),
            }
        }
    }

    /// Skips a char literal, a lifetime or a label that starts with the `'` at the cursor.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * Forwards [`Self::lex_char`]'s return values on failure.
    fn lex_quote(&mut self) -> Result<(), LexError> {
        if self.peek(1) == Some('\\') || self.peek(2) == Some('\'') {
            return self.lex_char(1);
        }
        self.advance();
        Ok(())
    }

    /// Skips a char literal whose opening `'` is `prefix_length - 1` scalars after the cursor.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`LexError`] if the literal is never closed.
    fn lex_char(&mut self, prefix_length: usize) -> Result<(), LexError> {
        let error = self.error_here(LexErrorKind::Char);
        self.advance_by(prefix_length);
        loop {
            match self.peek(0) {
                None => return Err(error),
                Some('\\') => self.advance_by(2),
                Some('\'') => {
                    self.advance();
                    return Ok(());
                }
                Some(_) => self.advance(),
            }
        }
    }

    /// Skips an identifier, a raw identifier, or a literal with a `b`, `c`, `r`, `br` or `cr`
    /// prefix, starting at the cursor.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * Forwards [`Self::lex_string`]'s return values on failure.
    /// * Forwards [`Self::lex_raw_string`]'s return values on failure.
    /// * Forwards [`Self::lex_char`]'s return values on failure.
    fn lex_identifier_or_prefixed_literal(&mut self) -> Result<(), LexError> {
        let (prefix_length, may_be_raw) = match (self.peek(0), self.peek(1)) {
            (Some('b' | 'c'), Some('r')) => (2, true),
            (Some('r'), _) => (1, true),
            (Some('b' | 'c'), _) => (1, false),
            _ => (0, false),
        };
        if prefix_length > 0 {
            if self.peek(prefix_length) == Some('"') {
                if may_be_raw {
                    return self.lex_raw_string(prefix_length, 0);
                }
                return self.lex_string(prefix_length + 1, LexErrorKind::String);
            }
            if may_be_raw {
                let num_hashes = self
                    .chars
                    .get(self.index + prefix_length..)
                    .map_or(0, |rest| {
                        rest.iter().take_while(|current| **current == '#').count()
                    });
                if num_hashes > 0 && self.peek(prefix_length + num_hashes) == Some('"') {
                    return self.lex_raw_string(prefix_length, num_hashes);
                }
            }
            if self.peek(0) == Some('b') && self.peek(1) == Some('\'') {
                return self.lex_char(2);
            }
        }
        self.skip_while(is_identifier_continue);
        if self.peek(0) == Some('#') && self.peek(1).is_some_and(is_identifier_start) {
            self.advance();
            self.skip_while(is_identifier_continue);
        }
        Ok(())
    }

    /// # Returns
    ///
    /// The scalar `offset` scalars after the cursor, or [`None`] past the end of the text.
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index + offset).copied()
    }

    /// # Returns
    ///
    /// The 1-based column of the cursor.
    const fn column(&self) -> usize {
        self.index - self.line_start + 1
    }

    /// # Returns
    ///
    /// A lex error of `kind` at the cursor, for a token that starts there.
    const fn error_here(&self, kind: LexErrorKind) -> LexError {
        LexError {
            kind,
            line: self.line,
            column: self.column(),
        }
    }

    /// Moves the cursor one scalar forward, keeping the line count in step.
    fn advance(&mut self) {
        if let Some(current) = self.peek(0) {
            self.index += 1;
            if current == '\n' {
                self.line += 1;
                self.line_start = self.index;
            }
        }
    }

    /// Moves the cursor `count` scalars forward.
    fn advance_by(&mut self, count: usize) {
        for _ in 0..count {
            self.advance();
        }
    }

    /// Moves the cursor forward while the scalar under it satisfies `predicate`.
    fn skip_while(&mut self, predicate: impl Fn(char) -> bool) {
        while self.peek(0).is_some_and(&predicate) {
            self.advance();
        }
    }
}

/// How far the lexer is through the file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Header {
    /// Only header tokens so far, outside any inner attribute.
    Open,

    /// After the `#` of an inner attribute, before its `[`.
    AttributeStart,

    /// Inside the brackets of an inner attribute, `depth` levels deep.
    Attribute { depth: usize },

    /// Ended at `end`.
    Ended { end: Position },
}

/// # Returns
///
/// Whether `current` can start an identifier.
fn is_identifier_start(current: char) -> bool {
    current == '_' || current.is_alphabetic()
}

/// # Returns
///
/// Whether `current` can continue an identifier or a number.
fn is_identifier_continue(current: char) -> bool {
    current == '_' || current.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// # Returns
    ///
    /// The `(line, marker_column, num_slashes, text)` of every comment line in `source_text`.
    fn comments(source_text: &str) -> Vec<(usize, usize, usize, String)> {
        let lexed = lex(source_text);
        assert_eq!(lexed.error, None, "{source_text:?}");
        lexed
            .comment_lines
            .into_iter()
            .map(|comment_line| {
                (
                    comment_line.line,
                    comment_line.marker_column,
                    comment_line.num_slashes,
                    comment_line.text,
                )
            })
            .collect()
    }

    fn comment(
        line: usize,
        marker_column: usize,
        num_slashes: usize,
        text: &str,
    ) -> (usize, usize, usize, String) {
        (line, marker_column, num_slashes, text.to_owned())
    }

    #[test]
    fn markers_are_classified_with_their_line_and_column() {
        let lexed = lex("// plain\n/// outer\n    //! inner\n//// four\n///! outer bang\n");
        let markers: Vec<_> = lexed
            .comment_lines
            .iter()
            .map(|comment_line| {
                (
                    comment_line.line,
                    comment_line.marker_column,
                    comment_line.marker,
                    comment_line.num_slashes,
                )
            })
            .collect();
        assert_eq!(
            markers,
            [
                (1, 1, CommentMarker::Plain, 2),
                (2, 1, CommentMarker::OuterDoc, 3),
                (3, 5, CommentMarker::InnerDoc, 2),
                (4, 1, CommentMarker::Plain, 4),
                (5, 1, CommentMarker::OuterDoc, 3),
            ]
        );
        assert_eq!(lexed.comment_lines[4].text, "! outer bang");
    }

    #[test]
    fn comment_text_and_width_drop_the_carriage_return_and_trailing_whitespace() {
        let lexed = lex("  /// é text  \r\n");
        let comment_line = &lexed.comment_lines[0];
        assert_eq!(comment_line.text, " é text  ");
        assert_eq!(comment_line.width, 12);
        assert!(comment_line.is_own_line);
    }

    #[test]
    fn slashes_inside_string_literals_are_not_comments() {
        let source_text = concat!(
            "let a = \"http://x\"; // a\n",
            "let b = b\"//\"; let c = c\"//\"; // b\n",
            "let d = r\"//\"; let e = br\"//\"; let f = cr\"//\"; // c\n",
            "let g = r#\"// \"# still \"## inside\"#; // d\n",
            "let h = r##\"\"# //\"##; // e\n",
            "let i = \"\\\" //\"; // f\n",
        );
        assert_eq!(
            comments(source_text),
            [
                comment(1, 21, 2, " a"),
                comment(2, 31, 2, " b"),
                comment(3, 48, 2, " c"),
                comment(4, 38, 2, " d"),
                comment(5, 23, 2, " e"),
                comment(6, 18, 2, " f"),
            ]
        );
    }

    #[test]
    fn char_literals_and_lifetimes_do_not_confuse_the_lexer() {
        let source_text = concat!(
            "let a = ['/', '/']; // a\n",
            "let b = ['\\'', '\\\\', '\"', b'\"', 'é']; // \"b\n",
            "fn f<'a>(x: &'a str) -> &'static str { 'outer: loop { break 'outer; } } // c\n",
        );
        let lexed = comments(source_text);
        let texts: Vec<_> = lexed
            .iter()
            .map(|(line, _, _, text)| (*line, text.as_str()))
            .collect();
        assert_eq!(texts, [(1, " a"), (2, " \"b"), (3, " c")]);
    }

    #[test]
    fn block_comments_hide_slashes_even_when_nested() {
        let source_text = "/* // */ // a\n/* /* */ // */ // b\n/*\n// c\n*/ // d\n";
        let texts: Vec<_> = comments(source_text)
            .into_iter()
            .map(|(line, _, _, text)| (line, text))
            .collect();
        assert_eq!(
            texts,
            [
                (1, " a".to_owned()),
                (2, " b".to_owned()),
                (5, " d".to_owned())
            ]
        );
    }

    #[test]
    fn raw_identifiers_do_not_start_raw_strings() {
        assert_eq!(
            comments("let r#type = 1; // \"a\n"),
            [comment(1, 17, 2, " \"a")]
        );
    }

    #[test]
    fn multi_line_strings_keep_line_numbers() {
        let source_text = "let a = \"one\n// two\\\n three\";\n// four\n";
        assert_eq!(comments(source_text), [comment(4, 1, 2, " four")]);
    }

    #[test]
    fn trailing_comments_are_not_own_line() {
        let lexed = lex("let a = 1; // trailing\n\t// own line\n");
        let own_line_flags: Vec<_> = lexed
            .comment_lines
            .iter()
            .map(|comment_line| comment_line.is_own_line)
            .collect();
        assert_eq!(own_line_flags, [false, true]);
    }

    #[test]
    fn shebang_line_is_skipped() {
        assert_eq!(
            comments("#!/usr/bin/env run // x\n// a\n"),
            [comment(2, 1, 2, " a")]
        );
        assert_eq!(comments("#![allow(x)] // a\n"), [comment(1, 14, 2, " a")]);
    }

    #[test]
    fn unterminated_tokens_are_lex_errors_after_the_comments_before_them() {
        let cases = [
            ("// a\n  /* /* */\n", LexErrorKind::BlockComment, 2, 3),
            ("// a\nlet s = \"open\n", LexErrorKind::String, 2, 9),
            ("// a\nlet s = r#\"open\"\n", LexErrorKind::RawString, 2, 9),
            ("// a\nlet c = '\\", LexErrorKind::Char, 2, 9),
        ];
        for (source_text, kind, line, column) in cases {
            let lexed = lex(source_text);
            assert_eq!(
                lexed.error,
                Some(LexError { kind, line, column }),
                "{source_text:?}"
            );
            assert_eq!(lexed.comment_lines.len(), 1, "{source_text:?}");
        }
    }

    #[test]
    fn continues_block_requires_consecutive_own_line_comments_with_the_same_marker() {
        let lexed = lex("// a\n// b\n/// c\n  /// d\n  /// e\n\n  /// f\nx; // g\n// h\n");
        let comment_lines = &lexed.comment_lines;
        let continues: Vec<_> = comment_lines
            .windows(2)
            .map(|pair| pair[1].continues_block(&pair[0]))
            .collect();
        assert_eq!(continues, [true, false, false, true, false, false, false]);
    }

    #[test]
    fn header_ends_at_the_first_token_outside_comments_inner_docs_and_inner_attributes() {
        let header_end = |source_text: &str| {
            lex(source_text)
                .header_end
                .map(|position| (position.line, position.column))
        };
        assert_eq!(
            header_end(concat!(
                "//! Docs.\n",
                "/*! Block docs. */\n",
                "#![doc = concat![\"]\"]]\n",
                "# ! [cfg_attr(a, allow(b))]\n",
                "/** Item docs. */\n",
            )),
            Some((5, 1))
        );
        assert_eq!(
            header_end("// a\n//// b\n/**/ /*** c */\n  /// d\n"),
            Some((4, 3))
        );
        assert_eq!(header_end("#!/usr/bin/env run\n\n#[test]\n"), Some((3, 1)));
        assert_eq!(header_end("// a\n\n  fn f() {}\n"), Some((3, 3)));
        assert_eq!(header_end("//! a\n#![allow(b)]\n"), None);
    }
}
