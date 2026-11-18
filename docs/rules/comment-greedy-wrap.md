# `comment-greedy-wrap`

Every paragraph of a `//`, `///` or `//!` comment must be filled greedily to the width, which is 100
columns by default. Each line holds as many words as fit, so a line is flagged when the first word
of the next line in the same paragraph would still fit at its end.

The rule only reports under-filled lines. It never rewrites a comment, and it never flags a line for
being too long.

```rust
/// Returns the number of tasks that are
/// still running.
fn running_tasks() -> usize {
    0
}
```

```console
$ yscope-rust-linter example.rs
example.rs:1:1: comment-greedy-wrap: line is 40 columns wide and the next line's first word `still` (5 columns) fits within 100; move it up
```

The fix fills the paragraph:

```rust
/// Returns the number of tasks that are still running.
fn running_tasks() -> usize {
    0
}
```

## Contents

* [The fit test](#the-fit-test)
* [Which comments are checked](#which-comments-are-checked)
* [Comment blocks](#comment-blocks)
* [Paragraphs inside a block](#paragraphs-inside-a-block)
* [The diagnostic and how to fix it](#the-diagnostic-and-how-to-fix-it)
* [Known limits](#known-limits)

## The fit test

A line `L` is flagged when the next line `N` is in the same paragraph and this holds:

```text
width(L) + 1 + width(first_word(N)) <= W
```

* `width(L)` is the width of the whole source line of `L`, without its trailing whitespace.
* `first_word(N)` is the text of `N` after its comment marker, up to the first whitespace after its
  leading whitespace.
* The `1` is the space that would join the two.
* `W` is the width, 100 by default.

The test is exact at the width. Here the first line is 95 columns wide, and `as` would bring it to
98:

```rust
fn retry() {
    // The scheduler retries each failed task up to three times before it finally marks the job
    // as failed.
}
```

```console
$ yscope-rust-linter example.rs
example.rs:2:5: comment-greedy-wrap: line is 95 columns wide and the next line's first word `as` (2 columns) fits within 100; move it up
```

With `permanently` as the next word, the joined line would be 107 columns wide, so nothing is
flagged:

```rust
fn retry() {
    // The scheduler retries each failed task up to three times before it finally marks the job
    // permanently failed.
}
```

### How widths are counted

* The width is the whole source line: the indentation, the comment marker and the text. An indented
  comment has less room than one at column 1.
* Trailing whitespace doesn't count.
* Every Unicode scalar value counts as one column. A tab is one column too.
* `--width <N>` changes `W` for the whole run.

The same paragraph is flagged at `--width 80`:

```rust
fn retry() {
    // The scheduler retries each failed task up to three times before
    // it finally marks the job as failed.
}
```

```console
$ yscope-rust-linter --width 80 example.rs
example.rs:2:5: comment-greedy-wrap: line is 70 columns wide and the next line's first word `it` (2 columns) fits within 80; move it up
```

`    // éé` is 9 columns wide, although it is 11 bytes long. So `x` fits within 11 columns:

```rust
fn f() {
    // éé
    // x
}
```

```console
$ yscope-rust-linter --width 11 example.rs
example.rs:2:5: comment-greedy-wrap: line is 9 columns wide and the next line's first word `x` (1 column) fits within 11; move it up
```

## Which comments are checked

These comments are checked:

* Plain `//` comments, including runs of four or more slashes such as `////`, which rustdoc treats
  as plain comments.
* Outer doc comments, `///`.
* Inner doc comments, `//!`.

Only own-line comments are checked: comments with nothing but whitespace before them on their line.
A trailing comment, after code on its line, is never checked, and it is never joined with the
comments around it:

```rust
fn f() {
    let timeout = 30; // In seconds, as the
    // config file states.
    let retries = 3; // A short
    let delay = 5; // trailing comment.
}
```

These are never checked:

* Block comments, `/* */`, and block doc comments, `/** */` and `/*! */`.
* Text that looks like a comment inside a string, raw string or char literal, or inside a block
  comment.
* Suppression comments, such as `// yscope-rust-linter: allow(comment-greedy-wrap): ...`. They are
  not part of any paragraph. See [Suppression comments](../suppression.md).

```rust
/* A block comment that ends
early is not checked. */
/** A block doc comment that
ends early is not checked either. */
const URL: &str = "
// Text inside a string literal
// is not a comment.
";
```

Two kinds of files are skipped or checked only in part:

* A file with a plain `//` comment holding the word `@generated` in its first five lines is skipped.
* A file with an unterminated literal or block comment gets a `parse-error` diagnostic. Only the
  comments before the unterminated token are checked.

## Comment blocks

A block is a run of own-line comments on consecutive lines with the same marker at the same column.
Two lines in different blocks are never in the same paragraph. A block ends at any of these:

* A line that isn't a comment, including an empty line.
* A line with a trailing comment.
* A suppression comment.
* A change of marker: `//`, `///`, `//!` and `////` are four different markers.
* A change of the marker's column.

None of these pairs is checked:

```rust
// A plain comment that ends
/// early, then a doc comment.
fn f() {}
```

```rust
//! Inner doc that ends
// early.
fn f() {}
```

```rust
//// Four slashes that end
// early.
fn f() {}
```

```rust
fn f() {
    // Indented four, ends
        // early at eight.
}
```

```rust
fn f() {
    // A comment that ends early.

    // Another comment.
    let a = 1;
    // A third comment.
}
```

## Paragraphs inside a block

Each line's body is its text after the comment marker, without the one space that usually follows
the marker. The body's indent is its number of leading spaces and tabs. The rule recognizes the
Markdown structures below by their bodies, in plain comments and doc comments alike.

Two consecutive lines are in the same paragraph only when the second one is ordinary text at the
paragraph's continuation indent. Everything else starts something new.

### Blank comment lines

A blank comment line ends the paragraph. The next paragraph is checked on its own:

```rust
/// Starts the scheduler.
///
/// The scheduler runs until the
/// shutdown signal arrives.
fn start() {}
```

```console
$ yscope-rust-linter example.rs
example.rs:3:1: comment-greedy-wrap: line is 32 columns wide and the next line's first word `shutdown` (8 columns) fits within 100; move it up
```

### Code fences

A fence is a line of three or more backticks or tildes, after any indent. The fence lines and every
line between them are code, which is never checked:

````rust
/// Parses a config.
///
/// ```
/// let config = parse("a = 1");
/// let b = 2;
/// ```
fn parse() {}
````

* A fence closes only on a fence line of the same character that is at least as long and has no info
  string. So `~~~` inside a backtick fence, or ```` ``` ```` inside a ````` ```` ````` fence, is
  still code.
* A backtick fence's info string can't hold a backtick, so ```` ```a``` ```` starts a prose line.
* A fence that is never closed makes the rest of its block code. The next block starts outside any
  fence.

````rust
/// ````text
/// ```
/// still code
/// ~~~
/// still code
/// ````
/// Prose after the fence
/// is checked.
fn f() {}
````

```console
$ yscope-rust-linter example.rs
example.rs:7:1: comment-greedy-wrap: line is 25 columns wide and the next line's first word `is` (2 columns) fits within 100; move it up
```

### Headers

A header is never filled, and nothing is pulled up onto it. A header is an ATX header: an indent of
at most 3, then 1 to 6 `#`, then whitespace or the end of the line.

```rust
/// Creates a sender.
/// # Returns
/// A new sender.
fn sender() {}
```

Text that starts with `#` but isn't an ATX header, such as `#544` or `#[derive(Debug)]`, is prose:

```rust
fn f() {
    // This was fixed in issue
    // #544, so the workaround is gone.
}
```

```console
$ yscope-rust-linter example.rs
example.rs:2:5: comment-greedy-wrap: line is 30 columns wide and the next line's first word `#544,` (5 columns) fits within 100; move it up
```

A header also closes every open list item. See [Indentation changes](#indentation-changes).

### Bullets

A list item starts with `*`, `-` or `+`, or with 1 to 9 digits followed by `.` or `)`. The marker
needs at least one space or tab after it. Each list item starts a new paragraph, so neither the line
before it nor the item before it is joined with it:

```rust
/// The steps are:
/// * Connect to the database.
/// * Run the migrations.
/// - Start the server.
/// 1. Ordered items work too.
/// 2) And this style.
fn f() {}
```

Without whitespace after the marker, the line is prose. `--verbose`, `*emphasis*` and `3.5 seconds`
are not list items:

```rust
/// The flag
/// --verbose is not a list item.
fn f() {}
```

```console
$ yscope-rust-linter example.rs
example.rs:1:1: comment-greedy-wrap: line is 12 columns wide and the next line's first word `--verbose` (9 columns) fits within 100; move it up
```

### Bullet continuations

A list item's content indent is the column where its text starts, counted from the start of the
body. A text line at exactly that indent continues the item, and that pair is checked. This holds
for nested and ordered items too:

```rust
/// * A bullet whose text ends
///   early.
/// * A nested list:
///   * An item whose text ends
///     early.
/// 10. An ordered item that ends
///     early.
fn f() {}
```

```console
$ yscope-rust-linter example.rs
example.rs:1:1: comment-greedy-wrap: line is 30 columns wide and the next line's first word `early.` (6 columns) fits within 100; move it up
example.rs:4:1: comment-greedy-wrap: line is 31 columns wide and the next line's first word `early.` (6 columns) fits within 100; move it up
example.rs:6:1: comment-greedy-wrap: line is 33 columns wide and the next line's first word `early.` (6 columns) fits within 100; move it up
```

A list item stays open across blank lines. So a later paragraph of the same item, at its content
indent, is prose and is checked:

```rust
/// * A list item.
///
///   Its second paragraph ends
///   early.
fn f() {}
```

```console
$ yscope-rust-linter example.rs
example.rs:3:1: comment-greedy-wrap: line is 31 columns wide and the next line's first word `early.` (6 columns) fits within 100; move it up
```

### Indentation changes

A line whose indent differs from the paragraph's continuation indent starts a new paragraph:

* A text line at indent 0 right after a list item is a new paragraph. It also closes the open list
  items.
* A line indented deeper than the item's content indent starts a new paragraph.

```rust
/// * A bullet.
/// A new paragraph without a blank line.
/// * Another bullet.
///     An over-indented line.
fn f() {}
```

An indented text line that starts a paragraph is prose only at the content indent of an open list
item. Anywhere else it starts a preformatted paragraph, which is never checked. This keeps aligned
case lists and indented code blocks as they are:

```rust
fn f() {
    // Cases:
    //
    //   index 0: alive, terminated
    //   index 1: alive, timed out
    //
    // A paragraph whose next line
    //   is indented deeper.
}
```

```rust
/// Example:
///
///     let a = 1;
///     let b = 2;
fn f() {}
```

A header closes every open list item, so indented text after it is preformatted:

```rust
/// * A list item.
///
/// # Details
///
///   This indented text is not part of
///   the list item any more.
fn f() {}
```

### Tables

A line whose body starts with `|` is a table row. A table row is never joined with the line before
or after it:

```rust
/// | Key | Value |
/// | --- | ----- |
/// | a   | 1     |
/// This line follows a
/// table row.
fn f() {}
```

```console
$ yscope-rust-linter example.rs
example.rs:4:1: comment-greedy-wrap: line is 23 columns wide and the next line's first word `table` (5 columns) fits within 100; move it up
```

A row without the leading `|`, such as `a | b |`, is prose.

### Block quotes

A line whose body starts with `>` is a block quote line. It is never joined with the line before or
after it:

```rust
/// > A quoted line that ends
/// > early.
/// > Another quoted line.
fn f() {}
```

### Tags

A line that starts with one of these tags, followed by `:`, starts a new paragraph: `NOTE`, `TODO`,
`FIXME`, `SAFETY`, `XXX`, `HACK`, `WARNING` and `IMPORTANT`. The tag must be upper case and directly
followed by the colon. The tagged paragraph's own continuation lines are checked:

```rust
fn f() {
    // Write the task outputs
    // NOTE: There is only one output type.
    // TODO: Support more output
    // types.
}
```

```console
$ yscope-rust-linter example.rs
example.rs:4:5: comment-greedy-wrap: line is 32 columns wide and the next line's first word `types.` (6 columns) fits within 100; move it up
```

`Note:` and `NOTES:` are not tags, so they are prose:

```rust
fn f() {
    // Write the task outputs
    // Note: lowercase is not a tag.
}
```

```console
$ yscope-rust-linter example.rs
example.rs:2:5: comment-greedy-wrap: line is 29 columns wide and the next line's first word `Note:` (5 columns) fits within 100; move it up
```

### Formula and pseudo-code lines

A formula line is never joined with the line before or after it. A line is a formula when it has all
of these, separated by whitespace:

* A first token that starts with a letter or `_`, and holds only letters, digits, `_`, `.`, `:`,
  `[`, `]`, `(` and `)`.
* One of these operators: `=`, `==`, `!=`, `<=`, `>=`, `+=`, `-=`, `*=`, `/=`, `:=`, `->` and `=>`.
* At least one more word.

```rust
fn f() {
    // soft_timeout = registered_at + 100ms
    // deadline = now - 900ms
    // Validate the fan-out case:
    // task_5.output_1 -> task_6.input_0
    // a[i] += 1
    // Prose after a formula
    // is checked.
}
```

```console
$ yscope-rust-linter example.rs
example.rs:7:5: comment-greedy-wrap: line is 28 columns wide and the next line's first word `is` (2 columns) fits within 100; move it up
```

`The total number = a + b` is prose, since `number` is not an operator. `x =` is prose, since
nothing follows the operator.

### Link reference definitions

A link reference definition is a `[label]:` line with an indent of at most 3, where the label isn't
blank and has no `[` or `]`. The `:` must end the line or be followed by whitespace. The definition
and its continuation lines, indented deeper than it, are never checked:

```rust
/// See [`Foo::bar`].
/// [`Foo::bar`]:
///   crate::foo::Foo::bar
/// [label]: https://example.com
/// Text after the definitions
/// is checked.
fn f() {}
```

```console
$ yscope-rust-linter example.rs
example.rs:5:1: comment-greedy-wrap: line is 30 columns wide and the next line's first word `is` (2 columns) fits within 100; move it up
```

### A trailing colon is not a boundary

A line that ends with `:` doesn't end its paragraph. To introduce a list, use list items, indented
lines or a blank line:

```rust
fn f() {
    // There should be 3 submissions:
    // The first two triggered by size.
}
```

```console
$ yscope-rust-linter example.rs
example.rs:2:5: comment-greedy-wrap: line is 37 columns wide and the next line's first word `The` (3 columns) fits within 100; move it up
```

## The diagnostic and how to fix it

```text
<path>:<line>:<column>: comment-greedy-wrap: line is <W(L)> columns wide and the next line's first word `<word>` (<W(word)> columns) fits within <W>; move it up
```

* The line and column point at the comment marker of the under-filled line.
* A count of 1 reads `1 column`.
* With `--diff` or `FILE:RANGES`, the diagnostic is kept when either of the two lines is selected.
  So a change to the next line can report the line above it.

To fix it:

* Move the words up from the next line, then check the lines below it: each move can leave room on
  the next line too.
* Reflowing the whole paragraph to the width does the same in one step.
* If the lines are short on purpose, give them a structure the rule recognizes: list items, a blank
  comment line, a code fence, a table, or indented (preformatted) lines.
* If none of these fits, suppress the rule with a reason. See [Suppression
  comments](../suppression.md).

## Known limits

* Lines that are too long are never flagged. Only an under-filled line followed by a word that fits
  is reported.
* Widths count Unicode scalar values, not display columns. A tab, a wide CJK character and a
  combining mark each count as one column.
* A hanging-indent continuation, such as `//     c + d` under `// total = a + b +`, starts a
  preformatted paragraph, so the pair isn't checked.
* Commented-out code and pseudo-code without a formula operator, such as `// let a = 1;` followed by
  `// }`, is prose and may be flagged. Put it in a code fence, indent it, or suppress the rule.
* A tagged paragraph runs until a boundary. An unrelated sentence right below a `NOTE:` line, with
  no blank line between them, is joined with it.
* Setext headers, a line followed by `===` or `---`, are not recognized. Use ATX headers (`#`).
* Block comments are never checked.
* After an unterminated literal or block comment, the rest of the file isn't checked.
