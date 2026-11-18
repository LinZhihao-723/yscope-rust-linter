# yscope-rust-linter design

This document is for maintainers. It describes how `yscope-rust-linter` is built: its pipeline, its
modules, and the models behind its rules. What the tool checks and how to use it are documented for
users elsewhere:

* [`README.md`](../README.md): installation, the command line, output and exit codes, and CI use.
* [`docs/rules/comment-greedy-wrap.md`](rules/comment-greedy-wrap.md): the comment rule, with its
  exemptions.
* [`docs/rules/errors-doc.md`](rules/errors-doc.md): the `# Errors` rule, with the accepted
  documentation format and its findings.
* [`docs/rules/symbol-order.md`](rules/symbol-order.md): the item ordering rule.
* [`docs/rules/type-params-doc.md`](rules/type-params-doc.md): the `# Type Parameters` rule.
* [`docs/rules/trait-params-doc.md`](rules/trait-params-doc.md): the `# Parameters` rule.
* [`docs/suppression.md`](suppression.md): the suppression comments.

This document doesn't repeat those. It links to them where the user-facing behavior is the
specification.

## Contents

* [Scope](#scope)
* [Architecture](#architecture)
  * [Module layout](#module-layout)
  * [Code conventions](#code-conventions)
  * [Dependencies](#dependencies)
* [Pipeline](#pipeline)
* [FileContext](#filecontext)
* [Rule registry](#rule-registry)
  * [Adding a rule](#adding-a-rule)
* [Comment lexer](#comment-lexer)
* [Markdown classifier](#markdown-classifier)
* [comment-greedy-wrap](#comment-greedy-wrap)
  * [Fit test](#fit-test)
  * [Blocks](#blocks)
  * [Line kinds](#line-kinds)
  * [Paragraph machine](#paragraph-machine)
  * [Design notes](#design-notes)
* [errors-doc](#errors-doc)
  * [Stages](#stages)
  * [Item walk](#item-walk)
  * [Out-of-line test modules](#out-of-line-test-modules)
  * [Doc lines](#doc-lines)
  * [Parsing the `# Errors` section](#parsing-the--errors-section)
  * [Symbol paths and matching](#symbol-paths-and-matching)
  * [File index](#file-index)
  * [Body scan](#body-scan)
  * [Comparison](#comparison)
* [Parameter sections](#parameter-sections)
* [symbol-order](#symbol-order)
  * [Checked scopes](#checked-scopes)
  * [Sort keys](#sort-keys)
  * [Minimal moves](#minimal-moves)
  * [symbol-order diagnostics](#symbol-order-diagnostics)
* [type-params-doc](#type-params-doc)
* [trait-params-doc](#trait-params-doc)
* [Suppression internals](#suppression-internals)
* [Diagnostics and output](#diagnostics-and-output)
* [Line ranges and the diff filter](#line-ranges-and-the-diff-filter)
* [Testing](#testing)
* [Known limits](#known-limits)
* [Rejected alternatives](#rejected-alternatives)
* [Open questions](#open-questions)

## Scope

The tool enforces y-scope Rust style rules that neither rustfmt nor clippy can check. It was built
against two code bases, [y-scope/clp](https://github.com/y-scope/clp) and
[y-scope/spider](https://github.com/y-scope/spider), and the motivating examples below come from
them.

Goals:

* No false positives on those code bases beyond a short, reviewed list, each of which is either a
  real violation or suppressed with a reason.
* Catch the real misses: under-filled comment lines, missing `Forwards` bullets, missing variant
  bullets, and stale bullets left behind by a refactor.
* Be cheap enough to run in CI on the changed lines only, including the lines a change deleted.

Non-goals:

* Requiring a `# Errors` section. Only existing sections are checked; `clippy::missing_errors_doc`
  covers their presence.
* Requiring docstrings. Every docstring rule skips an item without one: `missing_docs` and review
  cover their presence. `type-params-doc` and `trait-params-doc` do require their section in a
  docstring that exists.
* Type resolution. The tool works on syntax only and never runs rustc or rust-analyzer.
* Cross-file resolution, except the parent-file lookup for [out-of-line test
  modules](#out-of-line-test-modules).
* Auto-fixing. Messages say what to write, but the tool never rewrites files.

## Architecture

The crate is one binary, edition 2024, with no library target. Integration tests drive the built
binary, so nothing needs a public library surface.

### Module layout

Entry and configuration:

* `src/main.rs`: module declarations, the CLI entry, printing, and the exit-code mapping.
* `src/cli.rs`: the `clap` derive struct and its conversion into a validated `LintConfig`.
* `src/config.rs`: `LintConfig`, with the enabled `RuleSet`, the `Width`, `include_tests`, the
  targets, the diff input, the excludes and the `OutputFormat`.
* `src/error.rs`: the fatal `Error` enum (I/O, invalid arguments, invalid diff), with a hand-written
  `Display` and `Error`.

Input selection:

* `src/source.rs`: target parsing, the directory walk, excludes, the `@generated` check, and the
  per-file `LineFilter`.
* `src/line_ranges.rs`: `LineSpan` and `LineRanges`, with parsing, union and intersection.
* `src/diff.rs`: a unified diff to per-file `LineRanges`.

Per-file core:

* `src/driver.rs`: the run, and the per-file pipeline in `FileChecker`.
* `src/file_context.rs`: [`FileContext`](#filecontext), the per-file input of the rules.
* `src/lexer.rs`: the [comment lexer](#comment-lexer).
* `src/markdown.rs`: the [line-level Markdown classifier](#markdown-classifier) the comment and
  docstring rules share.
* `src/suppression.rs`: [suppression](#suppression-internals) parsing, coverage, and
  unused-suppression reporting.
* `src/diagnostic.rs`: diagnostic codes, positions, anchor spans, sorting and rendering.

Rule-agnostic helpers:

* `src/syntax/`: helpers over `syn` trees. `span.rs` converts span locations to positions,
  `item_lines.rs` finds the lines of every item or of one item, `symbol_path.rs` normalizes paths,
  `types.rs` looks through parenthesized types, `test_code.rs` recognizes test attributes and
  out-of-line test modules, `visibility.rs` classifies visibilities into the ordering groups,
  `crate_names.rs` tells crate-local paths from foreign ones, and `doc_items.rs` walks the items
  whose docstrings document their signatures.
* `src/doc/`: helpers over docstrings. `doc_lines.rs` turns `#[doc]` attributes into positioned
  lines, `links.rs` finds code spans, links and link reference definitions, `section.rs` selects a
  `# Title` section and groups its lines, and `param_section.rs` parses and compares the sections
  that document parameters one bullet each.

Rules:

* `src/rules/registry.rs`: `RuleId` and the `Rule` enum that dispatches a file to each enabled rule.
* `src/rules/greedy_wrap/`: `line_kind.rs` classifies lines, `paragraph.rs` splits blocks and runs
  the paragraph machine, and `check.rs` holds the `Checker` with the fit test.
* `src/rules/errors_doc/`: `items.rs` (item walk), `errors_section.rs` (section parsing),
  `symbol.rs` (matching), `file_index.rs` (per-file index), `body.rs` (body scan), `pass_through.rs`
  (helper pass-through), `macros.rs` (macro handling), `known_apis.rs` (library API tables) and
  `check.rs` (the `Checker` and the comparison).
* `src/rules/symbol_order/`: `scope.rs` (the counted items of a scope and their keys), `keys.rs`
  (sort keys and reasons), `moves.rs` (the kept subsequence and placements) and `check.rs` (the
  `Checker`, the scope walk and the diagnostics).
* `src/rules/type_params_doc/` and `src/rules/trait_params_doc/`: each a `check.rs` with the rule's
  `Checker`, plus whatever modules the rule needs.

Tests:

* `tests/cli.rs`: end-to-end tests of the binary.

### Code conventions

* `mod.rs` files are tables of contents: declarations and named re-exports only.
* Items inside private modules use plain `pub`. `clippy::nursery` is on, and its
  `redundant_pub_crate` lint rejects `pub(crate)` there. All modules stay private in `main.rs`, so
  nothing escapes the crate.
* Unit tests live next to the code, in `#[cfg(test)] mod tests`.
* There are no tuple structs, including newtypes. `Width` is `struct Width { columns: NonZeroUsize
  }`.
* The tool dogfoods itself: `yscope-rust-linter src tests` must be clean.

### Dependencies

`Cargo.toml` names every dependency down to its patch version, and `Cargo.lock` is committed so that
`cargo install --locked` reproduces a release.

* `clap` with `derive`.
* `proc-macro2` with `span-locations`, so that `Span::start()` returns real lines and columns
  outside a proc macro.
* `quote`, for `ToTokens`, which turns an item back into the tokens whose spans give the lines of an
  `allow-item` item.
* `syn` with `default-features = false` and `full`, `parsing`, `printing` and `visit`. The body
  scanner never clones syntax, so `clone-impls` isn't needed.

There is no `regex`, `walkdir`, `thiserror` or `anyhow`. The error enum is small enough to write by
hand, and `std::fs::read_dir` is enough for the walk.

## Pipeline

`driver::run` performs one run:

1. With `--diff`, it reads the diff (a file, or stdin for `-`) and parses it into per-file
   `LineRanges`.
2. `source::discover` merges the positional targets, the diff's files and the excludes into a sorted
   list of `SourceFile`s, each with a `LineFilter`.
3. For each file, it reads the source. A read failure is a fatal `Error`.
4. It skips a file that `source::is_generated` marks, and counts it separately.
5. It runs `FileChecker::check` on every other file.
6. It sorts and deduplicates the diagnostics of all files.

`FileChecker::check` handles one file:

1. `FileContext::new` lexes the file and splits the suppression comments from the other comments.
2. Each enabled rule checks the file through the context, in `RuleId::ALL` order.
3. `FileContext::finish` adds the lex failure as a `parse-error`, locates `allow-item` items, drops
   the diagnostics that suppressions cover, and adds the `unused-suppression` and
   `invalid-suppression` diagnostics.
4. If the file has a `LineFilter::Ranges`, the diagnostics whose anchor span misses it are dropped.

`main` prints the diagnostics to stdout, then a one-line summary to stderr with the numbers of
diagnostics, checked files and skipped generated files. A fatal `Error` stops the run before
anything is printed to stdout and exits with 2. Otherwise the exit code is 1 when there is at least
one diagnostic, including a meta diagnostic, and 0 when there is none.

## FileContext

`FileContext` is what the rules see of one file. It is built once per file and shared by every
enabled rule:

* `comment_lines()`: the lexed comment lines without the suppression candidates.
* `syntax_tree(rule_id)`: the `syn::File`, parsed on the first call and kept in a `OnceCell`.
* `is_test_module(rule_id)`: whether the file is an [out-of-line test
  module](#out-of-line-test-modules), decided on the first call with the run's `TestModuleCache`.

`syntax_tree` returns `None` in two cases, and the rule then checks nothing:

* The file failed to lex. A lex failure is an unterminated literal or block comment, which `syn`
  can't parse either, and the lex failure is already a `parse-error`.
* `syn::parse_file` fails. The context records `rule_id` as skipped, so only the rules that need the
  tree are skipped.

`is_test_module` records `rule_id` when a candidate declaring file can't be read or parsed, since
the verdict may then be wrong.

Both records become one meta diagnostic each in `finish`, whatever the number of rules that asked:

* `syn failed to parse the file (...); errors-doc, symbol-order are skipped`, at the `syn` error.
* `` can't tell whether this file is a `#[cfg(test)]` module: ...; errors-doc, symbol-order check it
  as non-test code ``, at line 1.

The rules are listed in `RuleId::ALL` order, with `is`/`checks` for one rule. A run with only
`errors-doc` keeps the messages it had before the other rules existed.

A file is parsed at most once, and not at all when no enabled rule needs the tree and no
`allow-item` needs its items. So a `comment-greedy-wrap`-only run parses only the files that have an
`allow-item`.

After the lex failure, the comment rule and the suppressions work on the comments lexed before it.

`finish(rule_diagnostics, rule_set)` consumes the context and completes the file's diagnostics, as
step 3 of the [pipeline](#pipeline) describes.

## Rule registry

Every rule is reached through `src/rules/registry.rs`:

* `RuleId` names a rule for `--rule`, suppressions and diagnostic codes. `RuleId::ALL` lists the
  rules in run order, and `RuleId::as_str` gives each its kebab-case name. `FromStr` and
  `clap::ValueEnum` are derived from these two.
* `Rule` holds an enabled rule's checker, with its settings and the state it keeps across the files
  of a run. `Rule::new` builds it from the `LintConfig`, and `Rule::check` dispatches a
  `FileContext` to it.

The checkers keep no state across files. The cache of [out-of-line test
module](#out-of-line-test-modules) verdicts is shared by every rule, so `FileChecker` owns it and
lends it to each file's `FileContext`. `TestModuleCache` keeps its interior mutability private. The
`comment-greedy-wrap` checker holds the width, and the other checkers hold `include_tests`.

### Adding a rule

1. Add a `RuleId` variant, add it to `RuleId::ALL`, and give it its name in `RuleId::as_str`.
   `--rule`, suppression parsing and the unknown-rule message pick it up from there.
2. Add the rule's module under `src/rules/`. Its `Checker` holds the rule's settings and cross-file
   state, and its `check` takes the `FileContext` and returns the rule's diagnostics.
3. Add a `Rule` variant that holds the checker, with its arms in `Rule::new` and `Rule::check`.
4. Give each diagnostic an anchor span that covers every line whose change can create the finding,
   so that the [diff filter](#line-ranges-and-the-diff-filter) keeps it. Set a fn header if a
   suppression above the item should cover it.
5. Document the rule in the README's rule table and in a page under `docs/rules/`, and add a section
   to this document.

A rule reads the file only through its `FileContext`. When `syntax_tree` returns `None`, the rule
checks nothing; the context reports the `parse-error`. A rule that skips test code by default asks
`is_test_module` unless `--include-tests` is set, and checks nothing for a test module.
Rule-agnostic helpers belong in `src/syntax/` and `src/doc/`, next to `src/lexer.rs` and
`src/markdown.rs`.

## Comment lexer

`src/lexer.rs` is a hand-written, single-pass state machine over the file's `char`s, with one
character of lookahead (two for the char-versus-lifetime decision). It tracks lines and columns, and
builds no token list. `lex` returns a `Lexed`: the comment lines, an optional `LexError`, and the
end of the file header.

States:

* `Code`: the start of the file, and after any literal or comment.
* Line comment: `//` in `Code`, until the end of the line. It records a `CommentLine`.
* Block comment: `/*` in `Code`. A nested `/*` increments the depth, and `*/` decrements it.
* String: `"`, `b"` or `c"` at a token start, until an unescaped `"`. A `\` skips the next char,
  including a newline.
* Raw string: `r`, `br` or `cr`, then `n` times `#`, then `"`, at a token start, until `"` followed
  by exactly `n` times `#`.
* Char: a `'` or `b'` that starts a char literal, until the closing `'`.

Decisions in `Code`:

* Token start: the `r`, `b`, `c`, `br` and `cr` prefixes count only when the preceding char isn't an
  identifier char, so `bar"` and `for"` never start a literal.
* Raw identifier: `r#` followed by an identifier char is a raw identifier (`r#type`) and stays in
  `Code`.
* Char versus lifetime: after `'`, a next char of `\` starts a char literal (`'\n'`), and so does a
  `'` two chars ahead (`'x'`, `'é'`). Anything else is a lifetime or a label (`'a`, `'outer:`).
* Shebang: a first line that starts with `#!` but not `#![` is skipped.

Nothing inside a string, raw string, char or block comment produces a `CommentLine`, so
`"http://x"`, `r#"// x"#` and `/* // */` are never comments. Block doc comments (`/** */`, `/*! */`)
are not extracted.

A `CommentLine` holds:

* `line` and `marker_column`: 1-based, with columns in Unicode scalar values.
* `marker`: `CommentMarker::{Plain, OuterDoc, InnerDoc}`. `////` and longer runs are `Plain`, as
  rustdoc treats them.
* `num_slashes`: the length of the slash run, so that two markers can be compared without storing
  their text.
* `text`: everything after the marker, without a trailing `\r`.
* `is_own_line`: whether only whitespace precedes the marker.
* `width`: the scalar count of the whole source line, with trailing whitespace removed.

`CommentLine::continues_block` decides whether a line continues the previous one's [block](#blocks).

The lexer also tracks the file header in a small `Header` state machine: leading whitespace, non-doc
comments, inner doc comments and inner attributes (with nested brackets). The first outer doc
comment, outer attribute or other token ends the header, and its position is `header_end`, which
[`allow-file`](#suppression-internals) placement is checked against without a parse.

An unterminated string, raw string, char or block comment stops the lexer with a `LexError` at the
token's start. The comment lines before it are kept.

## Markdown classifier

`src/markdown.rs` classifies one line's text after its comment marker and the one space that usually
follows it. Both rules use it, so a doc line plays the same role in `comment-greedy-wrap` as in the
`# Errors` parser:

* `indent`: the count of leading spaces and tabs.
* `atx_header`: an indent of at most 3, then 1 to 6 `#`, then whitespace or the end of the line. A
  closing `#` run isn't part of the title. `#544)` and `#[derive]` are not headers.
* `Fence::parse`: three or more backticks or tildes after any indent. A backtick fence's info string
  can't hold a backtick.
* `FenceTracker::is_code`: whether a line is a fence line or inside a fenced block. A fence closes
  only on a fence line of the same character, at least as long, with no info string.
* `bullet_marker`: `*`, `-`, `+`, or 1 to 9 digits and `.` or `)`, followed by at least one space or
  tab. Its content indent is the indent, plus the marker, plus that whitespace.
* `link_definition`: `[label]:` with an indent of at most 3 and a non-blank label without brackets,
  where the `:` ends the line or is followed by whitespace.

## comment-greedy-wrap

The user-facing rule, with an example of every exemption, is in
[`docs/rules/comment-greedy-wrap.md`](rules/comment-greedy-wrap.md). The `Checker` runs three steps:
`paragraph::blocks` splits the comment lines into blocks, `paragraph::joinable_pairs` finds the line
pairs of each block that are in the same paragraph, and the fit test runs on each pair.

### Fit test

A pair `(L, N)` is a violation when

```text
width(L) + 1 + width(first_word(N)) <= width
```

`width(L)` is `CommentLine::width`, the rtrimmed width of the whole source line (indent, marker and
text). `first_word(N)` is `N`'s body up to the first whitespace after its leading whitespace.

The diagnostic is at `L`'s marker column. Its anchor span is `L` through `N`, since a change to
either line can create the violation.

### Blocks

A block is a maximal run of own-line comments on consecutive lines with the same `marker`,
`num_slashes` and `marker_column` (`CommentLine::continues_block`). Two lines in different blocks
are never in the same paragraph.

A trailing comment is a block of its own, so it is never checked. Suppression candidates are removed
from the comment stream before the rule runs, so a suppression comment leaves a gap in the line
numbers and always ends a block.

### Line kinds

Each line's body is its `text` without one leading space. `Body::kind` classifies it, trying these
kinds in order:

1. `Blank`: empty after trimming.
2. `Fence`: `Fence::parse` matches.
3. `Header`: `atx_header` matches.
4. `TableRow`: starts with `|` after trimming.
5. `BlockQuote`: starts with `>` after trimming.
6. `LinkDef`: `link_definition` matches.
7. `Bullet { content_indent }`: `bullet_marker` matches.
8. `Tagged`: starts with one of `NOTE`, `TODO`, `FIXME`, `SAFETY`, `XXX`, `HACK`, `WARNING` or
   `IMPORTANT`, followed by `:`.
9. `Formula`: a first token of identifier characters, `.`, `:`, `[`, `]`, `(` and `)` that starts
   with a letter or `_`, then one of `=`, `==`, `!=`, `<=`, `>=`, `+=`, `-=`, `*=`, `/=`, `:=`, `->`
   or `=>` as the second whitespace-separated word, then more text.
10. `Text`: anything else, including `<https://...>` autolinks and inline HTML.

### Paragraph machine

`ParagraphMachine` walks one block from top to bottom. It is created per block, so its state never
crosses a block boundary. It keeps:

* `fence_tracker`: the fence that opened the current fenced block, if any.
* `paragraph`: the current paragraph, if any. It is `Prose { continuation_indent }`, `LinkDef {
  def_indent }` or `Preformatted`.
* `list_indents`: the content indents of the list items still open, in increasing order. A list item
  stays open across blank lines, so a loose item's later paragraphs are still prose.

`step` moves the machine to the next line `N` and returns whether `N` continues the previous line's
paragraph. `N` is joinable only when it is a `Text` line and the current paragraph is `Prose` whose
`continuation_indent` equals `N`'s indent. Every step also updates the state:

* A fence line or a line inside a fence ends the paragraph.
* `Blank`, `TableRow` and `BlockQuote` end the paragraph.
* `Header` ends the paragraph and closes every list item.
* `Formula` starts a one-line `Preformatted` paragraph, so it is joined with neither neighbor.
* `LinkDef` starts `LinkDef { def_indent }`. The following `Text` lines indented deeper than
  `def_indent` are its target and are never checked.
* `Bullet` closes the list items deeper than its own indent, pushes its content indent, and starts
  `Prose` at that content indent.
* `Tagged` starts `Prose` at its own indent.
* A `Text` line that doesn't continue the paragraph starts a new one. With indent 0, it starts
  `Prose { continuation_indent: 0 }` and closes every list item. With an indent in `list_indents`,
  it starts `Prose` at that indent and closes the deeper items. With any other indent, it starts
  `Preformatted`.

### Design notes

A pairwise check, which looks only at the two lines of a pair, gives false positives on the y-scope
code bases. The machine's state exists to avoid them:

* A header is never filled, and nothing is pulled up onto it, in either direction. A pairwise check
  flags `/// # Returns` followed by prose.
* Headers are ATX headers with an indent of at most 3 and whitespace after the `#` run. Any line
  starting with `#` would make `// #544).` after `// TODO: ... (see` a header.
* A fence closes only on a matching fence line, so `~~~` inside a backtick fence is code.
* Indented text outside a list item is `Preformatted`, which covers aligned case lists in plain
  comments such as `// index 0: alive EM, terminated -> removed`.
* `Formula` covers pseudo-code written one statement per line, such as `// deadline = now - 900ms`
  in `components/spider-storage/src/task_instance_pool.rs`.
* `Tagged` makes a `// NOTE:` line stacked under another comment start its own paragraph.
* `LinkDef` keeps the long target of an intra-doc link reference definition from being treated as
  prose.

A trailing `:` is deliberately not a boundary, in plain and doc comments alike. A list after a colon
is written as bullets or as indented, preformatted lines (see [Rejected
alternatives](#rejected-alternatives)).

`BlockQuote`, `+` and `N)` bullets and `~~~` fences have no use in the y-scope code bases. They stay
because each is one match arm of CommonMark compatibility and none of them misfires.

## errors-doc

The accepted documentation format, the findings and their messages are in
[`docs/rules/errors-doc.md`](rules/errors-doc.md). This section describes how the rule derives what
a body returns and compares it with what its docstring says.

The model has three parts:

* The [section parser](#parsing-the--errors-section) turns the `# Errors` section into documented
  variants, documented forwards and prose links.
* The [body scan](#body-scan) turns the fn body into obligations (what must be documented), opaque
  sites (where an error leaves the fn unattributed), and weaker facts used only to keep bullets from
  going stale.
* The [comparison](#comparison) pairs the two sides with the [matching
  rules](#symbol-paths-and-matching), and reports missing and stale documentation.

### Stages

`errors_doc::Checker::check` runs these stages on each file:

1. It gets the syntax tree from the `FileContext`, or checks nothing.
2. Unless `--include-tests`, it asks the `FileContext` whether the file is an [out-of-line test
   module](#out-of-line-test-modules). A test module is skipped. A declaring file that can't be read
   or parsed gives a `parse-error` at line 1, and the file is checked unless another candidate marks
   it as a test module.
3. It builds the [`FileIndex`](#file-index) and its qualifier links.
4. `items::collect` walks the file for checkable fns.
5. For each fn, `doc_lines::collect` and `errors_section::parse` find its section. A fn without one
   is skipped.
6. A trait fn without a default body gets only its section's bullet format findings, without the
   intro finding. Its section documents a contract, not a body, and a contract such as
   "Implementations must document their error conditions." has no bullets to introduce.
7. Any other fn's block is scanned by `body::scan`, and `FnCheck::findings` compares the section
   with the body's facts.

Every `errors-doc` diagnostic has the whole fn as its anchor span, from its first attribute or doc
line to its closing brace, and the fn's header as its fn header, from its first attribute or doc
line through its `fn` keyword.

### Item walk

`ItemWalker` implements `syn::visit::Visit` over the file with a stack of `ItemContext`s, each
holding the self names, the generic type parameters, and whether its fns are associated:

* `visit_item_impl` pushes the self type's last segment and the trait's name as self names, with the
  impl's type parameters, and marks the context as associated.
* `visit_item_trait` pushes the trait's name, with its type parameters, as an associated context.
* `visit_item_fn`, `visit_impl_item_fn` and `visit_trait_item_fn` record the fn, then recurse into
  its body in a fresh, non-associated context. So nested fns, impls and modules are checked on their
  own, while the body scanner never enters them.
* Unless `--include-tests`, every item with `#[cfg(test)]` (or a `cfg(all(..))` that requires
  `test`) is skipped without recursion, and so is a fn with an attribute whose last segment is
  `test` (`#[test]`, `#[tokio::test]`).

A fn is collected when its return type is a `Type::Path` whose last segment ends with `Result`
(`Result`, `io::Result`, `DbResult`), which covers `async fn`. A return type of `impl Future<Output
= R>`, with any further bounds, also counts when `R` is such a path. The scanned block is then the
`async` block that the fn's block ends with, and without such a tail the fn has no scanned block.

Each `FnItem` records:

* `block`: the scanned block, or none for a trait fn without a default body.
* `own_error_type`: the last segment of the `Result`'s second generic argument (`CacheError` in
  `Result<Vec<Publication>, CacheError>`), or `anyhow::Error` for `anyhow::Result`.
* `param_names`: the parameters that are plain identifiers.
* `callable_params`: those whose type is a fn pointer, an `impl` or `dyn` type bounded by a trait of
  `CALLABLE_TRAITS`, a type parameter with such a bound (inline or in the `where` clause), or a
  reference, `Box` or `Pin` of one of these.
* `scope`: the `SymbolScope` built from the context's self names and the generic type parameters of
  the context and the fn.
* `is_associated`, used by the [recursion rule](#callees).
* `anchor` and `header`: the line spans of the diagnostics.

### Out-of-line test modules

The `#[cfg(test)]` skip of the item walk only sees inline modules. A file declared in its parent as
`#[cfg(test)] mod tests;` is skipped too, by `syntax/test_code.rs`. A file is a test module when its
declaring file has a top-level `mod name;` item with `#[cfg(test)]`, or when the declaring file is
itself a test module.

The declaring file is the first existing candidate that declares the module:

* For `D/S.rs`: `D/mod.rs`, `D/lib.rs`, `D/main.rs` and `D.rs`, declaring `S`.
* For `D/mod.rs`: the same candidates one directory up, declaring `D`'s last component.
* `lib.rs`, `main.rs` and the files directly under `src/bin/` are crate roots and have no declaring
  file.

Each candidate is read and parsed at most once per run, and each verdict is cached. A missing
candidate is skipped silently. A candidate that exists but can't be read or parsed is skipped too,
and the verdict carries it as a problem. `#[path = "..."]` attributes are not followed.

### Doc lines

`doc_lines::collect` turns every `#[doc = "..."]` attribute with a string literal value into
`DocLine { text, line, column }`, with one leading space removed. A multi-line value is split on
`\n`:

* When the literal spans as many source lines as the value has lines (a `/** */` block, or a literal
  with real line breaks), each doc line gets its own source line.
* Otherwise, such as an escaped `\n` in `#[doc = "a\nb"]`, every doc line gets the literal's line.

Non-literal values (`include_str!`) are skipped.

### Parsing the `# Errors` section

`errors_section::parse` returns an `ErrorsSection`, or `None` when the docstring has no `# Errors`
section.

Section selection and line grouping are rule-agnostic, in `doc/section.rs`:
`section::find(doc_lines, title)` returns a `DocSection` (header position, body line range,
duplicate headers), and `DocSection::content` groups its body into a `SectionContent`. The rules
below hold for any title; `errors-doc` uses `Errors`.

Section selection:

* Headers are ATX headers outside fences. An Errors header's title is exactly `Errors`.
* The section starts at the first Errors header of the smallest level.
* When a header of a smaller level precedes it, the Errors header belongs to that other section, and
  the fn has no `# Errors` section. A `## Errors` under `# Returns` documents the items of a
  returned stream, not the fn.
* A second Errors header of the same level gets a duplicate-section format finding.
* The section ends at the next header of the same or a smaller level. Deeper headers stay in it, and
  fenced blocks inside it are skipped.

Link reference definitions are collected from the whole doc, not only the section, because they
usually sit at the very end. A definition's target may be on the following indented lines.
Definition lines are never bullets or prose.

Inside the section, lines are grouped into prose paragraphs, top-level bullets (at the section's
smallest bullet indent) and sub-bullets (deeper bullets, which belong to the preceding top-level
bullet). A bullet's continuation lines are joined with one space, so a bullet split across lines,
such as `Forwards` followed by a long link on the next line, is parsed as one string.

`doc/links.rs` finds the references in a joined string, in order:

* `` [`code`] ``, `` [`code`](target) `` and `` [`code`][ref] ``: link references whose path is the
  code text.
* `[text](target)` and `[text][ref]` with a non-code label: link references whose path is the
  target, or the definition's target.
* `` `code` `` outside a link: a `ReferenceKind::BareCode` reference. Bare code spans count only in
  forward bullets, where they name closure parameters such as `` `tx` ``.

Each top-level bullet is classified, in this order:

1. `Forward`: starts with the word `Forwards` or `Forward`, and some reference is followed by `'s`
   or `'`. Its paths are every reference before the first possessive, so `` [`a`] and [`b`]'s ``
   gives both.
2. `ForwardList`: starts with `Forwards` or `Forward`, has no possessive reference, ends with `:`,
   and has sub-bullets. Its paths are the first reference of each sub-bullet.
3. `ForwardUnnamed`: any other bullet that starts with `Forwards` or `Forward`. Its paths are every
   reference in it, leniently, and it gets an unnamed-forward format finding that keeps its
   one-segment bare code spans, so the message can suggest a parameter name.
4. `Variant`: starts with a link reference, optionally after `Returns a ` or `Returns an `. Its head
   paths are the link references in its head: the text up to the first ` if `, ` when `, ` for `, `
   unless `, or a `:` that ends the bullet. Its linked paths are the link references after the head
   and in its sub-bullets.
5. `Other`: anything else, including a bullet that starts with a bare code span. Its references join
   the prose links.

Further rules:

* A `Forward` bullet whose wording deviates from the canonical form gets a non-canonical-forward
  format finding, but keeps its paths. The wording check is `is_canonical_forward_wording`, with the
  canonical and mapped forms in `CANONICAL_FORWARD_WORDING` and `MAPPED_FORWARD_WORDING`.
* A forward path whose last segment is UpperCamelCase names a type, not a function. It gets a
  forward-names-type format finding and no stale-forward check, which would only repeat it.
* Sub-bullets of `Forward`, `Variant` and `Other` bullets are conditions, and their references
  document no error. Text after `except` in a `Forward` bullet is ignored too.
* Linked paths name the function a variant comes from, such as the `ensure_met` in ``*
  [`CacheError::StaleState`] if: ... (see [`StateRequirement::ensure_met`])``. The comparison lets
  them document a converted forward.
* The section's first line outside fences and link definitions must start a prose paragraph that is
  exactly `Returns an error if:`, and the section must have a top-level bullet. Otherwise the
  section gets one non-canonical-intro format finding at that line, or at the header when the
  section is empty.

The resulting `ErrorsSection` holds the header position, `variant_bullets` (head paths, linked
paths, position), `forward_bullets` (kind, paths, position), `prose_links`, `has_bullets` and the
format findings.

### Symbol paths and matching

Every name on either side becomes a `SymbolPath`, a list of identifier segments
(`src/syntax/symbol_path.rs`). Normalization applies to doc text and `syn` paths alike:

* Generic arguments are stripped (`Foo::<T>::bar` is `Foo::bar`), and so is a trailing `()` or `!`.
* A leading `::`, `crate`, `self` or `super` is stripped, repeatedly.
* `<T as Trait>::f` becomes `Trait::f`.
* `SymbolScope::normalize` drops a qualifier that is a generic type parameter of the fn or its impl
  or trait, so `T::new` becomes `new`.

The qualifier is the second-to-last segment. `SymbolScope` decides when two paths name the same
symbol:

* Two qualifiers are equal when they are the same identifier, or when both are in the `Self` set:
  `Self`, the impl's self type, the implemented trait in `impl Trait for X`, and the trait's name
  inside a `trait` block.
* Two paths match exactly when their last segments are equal and, if both are qualified, their
  qualifiers are equal. So a method call, which has no qualifier, matches a documented
  `Type::method` by its last segment.
* Two paths match loosely when their last segments are equal, both are qualified, and the file's
  `QualifierLinks` link the qualifiers.
* For mentions only, a documented path also matches any prefix of a body path, so
  `syn::Error::new_spanned` mentions `syn::Error`.

`QualifierLinks` links two different qualifiers when one of them is a trait name, or one is an alias
of the other:

* A trait name is a trait the file declares or implements, or a name that a `use` item imports and
  that the file never names again. A trait imported only to call its methods looks like that.
* An alias is a `use X as Y` rename or a `type Y = X` alias.

`SymbolScope::pair` pairs body facts with documented paths in two passes:

1. Pair every fact and documented path that match exactly.
2. Pair the facts left without an exact match with the documented paths left without one, by loose
   match.

So `ServerConfig::from_yaml_file(..)?` is paired with `` [`YamlConfig::from_yaml_file`] `` when the
file imports `YamlConfig` and never names it again. But `Storage::load()?` documented as ``
[`Cache::load`] `` stays unpaired, which is a missing forward plus a stale one. A fn that calls both
`A::new` and `B::new` and documents both still has each paired with its own bullet.

Each fact and documented path carries a key, and `pair` returns the sets of paired keys on each
side. A fact's key names its obligation and error. A documented path's key names its variant bullet,
its forward bullet and path, or the prose. So the callers never align parallel vectors by index.

### File index

`FileIndex::build` indexes a parsed file once, for every fn in it:

* `helper_fns(name)`: every fn item in the file (free, inherent, trait impl and trait default) with
  that name, with its signature, its body and its owner: the last segment of its impl's self type,
  its trait's name, or none for a free fn.
* `identifier_counts`: how often each identifier appears in the file, including inside macro bodies.
  The imported-only trait rule of `QualifierLinks` uses it.
* `is_called(name)`: whether the file calls `name`, as the last segment of a call path, a method
  name, or an identifier followed by a parenthesized group inside a macro body.
* The trait names, `use` imports and aliases of `qualifier_links()`.
* The `From` conversions between error types, and the enums the file declares.

A conversion comes from either of two declarations:

* `impl From<S> for T` whose `from` fn's tail and `return` expressions all build variants of `T`
  (`Self::Invalid(error)`, `T::Incomplete`, through blocks, `if` and `match`) converts `S` into
  those variants. If some branch isn't a variant of `T`, such as a helper call, the conversion's
  variants are unknown. An impl whose `S` is a type parameter is ignored.
* A variant `V` of an enum `T` with a `#[from]` field of type `S` converts `S` into `T::V`.

Two lookups use them:

* `convert(source_type, target)`: the variants that converting an error of `source_type` into
  `target` builds. Sources match by last segment, and by qualifier when both are qualified, so
  `std::io::Error` matches a `#[from] io::Error`. It is `None` when no conversion matches or the
  match has unknown variants.
* `conversion_variants_into(target)`: every variant that some conversion into `target` builds. It is
  `None` when the file doesn't declare `target` as an enum, or some conversion into it has unknown
  variants.

### Body scan

`BodyScanner` implements `syn::visit::Visit` and walks the scanned block once. `body::scan` returns
its `BodyFacts`. Paths are normalized with the fn's scope, and helpers are resolved through the
`FileIndex`.

#### Scopes and facts

The scanner keeps a `nested_scope_depth` counter. Closures, async blocks, `try` blocks and the
bodies of generator macros increment it, since a `?` inside them belongs to their value, not to the
fn. Nested items aren't entered at all (`visit_item` doesn't recurse).

`BodyFacts` holds:

* `obligations`: what the section must document, each with its position. They are recorded only at
  depth 0.
* `has_opaque_source` and `has_opaque_error`: whether some [opaque site](#opaque-sites) exists at
  depth 0.
* `nested_callees`: the callee of every obligation that would be recorded inside a nested scope,
  such as the `register` of `self.spawn_body(async move { this.register().await? })`. They only keep
  `Forwards` bullets from going stale.
* `mentions`: every expression path, call path, method path (qualified by its builder type, see
  [Query builders](#query-builders)) and struct-literal path, at any depth, plus `into_iter` and
  `next` for every `for` loop and every identifier of an unparsed macro. Patterns are never mentions
  (`visit_pat` doesn't recurse), so an `Err(Error::X) =>` arm doesn't count as returning `Error::X`.
* `conversion_targets`: the types `X` of every `X::from` path, such as `ProtocolError` in
  `.map_err(ProtocolError::from)`.

`BodyFacts::is_infallible` is true when the body has no obligation and no opaque site.

The hooks that drive classification:

* `visit_expr_try` classifies its operand as a `Try` site.
* `visit_expr_return` classifies its value as a `Tail` site, and so is the fn block's value: its
  tail expression, or a brace-delimited macro without `;` at its end (a tail `select! { .. }`).
* `visit_expr_closure` and `visit_expr_async` also classify the closure's value as a tail site,
  which only records nested callees.
* `visit_expr_match` and `visit_expr_if` apply [match-forwarding](#match-forwarding), looking
  through `&&` let-chains.
* `visit_local` records [`let` bindings](#bindings), and `visit_expr_assign` keeps a query builder
  binding across `query = query.bind(..)`.
* `visit_macro` dispatches to [macro handling](#macros).

These hooks run at every depth, so nested scopes record their nested callees, and their obligations
and opaque sites are dropped there.

#### Obligations and spellings

`ObligationKind` has six kinds:

* `Direct { error }`: the body constructs `error` and returns it.
* `Forward { callee }`: the body propagates `callee`'s error unchanged, up to `From`.
* `Either { callee, errors }`: the body maps `callee`'s error into `errors`. The y-scope code
  documents both ways (`.map_err(InternalError::TaskOutputs)?` as a forward, and `.map_err(|error|
  ClientError::Transport(..))?` as the variant), so either is accepted.
* `EitherUnknown { callee, mapper }`: the body maps `callee`'s error with a mapper the scan can't
  resolve. `mapper` describes it for the message.
* `HigherOrder { callee, closure_errors }`: the body passes closures to `callee`, whose result
  leaves the fn, and the closures build and return `closure_errors` (see [Higher-order
  calls](#higher-order-calls)).
* `HelperError { helper, error_type }`: the body returns an error that `helper` builds, and the scan
  can't resolve which. A helper returns an error, not a `Result`, so it can't fail and is never a
  callee: `callee()` is [`None`] for it, as for `Direct`. `error_type` is the fn's own error type
  when the call is converted, or else the return type of the helper fns of the file, if they agree.

`error` and `callee` are `Spellings`: the paths that may document them, the primary one (used in
messages) first, plus the error type of the value when the expression names it.

* A wrapped variant, `Err(CacheError::Internal(InternalError::X))`, is spelled `InternalError::X`
  first and `CacheError::Internal` second, with error type `CacheError`.
* A constructor call `T::ctor(..)`, such as `Error::not_found(id)`, is spelled `T`, then `T::Ctor`,
  then `T::ctor`, with error type `T`. So `` [`serde::de::Error::custom`] `` documents
  `D::Error::custom(..)`.
* A method at the end of a chain, `collection.find(filter).with_options(options).await?`, is spelled
  as the last method first, then the chain's earlier method calls. The call at the chain's root is
  not a spelling, so `SpiderClient::builder(e).connect()?` forwards `connect`, which ``
  [`SpiderClient::builder`] `` doesn't document.
* A call that passes callables is spelled with their callees first.
* anyhow errors have the error type `anyhow::Error`.

Positions:

* `?` obligations and opaque sites are at the `?` token. A multi-line chain then points at the line
  with `.await?`, which is also where a trailing suppression goes.
* `Err(..)` is at the `Err` identifier, and `.ok_or(..)` and `.ok_or_else(..)` at the method name.
* Tail and `return` forwards are at the callee's last path segment or method name.
* Macros are at the macro path's first segment.

#### Bindings

`visit_local` records `let ident = <init>` bindings, a later one of a name replacing an earlier one:

* A closure (`let to_status = |e| ..;`) keeps two summaries: the errors its body classifies to, for
  use as a mapper, and the callee of its value, for a call of the closure.
* A spawn (`let handle = tokio::spawn(..);`) makes a `?` on the awaited binding an `OpaqueError`
  site, as an awaited spawn is.
* A query builder keeps the builder's type.
* An error constructor (`let e = Error::B;`) keeps its errors, so `return Err(e)` returns
  `Error::B`.
* Any other call or method call (`let result = run().await;`), unless the `let` has an `else`, keeps
  its callee, so `result` or `result?` in a return position forwards `run`.

A binding stores only these summaries, computed when the `let` is visited, never syntax.

#### Return positions

An `Err(..)` is a `Direct` obligation only in a return position, where its value leaves the fn:

* The fn's value, a `return` value, and the operand of `?`.
* Inside any of them: a block's tail, an `if` or `else` branch's tail, a `match` arm, a `select!`
  arm body, and an argument of a value method that may return it (`.map_or(Err(..), ..)`,
  `.unwrap_or_else(|| Err(..))`).

So `tx.send(Err(Error::Cancelled))`, `results.push(Err(..))`, `value == Err(..)` and `assert_eq!(r,
Err(..))` record nothing. The same walk classifies the calls in return positions, so one rule,
`classify_fallible(expr, site)`, covers every value that leaves the fn.

#### Error expressions

`classify_error(expr)` gives the errors an expression builds. It first peels parentheses, `.await`,
`.into()`, `Into::into(x)`, `From::from(x)`, `X::from(x)` and `Box::new(x)`. Its result is a set of
errors, plus two flags: an unknown branch (a helper or mapper it can't resolve) and a value branch
(something it can't name as an error, such as a variable).

* A path whose last segment is UpperCamelCase (`Error::Stopped`): that path.
* A call whose callee's last segment is UpperCamelCase (`Error::Io(e)`): the callee, with a wrapped
  variant spelled as above.
* A struct literal (`Error::Invalid { .. }`): its path.
* A call `T::ctor(..)` with an UpperCamelCase `T` that isn't a name of `Self`, and a lower-case
  `ctor` (`Status::internal(..)`): the type `T`.
* `anyhow!(..)`, `anyhow::anyhow!(..)` or `format_err!(..)`: `anyhow::Error`.
* `if`, `match` and blocks: the union of the branches' tails.
* A call of a lower-case fn path (`unsupported_type_error()`, `Self::to_status(..)`), or a method
  call on `self` (resolved as `Self::to_status`): a [helper](#helper-and-pass-through-resolution).
  Unresolved, it is unknown, and so is any other method call or a call of something other than a
  path.
* A variable bound to an error constructor: that binding's errors.
* Anything else (a variable, a field, a literal, `format!`): a value.

An `Err(arg)` in a return position then records:

* `Direct` for each error of `classify_error(arg)`, [converted](#conversions) when `arg` is
  converted or the site is a `?`.
* The errors of a resolved helper (`Err(self.classify_failure(e).await)`) are `Direct` too. The
  helper can't fail, so it is never suggested or accepted as a Forwards target.
* An `OpaqueError` site when `arg` converts a value (`Err(error.into())`), since the conversion
  makes some variant the scan can't name.
* An `OpaqueSource` site for a value branch (`Err(error)`).
* Both opaque sites for an unknown helper, which may also pass on an error it is given. When none
  of the helper's errors resolve, a `HelperError { helper, error_type }` too.
* For each passed parameter of a resolved helper, a `Forward` of the callee whose error the argument
  binds, as described next.

#### Helper and pass-through resolution

Helper resolution is one level deep and same-file only. The helper is a `let` closure binding of
that name in the current body, or else the `helper_fns` with that last segment, narrowed by owner:

* An unqualified path calls a free fn, since an associated fn needs `Self::`.
* A path qualified by a name of `Self` calls a fn of the scanned fn's impl or trait.
* A path qualified by another name calls a fn whose owner is that name.
* When no fn fits, every fn with that name is a candidate.

The result is the union of `classify_error` over the helpers' tail and `return` expressions, where a
nested helper call is unknown instead of being resolved again. So `.map_err(to_transport_error)?`,
where `to_transport_error`'s body is `SchedulerError::Transport(..)`, maps into
`SchedulerError::Transport`.

Before that, `pass_through.rs` checks each outcome leaf of the helper (through blocks, `if` and
`match`, after peeling conversions) for passing on a parameter unchanged. Such a leaf is a passed
parameter of the result instead of a value:

* The parameter itself (`return error;`, `error.into()`), unless a destructuring that rebinds its
  name is in scope there.
* The variant that a top-level `let V(x) = param` (with or without `else`) matched, rebuilt around
  its binding: `DbError::Sql(error)` after `let DbError::Sql(error) = error else { .. };`. Another
  variant around the binding (`DbError::PayloadRejected(error)`) is an error as usual.

When the argument for a passed parameter is the binding of an enclosing `Err(binding)` match arm or
`if let` on a call, the call's callee is forwarded. So `Err(e) => return Err(self.classify(chunk,
e).await)` in `match insert(..).await` forwards `insert`, and `classify`'s other errors are
`Direct`. Any other argument is an `OpaqueSource` site.

#### Values that leave the fn

`classify_fallible(expr, site)` classifies a value at a `Try` or `Tail` site. It peels `.await` and
parentheses, then takes the first case that applies:

1. An awaited `spawn(..)` or `spawn_blocking(..)`: an `OpaqueError` site, since `?` turns its join
   error into some variant of the fn's error type.
2. A conversion (`x.into()`, `Into::into(x)`, `X::from(x)`): classify `x`. A converted value that
   isn't a call is an `OpaqueSource` site, so a tail `r.into()` never suggests forwarding `into`.
3. `Err(e)`: as in [Error expressions](#error-expressions). `Ok(..)` and `Some(..)` give nothing.
4. `.ok_or(e)` and `.ok_or_else(|..| e)`: `Direct` for the errors of the argument or the closure
   body. The receiver is an `Option` and is not forwarded.
5. `.map_err(f)` on a receiver whose callee is `c`, after peeling the receiver's pass-through
   adaptors:
   * A conversion mapper (`Into::into`, `From::from`, `X::from`, or a closure whose body is `e`,
     `e.into()` or `X::from(e)`): `Forward(c)`.
   * A mapper that constructs errors (an UpperCamelCase path such as `Error::Wrap`, a closure whose
     body classifies to errors, or a helper or closure binding that resolves to errors): `Either {
     c, errors }`.
   * An unknown mapper: `EitherUnknown { c }` and an `OpaqueError` site.
   * When the receiver has no callee (a variable), the errors become `Direct`, a conversion is an
     `OpaqueSource` site, and an unknown mapper an `OpaqueError` site.
6. `.context(..)` and `.with_context(..)`: `Either { c, [anyhow::Error] }`. anyhow's `Context` is
   also implemented for `Option`, so documenting `anyhow::Error` alone is accepted. Without a
   callee, it is an `OpaqueSource` site.
7. `.and_then(f)`: classify the receiver and `f`, since both errors leave the fn.
8. `.or_else(f)`: classify `f` only, since it replaces the receiver's error.
9. Pass-through adaptors (`PASS_THROUGH_METHODS`): classify the receiver. A closure argument only
   adds mentions.
10. Sinks, whose errors come from the items of an iterator or stream:
    * `.try_for_each(f)`, `.try_fold(init, f)` and `.try_for_each_concurrent(n, f)`: classify the
      value of `f`.
    * `SINK_METHODS` and `SINK_FNS`: walk the receiver chain back past `ITEM_KEEPING_METHODS` to one
      of `ITEM_PRODUCING_METHODS`, and classify the value of its callable. In this position any
      lower-case path is a fn, so `.map(parse).collect()?` forwards `parse`.
    * When the walk finds no callable: an `OpaqueSource` site.
11. Value methods, which return a value instead of a callee's result:
    * `RECEIVER_VALUE_METHODS` (`expect`, `unwrap`, `clone`, ...) may return the receiver's value:
      an `OpaqueSource` site. The `Result` that `handle.await.expect(..)` returns comes from the
      joined task.
    * `ARGUMENT_VALUE_METHODS` (`map_or`, `unwrap_or_else`, ...) may return an argument: each
      argument, or a closure's body, is classified at the same site.
12. An inner `?`, the outer `?` of `y??`: when the call at the root of `y`'s chain ends in `timeout`
    or `timeout_at`, `Forward` of its future's callee. Otherwise (a `JoinHandle`, a channel reply),
    an `OpaqueSource` site.
13. Blocks, `if` and `match`: classify each branch's value at the same site.
14. A macro: a `select!`'s arm bodies and a `try_join!`'s futures are classified at the same site.
    An anyhow macro gives nothing here, since `visit_macro` records it. Any other macro at a `?`
    site is an `OpaqueSource` site.
15. Any other call or method call: `Forward(callee_of(expr))`, or `HigherOrder` when it has closure
    errors. A call whose callee can't be named, such as `(self.callback)(..)`, is an `OpaqueSource`
    site.
16. A variable with a call binding: `Forward` of the bound callee. A variable with a spawn binding:
    an `OpaqueError` site.
17. Any other variable or field: an `OpaqueSource` site.
18. Anything else: an `OpaqueSource` site at a `?` site, and nothing at a tail site, where `Ok(..)`
    and literals are the success value.

#### Callees

`callee_of` gives a call's callee path (`foo`, `Self::foo`, `module::func`), or a method call's
method path with the chain's earlier method calls as alternative spellings, and then adds the
spellings of callable arguments:

* `block_on` is transparent. For a call whose path ends in `block_on`, or a method call
  `.block_on(x)`, it gives `callee_of(x)`. So
  `runtime.block_on(provider.provide_credentials()).context(..)?` forwards `provide_credentials`.
* A call of a `let` closure binding gives the callee of the closure body's tail. So `fetch(1)?`
  after `let fetch = |key| load(key);` forwards `load`. A closure can't be named in a doc link, so
  the binding itself is never a callee.
* `Ok`, `Some`, `Err`, value methods and calls of anything other than a path have no callee.

A call of the scanned fn itself forwards nothing. It is recursive when its last segment is the fn's
name, and either the path is unqualified and the fn isn't associated, or the qualifier is a name of
`Self`. A method call `self.name(..)` is recursive too. Inside an impl or trait, an unqualified
`parse(x)` can't call the method `parse`, so it forwards the free fn.

#### Higher-order calls

A call or method call whose arguments include callables gets their callees as its first spellings,
so documenting either the helper or the callable documents the call. The callables are:

* Inline closures and async blocks, through their value's callee. So `call_with_retry(cfg, move ||
  async move { client.start_job(req).await })` is spelled `start_job` first and `call_with_retry`
  second.
* Callable paths: a qualified path whose last segment isn't UpperCamelCase (`Self::init`,
  `JobExecutionState::ensure_running`), a free fn of the file that no parameter shadows, a `let`
  closure binding, or a callable parameter of the fn.

The closure errors of the call are the errors that its closures and async blocks build and let leave
them: in their value, their `return` expressions and the operands of their `?` expressions
(`Err(..)`, `.ok_or(..)`, `.ok_or_else(..)` and `.map_err(..)`, but not calls). A forwarded call
with closure errors is a `HigherOrder` obligation instead of a `Forward`. For example, a
`decode_frame(buffer, |reader| ..)` call whose closure returns
`Err(ProtocolError::MalformedFrame(..).into())` is documented by a `ProtocolError::MalformedFrame`
bullet or by forwarding `decode_frame`. The calls inside the closure stay the closure's, as `?` in
closures always does.

#### Conversions

`?` converts an error into the fn's own error type with `From`, and so do `.into()` and `X::from` in
an `Err(..)` argument. When the error's type has a conversion into the own error type in the file
(`FileIndex::convert`), the error is respelled with the conversion's variants first and the old
spellings after them. In a `Result<_, DecodeError>` fn with `impl From<ProtocolError> for
DecodeError { fn from(e: ProtocolError) -> Self { Self::Invalid(e) } }`:

* `Err(ProtocolError::MalformedFrame(..).into())` is `Direct(DecodeError::Invalid)`, also spelled
  `ProtocolError::MalformedFrame`.
* `usize::try_from(n).map_err(|_| ProtocolError::MalformedFrame(..))?` maps `try_from`'s error into
  `DecodeError::Invalid`.

The conversion applies at a `?` site to `Direct`, `Either` and closure errors, and to an `Err(..)`
whose argument is converted. Errors whose type has no such conversion in the file keep their
spellings.

#### Query builders

A method on a `sqlx` query builder is qualified by the builder's type, so that the forward names the
exact function. `BUILDER_FNS` maps the builder fns, called as `sqlx::<fn>` or unqualified, to their
types: `query` and `query_with` build a `Query`, `query_as` and `query_as_with` a `QueryAs`, and
`query_scalar` and `query_scalar_with` a `QueryScalar`. The type passes through
`BUILDER_KEEPING_METHODS` (`bind`, `persistent`) and through `let` and assignment bindings. So
`sqlx::query_scalar(SQL).bind(id).fetch_one(&pool).await?` forwards `QueryScalar::fetch_one`.

#### Match-forwarding

The y-scope code often forwards a callee's error through a binding:

```rust
match self.start_job(spider_job_id).await {
    Ok(_) | Err(ClientError::InvalidJobState(_)) => {}
    Err(error) => return Err(error.into()),
}
```

The rule applies to `match scrutinee { .. }` and to `if let Err(e) = scrutinee`, including an `if
let` in a `&&` let-chain. It classifies the scrutinee as a tail value when all of these hold:

* The peeled scrutinee is a call.
* An arm's pattern, or the let pattern, is `Err(ident)`.
* That arm's body, or the `if` block, passes the binding on unchanged: `Err(ident)`,
  `Err(ident.into())`, `Err(X::from(ident))`, `bail!(ident)`, `Err(anyhow!(ident))`, or `return
  Err(..)` of any of these.

A binding passed on this way records no `Direct(anyhow::Error)` for its `bail!` or `anyhow!`, and no
opaque site for its `Err(..)`. Two related shapes are handled too:

* An arm that wraps the binding in a variant, `Err(Error::Cache(error))` or `return Err(Error::Db {
  source: error })`, maps the callee's error into `Either { callee, errors }`, and the `Err(..)`
  records nothing itself.
* A catch-all arm that applies `?` to its binding, `result => result?`, forwards the scrutinee.

#### Opaque sites

An opaque site is a depth-0 place where an error leaves the fn but the scan can't attribute it. They
never produce a finding. They only relax the [reverse checks](#comparison).

* `OpaqueSource`: the error's source is unknown. Examples are `?` on a variable or field, a tail
  variable without a call binding, an awaited handle, the outer `?` of `x??` (except `timeout`), a
  sink without a callable, a receiver-value method, an unnameable callee, an unknown macro at a `?`
  site, an `Err(..)` of a value, and an unparsable macro with a postfix `?` among its tokens.
* `OpaqueError`: the source is known but the mapping isn't. Examples are an unresolved mapper or
  constructor, a converted value in `Err(..)`, and an awaited spawn or spawn binding.

A postfix `?` in macro tokens is a `?` after an identifier or a parenthesized or bracketed group, so
tracing's `?id` sigil in `tracing::info!(id = ?id)` doesn't count.

#### Macros

`macros::kind` dispatches on the macro path's last segment:

* anyhow: `bail!`, `ensure!`, `anyhow!` and `format_err!`, with or without the `anyhow::` prefix.
  `bail!` and `ensure!` record `Direct(anyhow::Error)` at depth 0, unless match-forwarding applies.
  `anyhow!` and `format_err!` only build a value, so they record nothing outside an `Err(..)` in a
  return position. All of them mention `anyhow::Error`.
* `select!` (`tokio::select!`, `futures::select!`, bare `select!`): parsed with a custom `syn`
  parser for `[biased;] (pat = expr [, if cond] => body [,])* [else => body [,]]`. The futures,
  guards and arm bodies are visited at the current depth, so a `?` in an arm is an obligation of the
  fn. The patterns are ignored. In a return position, the arm bodies are the macro's value.
* `try_join!`: parsed as an expression list. In a return position, each future is classified as a
  call whose error leaves the fn.
* Generators: the bodies of `stream!` and `try_stream!` are visited as a nested scope, like an async
  block.
* Pattern-taking macros: `matches!`, `assert_matches!` and `debug_assert_matches!` are parsed as an
  expression, a comma, and a `syn::Pat` with an optional `if` guard. Only the expression and the
  guard are visited, so `matches!(r, Err(X::Y(_)))` mentions nothing.
* Everything else, including the assert family: the tokens are parsed as a comma-separated
  expression list, and then as a block's statements. If one parse succeeds, the result is visited at
  the current depth. If both fail (`sqlx::query!` with SQL, or `tracing::info!(id = ?id, "..")`),
  the scanner records only mentions, one per identifier in the tokens, and the postfix-`?` opaque
  site.

#### Library API tables

`known_apis.rs` holds every table of library APIs that the scan recognizes by name:
`PASS_THROUGH_METHODS`, `SINK_METHODS`, `SINK_FNS`, `ITEM_PRODUCING_METHODS`,
`ITEM_KEEPING_METHODS`, `RECEIVER_VALUE_METHODS`, `ARGUMENT_VALUE_METHODS`, `BUILDER_FNS`,
`BUILDER_KEEPING_METHODS`, `SPAWN_FNS`, and the `CALLABLE_TRAITS` of the item walk. Every table is
matched by the last segment of a fn path or by a method name only, so a same-named fn of any type is
treated the same way. A new library shape is supported by extending a table, and by a test in
`body.rs` that shows the shape it covers.

### Comparison

`FnCheck::findings` compares the `ErrorsSection` with the `BodyFacts`, using the fn's scope, the
`FileIndex` and the `QualifierLinks`. Obligations are first deduplicated by `ObligationKey`, which
leaves out an `EitherUnknown`'s mapper description, so each key is reported once, at its first site.
All matching uses the two-pass `SymbolScope::pair`, once for errors against variant heads and prose
links, and once for callees against forward paths and variant linked paths.

Forward checks find missing documentation:

* `Direct(e)` is satisfied when a variant head path or a prose link matches `e`.
* `Forward(c)` is satisfied when a path of a `Forward`, `ForwardList` or `ForwardUnnamed` bullet
  matches `c`, or a variant bullet's linked path does. The second form documents a converted forward
  as the variant it becomes, with the function in its condition.
* `Either` and `HigherOrder` are satisfied by `Forward(c)`'s condition, or when every one of their
  errors satisfies `Direct`'s condition.
* `EitherUnknown { c }` is satisfied by `Forward(c)`'s condition, or by any variant bullet or prose
  link at all.
* `HelperError { error_type }` is satisfied by a variant head path or a prose link that names
  `error_type` or one of its variants, or by any of them when `error_type` is unknown. A Forwards
  bullet never satisfies it.

A converted error needs a head that names it. When the file declares the conversion, the converted
variant documents it, and so does the inner error. Otherwise, `Err(DbError::Corrupted.into())` in a
`Result<_, Error>` fn is documented by ``[`Error`] converted from [`DbError::Corrupted`] if ...``,
and not by an unrelated `` [`Error::Corrupted`] `` bullet: nothing in the file links
`Error::Corrupted` to the conversion.

Reverse checks find stale documentation. They apply to bullets only, never to prose links.

* A variant bullet is stale when none of its head paths is paired with an error or matched by a
  mention, and none of its linked paths is paired with a callee. `is_kept_variant` keeps it in these
  cases:
  * A head path equals the fn's own error type, which the signature guarantees.
  * Every head path names an error type rather than a variant (no UpperCamelCase qualifier, such as
    `` [`std::io::Error`] ``), and the body has an `OpaqueSource` site. A `?` on a handle or a
    variable can return such a foreign type through `From`.
  * A head path `X::V` has a conversion target `X`, and `conversion_variants_into(X)` is unknown or
    holds `X::V`. `.map_err(ProtocolError::from)` may build any `#[from]` variant of a
    `ProtocolError` declared in another file.
* A body with an `OpaqueError` site disables the stale-variant check, since the unresolved mapper
  may build any variant.
* A forward path, unless it names a type, is stale when no callee is paired with it and
  `is_stale_forward` holds. A path is kept in these cases:
  * A nested callee matches it, exactly or loosely, such as a `?` inside the async block the body
    hands to `Self::spawn_body`.
  * The body mentions it and has an opaque site, since the call's error may leave through that site,
    as in `let r = Wrapper(foo()); r.0?`.
  * The body has an `OpaqueSource` site and the file calls the path's last segment anywhere
    (`FileIndex::is_called`). The opaque `Result` may come from that call:
    `indexing_task.await.context(..)??` forwards the `run_indexer` of a task that another fn
    spawned.
  * It names a callable parameter of the fn that the body mentions, such as `body` in a fn that
    hands `body` to a spawned task.
* `ForwardUnnamed` bullets get no reverse check.

Delegation merge: when a fn has at least one missing `Forward` and at least one stale variant
bullet, those findings are replaced by one delegation finding at the header. It lists the callees to
forward and the number of stale variants. This turns a public facade that lists the leaf variants of
each delegated call into one finding per fn instead of one per variant. For example,
`components/spider-client/src/client.rs` lists 41 leaf variants across 10 fns that tail-call
`self.job_orchestration.<fn>(..)`. Other findings of the fn are reported as usual.

The intro finding is dropped for a section without bullets in a fn whose body is infallible. Such a
section states that the fn never fails, such as one kept for `clippy::missing_errors_doc`.

Positions: missing findings are at the obligation's position. Stale and format findings are at the
bullet's first doc line and column. Delegation and duplicate-section findings are at the header. The
messages are built in `check.rs` (`missing_finding`, `stale_variant_finding`,
`stale_forward_finding`, `delegation_finding` and `format_message`), and name callees as the body
writes them (`Self::load`, `Type::func` or a bare method name).

## Parameter sections

`type-params-doc` and `trait-params-doc` check sections of the same shape: one top-level bullet per
parameter, ``* `name` - description``. `doc/param_section.rs` holds everything they share, so the
two rules differ only in which items they check and which names those items have.

`ParamSectionKind` names a kind of section by its title and the noun its messages use:
`TYPE_PARAMETERS` (`Type Parameters`, "generic parameter") and `PARAMETERS` (`Parameters`,
"parameter"). The title must match exactly, so `# Type parameters` and `# Arguments` are not
sections of either kind.

`ParamSectionKind::parse(doc_lines)` selects the section with
[`section::find`](#parsing-the--errors-section) and groups it with `DocSection::content`, so
selection, fences, link definitions and continuation lines behave exactly as for `# Errors`. Each
top-level bullet becomes a `ParamBullet`, whose `BulletForm` is one of:

* `Nameless`: the bullet starts with prose.
* `NonCanonical { name }`: the bullet starts with a reference whose code is `name`, either a code
  span (`` `T` ``) or a code link (``[`T`]``), but isn't canonical.
* `Canonical { name }`: the bullet starts with a bare code span, then ` - `, then a non-blank
  description. The description may continue on the next lines.

Sub-bullets belong to their bullet and are never parameters. Prose in the section is ignored.

`ParamSection::compare(required, accepted)` compares the bullets with the item's parameters.
`required` are the names that must be documented, `accepted` are further names a bullet may document
without being required. It reports, in this order:

1. `DuplicateHeader` for each later header with the same title at the same level.
2. Per bullet, in source order: `NamelessBullet` without a name; `UnknownName` for a name in neither
   list; `DuplicateBullet` for a name an earlier bullet has; else `NonCanonicalBullet` if the bullet
   isn't canonical. A non-canonical bullet still documents its name.
3. `Missing` at the header for each required name that no bullet has.

`compare` is only meaningful for an item that has parameters. The rules handle the other two cases
themselves, with the shared messages:

* An item with parameters but no section: `missing_section_message`, at the docstring's first line.
* A section on an item without parameters: `no_parameters_message`, at the header, instead of an
  `UnknownName` per bullet.

Messages, with `T` the title, `N` the noun and `I` the item as `DocItem::description` gives it (for
example ``fn `load` ``):

* `` I has Ns but no `# T` section; add one with a bullet for each of `'a`, `Item` ``
* `` I has no Ns; remove the `# T` section ``
* `` the docstring has a second `# T` section at the same level; merge them ``
* `` this `# T` bullet doesn't start with the N it documents; write "* `name` - description" ``
* `` write this `# T` bullet as "* `name` - description" ``
* `` `# T` documents `x`, which isn't a N of I; remove the bullet ``
* `` `x` is already documented on line 12; remove this bullet ``
* `` `# T` doesn't document `x`; add "* `x` - ..." ``

The order of the bullets isn't checked against the order of the parameters.

`syntax/doc_items.rs` walks the items both rules look at. `doc_items::collect(syntax_tree,
include_tests)` returns a `DocItem` (kind, name, attributes and `ItemLines`) for every fn, struct,
enum, union, trait and type alias, including the ones nested in modules and fn bodies. Its
`DocItemKind` tells free fns, inherent methods, trait impl methods and trait method declarations
apart, and each kind carries the item's signature if it is a fn, or its generics otherwise.
`DocItem::generics` gives the item's own generics either way. Unless `include_tests`, it skips
`#[cfg(test)]` items with everything in them and fns with a `test` attribute, as the `errors-doc`
item walk does. Both rules check only documented items: `doc_lines::collect_documented` gives an
item's doc lines only when at least one of them is non-blank.

## symbol-order

The rule checks the "Symbol ordering" convention of the y-scope Rust `coding-style.md`: items are
grouped by visibility, ordered by kind within a group, and each type is directly followed by its
impl blocks.

### Checked scopes

The rule checks two kinds of scopes, each on its own:

* Modules: the file root and the body of every inline `mod name { .. }`, nested ones included.
  Items in fn bodies and other blocks are not in a module scope and aren't checked.
* Inherent impl blocks (`impl Type { .. }` without a trait) in a checked module, whatever module
  their type is defined in.

Trait impl blocks and trait declarations aren't checked inside. Their item order follows the trait
or rustfmt's `reorder_impl_items`.

Unless `--include-tests`, an out-of-line test module file is skipped, as
`FileContext::is_test_module` decides, and so is every item with `#[cfg(test)]` (or a `cfg(all(..))`
that requires `test`), including whole `mod tests` blocks and inherent impl blocks, and every fn
with a `#[test]`-like attribute, as `doc_items` skips them. With `--include-tests`, those items are
counted like any other, and the body of a `#[cfg(test)] mod tests` is checked as a module.

In a module, these items are exempt: they are neither checked nor counted, so they may sit anywhere,
and the items around them are compared as if they weren't there.

* `use` and `extern crate` items.
* Modules, both `mod x;` declarations and inline modules. An inline module's body is still checked
  as its own scope.
* `macro_rules!` definitions and item-position macro invocations.
* `extern` blocks and items `syn` keeps as verbatim tokens.
* Impl blocks whose self type isn't a struct, enum or union defined in the same module (see below).

In an inherent impl, associated types, macro invocations and verbatim items are exempt.

### Sort keys

Each counted item gets a sort key. The scope is in order when the keys don't decrease.

In a module, the key is `(group, category)`, and the type category carries `(anchor, impl rank)`
(`Category::Type` in `keys.rs`), compared after the category itself:

* `group`: the item's `VisibilityGroup` (`syntax/visibility.rs`): `pub`, `pub(crate)`,
  `pub(super)`, `pub(in path)`, private. `pub(in crate)` counts as `pub(crate)`, `pub(in super)` as
  `pub(super)`, and `pub(self)` as private. An impl block has no visibility of its own and takes
  its type's group.
* `category`: const 0, static 1, type 2, fn 3. Structs, enums, unions, traits, trait aliases and
  type aliases are all types, in any order among themselves, and an impl block has its type's
  category.
* `anchor`: for a type, its index in the module's items; for an impl block, its type's index. Types
  never conflict on it, since their indices increase in source order, and it puts each impl block
  right after its own type.
* `impl rank`: 0 for the type itself, 1 for an inherent impl, 2 for an impl of a crate-local trait,
  3 for an impl of any other trait.

An impl block belongs to a type when its self type, without parentheses, is a path of exactly one
segment (generic arguments allowed) that names a struct, enum or union defined in the same module.
`impl crate::a::Foo`, `impl<T> Trait for T`, `impl Trait for &Foo`, `impl dyn Trait` and impls on
type aliases are exempt. When the module defines the name more than once, as `#[cfg]` variants do,
the impl block belongs to the nearest definition before it, or else to the first one.

A trait is crate-local when `CrateNames::is_crate_path` (`syntax/crate_names.rs`) holds for its
path: the path starts with `crate`, `self` or `super`, or its first segment is a trait or module the
file defines, or a name the file imports with a `use` rooted at `crate`, `self`, `super` or one of
its own modules. A path with a leading `::`, a std or prelude trait (`Display`, `From`, `Clone`),
and a trait of another crate of the same workspace (`spider_core::Task`) are external. A negative
impl (`impl !Send for Foo`) is ranked by its trait like any other.

In an inherent impl, the key is `(category, group)`: an associated const is `(0, pub)` whatever its
visibility, and a fn is `(1, group)`. So consts come before fns, and fns are ordered `pub` first,
private last. Consts aren't ordered among themselves.

### Minimal moves

Reporting every item whose key is smaller than the key of some earlier item would cascade: one
private fn at the top of a module would flag every `pub` item after it. Instead, the rule keeps a
longest non-decreasing subsequence of the keys in place, and flags only the items outside it. Moving
exactly the flagged items, each to the place its diagnostic names, sorts the scope, and no smaller
set of moves does.

Among the longest subsequences, the rule keeps the one whose indices are lexicographically smallest,
so for equally long choices the earlier items stay and the later ones move. With `pub a`, `fn b`,
`pub c`, the kept items are `pub a` and `fn b`, and `pub c` is flagged. With `fn a`, `pub b`, either
item could move, and `pub b` is flagged.

The sequence is found by the quadratic dynamic program: the length `L[i]` of the longest
non-decreasing subsequence that starts at each index, then a greedy walk that starts at the smallest
`i` with the maximum `L[i]` and steps to the smallest later `j` with `key[j] >= key[i]` and `L[j] =
L[i] - 1`. Scopes hold at most a few hundred items, so the quadratic cost is negligible.

A flagged item always conflicts with some kept item: with no larger kept item before it and no
smaller kept item after it, it would extend the kept subsequence. `moves::place` panics in that
impossible case, since it means `moves::kept` is wrong.

### symbol-order diagnostics

One diagnostic per flagged item, at the item's first token after its outer attributes (its
visibility, or its keyword):

```text
<item> should come <after|before> <reference>: <reason>
```

* `<item>` and `<reference>` describe an item by its group, its kind and its name, such as
  ``pub(crate) fn `load` `` or ``private struct `State` ``, and the same way inside an inherent
  impl, such as ``pub const `LIMIT` ``. An impl block is described without a group, as
  ``impl `State` `` or ``impl `Display for State` ``, and a negative impl as
  ``impl `!Send for State` ``. A trait's path segments are joined by `::`, and its generic
  arguments are dropped, as are the self type's.
* `<reference>` is the last kept item whose key is at most the flagged item's key, with `after`. If
  there is none, it is the first kept item, with `before`. So a misplaced impl block is told to come
  after its type or its type's last kept impl.
* `<reason>` compares the flagged item with the kept item it conflicts with: the first kept item
  before it with a larger key, or else the last kept item after it with a smaller key. The first key
  component that differs gives the reason:
  * group: `pub items come before private items`, with the two groups' names.
  * category: `consts come before fns`, with the plurals `consts`, `statics`, `types and their
    impls` and `fns`.
  * anchor, or impl rank against rank 0: `a type's impl blocks directly follow it`.
  * impl rank: `inherent impls come before impls of crate traits`, `impls of crate traits come
    before impls of external traits`, or `inherent impls come before impls of external traits`.
  * In an inherent impl: `associated consts come before fns`, or `pub fns come before private fns`.

Examples:

```text
src/a.rs:12:1: symbol-order: pub fn `run` should come after pub struct `Runner`: pub items come before private items
src/a.rs:30:1: symbol-order: impl `Display for Runner` should come after impl `Runner`: a type's impl blocks directly follow it
src/a.rs:41:5: symbol-order: pub fn `new` should come before private fn `helper`: pub fns come before private fns
```

The anchor span runs from the first line of the earlier of the flagged and the reference item to the
last line of the later one, so a change anywhere between them, such as an inserted item that makes
an old one misplaced, reports it. The fn header is the flagged item's `ItemLines::header`, so an
`allow(symbol-order)` right above the item, or between its doc and its signature, covers it.

## type-params-doc

The rule checks the `# Type Parameters` section that the y-scope Rust `coding-style.md` requires
whenever an item has generic parameters, as the review guide's "`# Type Parameters` is required
when type parameters exist" repeats.

Checked items, from `doc_items::collect`: documented free fns (nested ones too), inherent methods,
trait method declarations (with or without a default body), structs, enums, unions, traits and type
aliases. Not checked:

* Trait impl methods: their docstring is optional, and the trait declaration documents the
  parameters.
* An item without a docstring. Like `errors-doc`, which skips a fn without an `# Errors` section,
  the rule only checks docstrings that exist.
* Test code, unless `--include-tests`, as `doc_items::collect` and `FileContext::is_test_module`
  decide.

An item's generic parameters are its own `Generics::params`, in declaration order:

* A lifetime parameter is named with its tick, `'a`, as Spider writes it in
  `components/spider-tdl/src/ffi.rs`.
* A type parameter and a const generic are named by their identifier.

Not parameters of the item: an impl block's own generics (a method documents only its own), a
trait's generics on its methods, elided and anonymous lifetimes (`&self`, `'_`), `impl Trait`
argument types, and `for<'a>` bound lifetimes.

Checks, with the [shared messages](#parameter-sections) of `TYPE_PARAMETERS`:

* A documented item with at least one generic parameter and no `# Type Parameters` section: the
  missing-section message at the docstring's first line, listing every parameter.
* A section on an item without generic parameters: the no-parameters message at the header.
* Otherwise, `compare(params, [])`: missing, unknown, duplicate, nameless and non-canonical bullets,
  and duplicate headers. A bullet must spell a lifetime with its tick, so `` `a` `` for `'a` is an
  unknown name plus a missing parameter.

Every diagnostic has the item's span as its anchor and the item's header as its fn header.

## trait-params-doc

The rule checks the two `# Parameters` conventions of the y-scope Rust `coding-style.md` and review
guide ("`# Parameters` only in trait declarations"): a trait declaration documents its methods'
parameters, and nothing else has a `# Parameters` section. The heading is `# Parameters`, as the
guides and the existing uses in CLP and Spider write it; `# Arguments` isn't recognized.

Checked items, from `doc_items::collect`, documented, and not test code unless `--include-tests`:

* Trait method declarations, with or without a default body.
* For the reverse check, every other item `doc_items::collect` returns: free fns, inherent methods,
  trait impl methods, structs, enums, unions, traits and type aliases.

A trait method's parameters are its typed inputs, without the receiver (`self`, `&self`, `&mut
self`, `self: Box<Self>`):

* An identifier pattern (`job_id: JobId`, `mut buffer: Vec<u8>`) is a required name. A name with a
  leading underscore, such as `_context`, is required too.
* `_` is skipped.
* A destructuring pattern (`(a, b): (u8, u8)`, `Point { x, y }: Point`) is skipped. The identifiers
  it binds are accepted names: a bullet may document them, but none is required. A binding with a
  subpattern (`point @ Point { .. }`) counts as a destructuring pattern, so `point` is accepted.

Checks, with the [shared messages](#parameter-sections) of `PARAMETERS`:

* A documented trait method with at least one required name and no `# Parameters` section: the
  missing-section message at the docstring's first line, listing the required names.
* A section on a trait method with neither required nor accepted names: the no-parameters message
  at the header.
* Otherwise, `compare(required, accepted)`.
* A `# Parameters` section on any other documented item: at the header, `` `# Parameters` belongs
  only in trait method declarations; remove it from <item> ``. Its bullets aren't checked.

Every diagnostic has the item's span as its anchor and the item's header as its fn header.

## Suppression internals

The suppression forms and their user-facing semantics are in
[`docs/suppression.md`](suppression.md). This section describes `src/suppression.rs`.

`suppression::parse(comment_lines, header_end)` runs inside `FileContext::new`, before any rule:

* A comment line is a candidate when it is a plain `//` comment and its trimmed text starts with
  `yscope-rust-linter:`. Doc comments are never candidates, so a suppression can't leak into
  rustdoc.
* Candidates are removed from the comment stream that `comment_lines()` returns. The removed line
  breaks the run of consecutive lines, so a candidate of any form, valid or not, ends a
  `comment-greedy-wrap` block.
* `Directive::parse` reads the keyword of a `SuppressionKind`, the rule ID list and the reason. An
  unknown keyword, an unknown or meta rule ID, or a missing reason (for every form but `allow-end`)
  is an `InvalidSuppression`.
* A `Parser` processes the candidates in source order, and keeps the open regions.

The result is `ParsedComments`: the `Suppressions`, the remaining comment lines, and the
`invalid-suppression` diagnostics.

Each valid suppression has a `Scope`:

* `Unit { lines }`, for `allow`. A trailing `allow` covers its own line. An own-line `allow` covers
  the next unit, which starts at the first following line that isn't an own-line candidate: the
  whole comment block starting there (`block_last_line`), or else that single line.
* `Item { lines }`, for `allow-item`, once located.
* `Region { start_line, end_line }`, for a matched `allow-start` and `allow-end`. `open_regions`
  opens one region per rule, and `close_regions` closes the open region of each rule it names.
  Regions that opened at the same `allow-start` and close at the same `allow-end` become one
  suppression. An unclosed start, an end without an open region, and a second start of an open rule
  are invalid.
* `File`, for `allow-file` at or before `header_end`. A later one is invalid.

`Suppression::covers` decides whether a suppression covers a diagnostic. Only rule codes that the
suppression names are covered, never meta codes. A `Unit` also covers a diagnostic whose fn header
intersects its lines, so one `allow` above a fn, or between its doc and its signature, covers every
`errors-doc` finding of the fn. A `Region` covers the lines strictly between its ends.

`allow-item` needs the syntax tree, so it is resolved in `FileContext::finish`:

* Parsing records an `UnlocatedItem` with its target line: the first following line that isn't an
  own-line candidate. A trailing `allow-item` is invalid.
* When some item is unlocated, `finish` parses the file and calls `syntax/item_lines.rs`, which
  collects every item with its span and its header, from its first attribute or doc line through the
  line where the item itself starts. Items include nested ones, and the fns, consts and types of
  impls and traits.
* `locate_items` picks the outermost item whose header holds the target line, and the suppression
  covers that item's span. Without one, the suppression is invalid.
* When the file failed to lex or parse, `reject_items` makes each `allow-item` invalid, with a
  message that says why its item can't be located.

`Suppressions::apply` drops every diagnostic that some suppression covers, marking each covering
suppression as used. Then it adds an `unused-suppression` for every unused suppression whose rules
are all enabled in this run, at the suppression's comment (for a region, its `allow-start`). The
invalid-suppression diagnostics are added after `apply`, so they can't be suppressed. The line
filter then runs on everything, so an unused or invalid suppression is reported only when its own
line is in range.

## Diagnostics and output

`src/diagnostic.rs` defines:

* `DiagnosticCode`: `Rule(RuleId)`, or one of the meta codes `InvalidSuppression`,
  `UnusedSuppression` and `ParseError`. Meta codes are always on, aren't selected by `--rule`, and
  can't be suppressed.
* `Position`: a 1-based line and column, with columns in Unicode scalar values. `proc-macro2`
  columns are 0-based scalar counts, so `syntax/span.rs` adds 1.
* `Diagnostic`: a code, a position, a message, an anchor span, and an optional fn header.
  `Diagnostic::new` sets the anchor to the position's line. `with_anchor` and `with_fn_header` widen
  it.

The anchor span is used only by the line filter, and is never printed:

* `comment-greedy-wrap`: the flagged line and the next line.
* `errors-doc`: the whole fn, from its first attribute or doc line to its closing brace. A CI run
  then reports every `errors-doc` finding of any fn it touches, including a body change that makes
  the docstring stale.
* `type-params-doc` and `trait-params-doc`: the whole item, as `item_lines::of` finds it.
* `symbol-order`: the lines from the misplaced item through the item it should be placed next to,
  so a change anywhere between them reports it.
* Meta codes: their own line.

The fn header is used only by `allow` suppressions, as [Suppression
internals](#suppression-internals) describes.

`FileDiagnostic` pairs a diagnostic with its file path, as `source::discover` normalized it.
`sort_and_dedup` sorts by path, line, column, code and message, and removes the duplicates that
would print the same line. `render` prints the text format, or a GitHub workflow command. In the
GitHub format, `%`, `\r` and `\n` are percent-encoded everywhere, and `:` and `,` are also encoded
in the property values.

## Line ranges and the diff filter

Line filters let CI lint only the changed lines.

`src/line_ranges.rs` defines `LineSpan`, an inclusive 1-based span, and `LineRanges`, a set of spans
with `insert`, `union` and `intersects(span)`. `LineRanges` parses `N[-M](,N[-M])*`, where a bare
`N` means `N-N`, and rejects `0`, reversed ranges and overflow.

`source.rs` turns the inputs into `SourceFile`s:

* `TargetSpec::from_str` reads `FILE:RANGES` only when the text after the last `:` has the ranges
  form. Otherwise the whole argument is a path, so a path with a colon still works.
* `source::discover` normalizes every path lexically (dropping `.` components), drops the files
  under an exclude by whole components, and merges the inputs of the same file with
  `LineFilter::union`. A file given once without ranges is `LineFilter::All`.
* A directory is walked in sorted order. Directories named `target`, hidden directories and
  directory symlinks are skipped. A file named explicitly is linted even inside them.
* `is_generated` looks for the word `@generated` in a plain `//` comment among the first five lines.
  This skips prost and tonic output, which says `// This file is @generated by prost-build.`. A doc
  comment that mentions the marker doesn't count.

`diff::parse` turns a unified diff into the changed lines on the new side of each `.rs` file:

* A `+++ b/<path>` line starts a file, and `+++ /dev/null` (a deleted file) starts one whose hunks
  are skipped. A path without the `b/` prefix, or a quoted path (`+++ "b/a b.rs"`), is invalid.
* A hunk header `@@ -a[,b] +c[,d] @@` selects lines of the current file, where `d` defaults to 1.
  With `d > 0`, it selects `c..=c+d-1`. With `d == 0`, a pure deletion after line `c`, it selects
  `max(c, 1)..=c+1`, the lines on both sides of the deleted text.
* A header outside a file, a new side that starts at line 0 with lines (`+0`, `+0,1`), and a number
  that overflows `usize` are invalid, so external input never panics.
* The hunk's content lines are counted against its header, so a content line that looks like a
  header (`+++ x`) stays in its hunk. Context lines of a diff made without `-U0` are selected, since
  the header's range covers them.
* Every other line is ignored.

The pure-deletion rule is why no further widening is needed. A deleted `Forwards` bullet, a deleted
`foo()?;` line, or a deleted line that kept two comment paragraphs apart still selects a line of the
fn or comment it belonged to, and the anchor spans cover the rest.

## Testing

* Unit tests live in each module's `#[cfg(test)] mod tests`. A test module's `//!` doc names only
  the subject under test, never the cases.
* Classifiers are tested by tables, such as one test that classifies a sample body of every
  `LineKind`, including `#544)` and `<https://...>` as `Text`.
* Every `comment-greedy-wrap` exemption has a fixture that isn't flagged and a twin that is. The
  CommonMark-only kinds have only their row in the classification table.
* Every `errors-doc` finding kind has a minimal fixture that produces it, and a satisfied version
  that is clean. Each body-scan case and each keep rule of the reverse checks has its own test.
* `driver.rs` tests the interactions of the pipeline: a lex failure, a `syn` failure in a run with
  and without `errors-doc`, the anchor spans against line filters, suppressions ending paragraphs,
  and the lazy parse for `allow-item`.
* Every `symbol-order`, `type-params-doc` and `trait-params-doc` check and exemption has a fixture
  that produces it or is clean, in the rule's own unit tests. `doc/section.rs`,
  `doc/param_section.rs`, `syntax/visibility.rs`, `syntax/crate_names.rs` and `syntax/doc_items.rs`
  have their own unit tests.
* `tests/cli.rs` runs the built binary (`CARGO_BIN_EXE_yscope-rust-linter`) on files each test
  writes under `CARGO_TARGET_TMPDIR`. It covers the output format and order, exit codes, the
  directory walk, `@generated` skipping, `--rule`, `--exclude`, `--diff -`, `--format github` and
  `--version`.
* The repository lints itself with `yscope-rust-linter src tests`, which must be clean.

Before a release, run the tool on current y-scope/clp and y-scope/spider and triage every finding as
a true positive, a false positive (fix the tool, or add it to [Known limits](#known-limits)), or a
convention question. Seeding known misses (delete a `Forwards` bullet, add a stale variant bullet,
under-fill a bullet continuation) and checking that each is caught, both with a full run and with
`--diff` on the seeding change, verifies the diff filter end to end.

## Known limits

Comments:

* Block doc comments (`/** */`, `/*! */`) aren't checked by `comment-greedy-wrap`. Comments inside
  `macro_rules!` bodies are lexed and checked like any others.
* Columns count Unicode scalar values, and a tab counts as 1. rustfmt-formatted code has no tabs in
  comments.
* Commented-out code or pseudo-code without a formula operator (`// return x;` followed by `// }`)
  may be flagged.
* A hanging-indent continuation (`// total = a + b +` followed by `// c + d`) starts a preformatted
  paragraph, so the pair isn't checked. Telling a hanging indent from an aligned list or diagram
  needs more than syntax.

What `errors-doc` checks:

* A file that `syn` can't parse gets no `errors-doc` check, only a `parse-error`.
* Return-type aliases not ending in `Result` (`type Fallible<T> = ...`) aren't recognized, so the fn
  isn't checked.
* An `impl Future<Output = ..>` whose `Output` doesn't end in `Result` or whose body doesn't end
  with an `async` block, and `Pin<Box<dyn Future>>` and `Poll<Result>` returns, aren't scanned. For
  example, `components/spider-storage/src/cache/tree/control_block.rs` returns an `Output` alias.
* Out-of-line test modules declared with `#[path]`, or inside an inline module of the parent, are
  checked as non-test code.
* `?` inside `assert!` arguments counts normally. It is rare outside tests, which are skipped by
  default.
* The section's wording isn't read, so a section that states the fn never fails gets the intro
  finding when its body has an error source.

Errors the scan can't attribute:

* Macros whose tokens parse neither as expressions, statements nor the `select!` grammar
  (`sqlx::query!` with SQL) hide their `?` and `Err(..)`. Their identifiers are mentions, and a
  postfix `?` is an opaque source, so a `Forwards` bullet naming a call inside is kept.
* Errors flowing through variables other than call bindings (`Err(error)` outside match-forwarding,
  `self.reply.await?`, `x??` through a `JoinHandle`, `commit?` on a tuple binding, a variable
  reassigned after its `let`) are opaque sources, not obligations.
* An `Err(..)` bound with `let` and returned later, or the `break Err(..)` value of a loop, isn't in
  a return position, so it records nothing.
* `?` in closures and async blocks records no obligation, by design. Their callees are nested
  callees, which only keep `Forwards` bullets.
* A helper that handles the errors of the closure it is given (`retry(|| self.send())`) is
  documented by the closure's callee too.
* Mappers and constructors defined in another file, or more than one helper level deep, are
  `EitherUnknown` with an `OpaqueError` site, which disables the fn's stale-variant check.
* A helper that passes on the error it is given in a shape other than the two pass-through shapes
  (`match error { DbError::Sql(e) => .., other => other }`) makes the doc forward the helper or list
  the variant.
* `From` conversions declared in another file aren't resolved. A variant converted into a wrapper
  enum by `.into()` or `?` is the inner variant, and a forwarded call's converted error is a
  `Forward` of the callee.

Matching:

* Name collisions (two different `send` methods) let one bullet satisfy both.
* Helpers of the same name in several impls of the file are narrowed by owner. When nothing fits,
  every fn of that name is merged.
* Renamed imports and aliases from another file, and a trait method called through a type whose
  trait the file names more than once, give a missing and stale pair.
* Only the `sqlx` builder fns of `BUILDER_FNS` are known, so a method on another builder matches by
  last segment only.
* A callable argument named by an unqualified path that the file doesn't declare (`execute(cfg,
  imported_fn)`) isn't a spelling of the call, except in a sink's callable position.
* A stale `Forwards` bullet is kept in a fn with an `OpaqueSource` site when another fn of the file
  calls its function. A function called only inside a task spawned in another file is stale, which
  is a false positive.
* A `Forwards` bullet whose call's error is handled (`if let Err(e) = foo() { log }`) isn't stale in
  a fn with an opaque site.

What `symbol-order` checks:

* Crate-local traits are recognized by syntax only. A trait imported through a glob (`use
  super::*`), re-exported under another name in another file, or reached through a `use` that names
  a crate-local module of another file is external.
* `use` and inline modules are scoped file-wide, not per module, when `CrateNames` collects
  crate-local names.
* An impl block whose self type is a qualified path, a type alias or a generic parameter is exempt,
  even if it is placed far from its type.
* Lexicographically smallest kept items may flag an old item rather than the one a change inserted:
  inserting a private fn between two `pub` fns flags the later `pub` fn. The anchor span still
  covers the inserted lines.
* Items generated by macros are invisible, and macro invocations are exempt.

What `type-params-doc` and `trait-params-doc` check:

* Sections are recognized by their exact title, so a misspelled title (`# Type parameters`, `#
  Arguments`) is neither checked nor reported.
* A docstring built only from `#[doc = include_str!(..)]` counts as undocumented.
* Bullet order isn't checked.

Input:

* Quoted paths in `--diff` input (names with spaces or non-ASCII bytes) are an invalid argument.

## Rejected alternatives

* Keeping a trailing-`:` boundary, everywhere or in plain `//` comments only. The boundary would
  hide real misses, such as `// There should be 3 submissions:` followed by a short sentence in
  `components/log-ingestor/tests/test_compression_listener.rs`. The only false positive it prevented
  is a formula line after a colon, which `Formula` exempts on its own.
* Keeping an `Html` line kind and a hard-break `\` boundary. `Html` misfired on `<https://...>`
  autolinks, and the only `\` line ending in the y-scope code is in an already preformatted ASCII
  tree.
* Dropping the `#[cfg(test)]` skip and `--include-tests`. Test fns and test modules are skipped by
  default, and the inconsistency between inline and out-of-line modules is fixed by the parent-file
  lookup instead. Integration test crates under `tests/` are neither, so their helpers stay checked,
  which is why pattern-taking macros are handled.
* Dropping `ForwardList`. It has a real use in
  `components/log-ingestor/src/ingestion_job/sqs_listener.rs`, which would otherwise be an
  unnamed-forward false positive.
* Treating a tail call as not forwarding. A tail call returns the callee's `Result`, so its errors
  leave the fn as they do with `?`. The false positives of tail calls such as
  `OnceCell::get_or_try_init(Self::init)` came from the callable argument, which now documents the
  call.
* Requiring the calls inside a closure passed to a helper. `?` in a closure belongs to the closure.
  The closure's built errors document the helper call instead, and its calls keep `Forwards` bullets
  as nested callees.
* Resolving `From` conversions across files. It needs module resolution, which is a non-goal.
  Same-file conversions cover the false positives seen, and a variant bullet can name the converted
  callee in its condition.
* Skipping the stale-variant check whenever the body has any opaque site. A `?` on a variable is
  common and says nothing about which variants the fn builds. Only an unresolved mapper
  (`OpaqueError`) disables the check, and an `OpaqueSource` keeps only the bullets whose heads name
  error types.
* Pairing a converted foreign error with any unmatched own-type variant bullet. It accepted any
  unrelated `Error::*` bullet and kept every such bullet from going stale, so one conversion
  silenced the section.
* Pairing any two qualified paths with the same last segment in the loose pass. It accepted a
  `Forwards` bullet that names the wrong function, such as `` [`Cache::load`] `` for
  `Storage::load()?`. The loose pass needs a trait or an alias to link the qualifiers.
* Checking more `impl Future` shapes. `Output` is often an alias that doesn't end in `Result`, so
  the gain is small.
* A `--targets-from` input beside `--diff`. `--diff` covers CI, including pure deletions, and
  `FILE:RANGES` targets cover ad hoc use.

## Open questions

* `_`-prefixed trait method parameters. `trait-params-doc` requires a bullet for `_context` as for
  any named parameter. If CLP and Spider treat such parameters as undocumented by convention, they
  could join `_` in being skipped.
* Non-canonical parameter bullets. A few existing sections write ``* [`TypeMarker`]: ...``. The
  param-doc rules report them; if that is too strict, `NonCanonicalBullet` could be dropped while
  the name still counts.

* Public facades that list leaf variants. `components/spider-client/src/client.rs` documents the
  leaf `ClientError` variants of each delegated call instead of forwarding the call. The convention
  says to forward helpers instead of re-listing every leaf error, so the tool reports one delegation
  finding per fn. If public client APIs may flatten leaf variants, the merge could be skipped for
  `pub` fns whose body is a single tail call.
* A `NOTE:` paragraph followed by an unrelated sentence.
  `components/spider-storage/src/cache/job.rs` has a `// NOTE:` paragraph followed, with no blank
  line, by a sentence about `AcqRel`. The tool treats them as one paragraph and flags the short
  line. The fix in the code is a blank `//` line between them. It is open whether that is the
  intended convention, or whether a tagged paragraph should end before such a sentence.
