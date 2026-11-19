# Suppression comments

A suppression comment silences the findings of one or more rules in part of a file. Use one when a
rule is wrong for that code, and say why in the reason.

## Contents

* [Syntax](#syntax)
* [`allow`: a line, a comment block or a fn](#allow-a-line-a-comment-block-or-a-fn)
* [`allow-item`: a whole item](#allow-item-a-whole-item)
* [`allow-start` and `allow-end`: a region](#allow-start-and-allow-end-a-region)
* [`allow-file`: the whole file](#allow-file-the-whole-file)
* [Invalid suppressions](#invalid-suppressions)
* [Unused suppressions](#unused-suppressions)
  * [Deprecated rules](#deprecated-rules)
* [Meta codes](#meta-codes)
* [Suppressions with `--diff` and `FILE:RANGES`](#suppressions-with---diff-and-fileranges)

## Syntax

There are five forms:

```text
// yscope-rust-linter: allow(<rule-id>[, <rule-id>]...): <reason>
// yscope-rust-linter: allow-item(<rule-id>[, <rule-id>]...): <reason>
// yscope-rust-linter: allow-start(<rule-id>[, <rule-id>]...): <reason>
// yscope-rust-linter: allow-end(<rule-id>[, <rule-id>]...)
// yscope-rust-linter: allow-file(<rule-id>[, <rule-id>]...): <reason>
```

* A suppression is a plain `//` comment whose text starts with `yscope-rust-linter:`. A `///` or
  `//!` comment is never a suppression, so a suppression can't leak into rustdoc.
* Each `<rule-id>` is a rule name, such as `type-params-doc`, its code, such as `DOC003`, or a set
  prefix, `DOC` or `ORG`, which names every rule of the set. The [rule index](rules/README.md) lists
  the names and codes. A suppression may name several, separated by commas.
* Names are lowercase, and codes and set prefixes uppercase. A lowercase code such as `doc003` is an
  unknown rule.
* Every form but `allow-end` needs a reason after `):`. The reason must not be blank. `allow-end`
  may have one too.
* Spaces after `yscope-rust-linter:`, around each rule ID and before the `:` of the reason are
  optional. There must be no space between the form and its `(`.
* A suppression comment is never part of a `comment-greedy-wrap` paragraph. It also ends the comment
  block it is in.

A good reason says why the rule is wrong here, not what the rule checks:

```text
// yscope-rust-linter: allow(comment-greedy-wrap): a hand-aligned frame layout
// yscope-rust-linter: allow(errors-doc): the errors come from the callback the caller provides
```

A code or a set works the same as the names it stands for:

```text
// yscope-rust-linter: allow(DOC003): the parameters are documented on the trait
// yscope-rust-linter: allow-file(DOC): the bindings are generated
```

## `allow`: a line, a comment block or a fn

What an `allow` covers depends on where it is:

* A trailing `allow`, after code on its line, covers that line.
* An own-line `allow` covers the next unit. The unit starts at the first line after it that isn't
  another own-line suppression:
  * If that line is an own-line comment, the unit is the whole comment block that starts there.
  * Otherwise, the unit is that one line.
* Stacked own-line suppressions all cover the same unit.
* For `errors-doc`, a covered line anywhere in a fn's header covers every finding of the fn. The
  header runs from the fn's first attribute or doc line through its `fn` line.

An own-line `allow` above a comment block covers the whole block:

```rust
fn f() {
    // yscope-rust-linter: allow(comment-greedy-wrap): a hand-aligned frame layout
    // Layout of a frame:
    // magic   4 bytes
    // length  4 bytes
}
```

Without the suppression, the layout is flagged:

```rust
fn f() {
    // Layout of a frame:
    // magic   4 bytes
    // length  4 bytes
}
```

```console
$ yscope-rust-linter example.rs
example.rs:2:5: DOC001 comment-greedy-wrap: line is 25 columns wide and the next line's first word `magic` (5 columns) fits within 100; move it up
example.rs:3:5: DOC001 comment-greedy-wrap: line is 22 columns wide and the next line's first word `length` (6 columns) fits within 100; move it up
```

An own-line `allow` above a code line covers that line:

```rust
/// Runs the job.
///
/// # Errors
///
/// Returns an error if:
///
/// * Forwards [`Store::insert`]'s return values on failure.
fn run(&self, job: Job) -> Result<(), Error> {
    self.store.insert(&job)?;
    // yscope-rust-linter: allow(errors-doc): the audit log documents its own errors
    self.audit.record(&job)?;
    Ok(())
}
```

A trailing `allow` covers only its own line. An `errors-doc` finding for a `?` is at the `?`, so on
a multi-line chain the suppression goes on the line with the `?`:

```rust
/// Runs the job.
///
/// # Errors
///
/// Returns an error if:
///
/// * Forwards [`Store::insert`]'s return values on failure.
fn run(&self, job: Job) -> Result<(), Error> {
    self.store.insert(&job)?;
    self.audit
        .record(&job)
        .await?; // yscope-rust-linter: allow(errors-doc): the audit log documents its own errors
    Ok(())
}
```

An `allow` above a fn's docstring, or between the docstring and the `fn` line, covers every
`errors-doc` finding of the fn:

```rust
/// Runs the callback.
///
/// # Errors
///
/// Returns an error if:
///
/// * Forwards the callback's return values on failure.
// yscope-rust-linter: allow(errors-doc): the errors come from the callback the caller provides
fn run(callback: impl Fn() -> Result<(), Error>) -> Result<(), Error> {
    callback()?;
    load()
}
```

The next unit starts right after the suppression. So an `allow` followed by an empty line covers
only that empty line, and the comment below it is still checked:

```rust
fn f() {
    // yscope-rust-linter: allow(comment-greedy-wrap): a diagram follows

    // a
    // b
}
```

```console
$ yscope-rust-linter example.rs
example.rs:2:5: LNT003 unused-suppression: this suppression of comment-greedy-wrap covers no finding on line 3; remove it
example.rs:4:5: DOC001 comment-greedy-wrap: line is 8 columns wide and the next line's first word `b` (1 column) fits within 100; move it up
```

## `allow-item`: a whole item

An `allow-item` covers an item from its first attribute or doc line through its last line, including
every comment inside it.

* It must be an own-line comment.
* It goes directly above the item, or among the item's attributes and doc comments.
* Items are fns, impls, traits, inline modules, structs, enums, unions, consts, statics, type
  aliases and `macro_rules!` definitions. The fns, consts and types of impls and traits are items
  too.
* The outermost item that starts there is covered. So an `allow-item` above an impl covers the whole
  impl, and one above a method covers only that method.
* The file must parse, since the item is located in its syntax tree.

```rust
// yscope-rust-linter: allow-item(comment-greedy-wrap): legacy comments, reflowed in a follow-up
impl Runner {
    /// Starts the runner and
    /// waits.
    fn start(&self) {
        // A comment that ends
        // early.
    }
}

impl Other {
    /// Not covered, so this
    /// is flagged.
    fn other(&self) {}
}
```

```console
$ yscope-rust-linter example.rs
example.rs:12:5: DOC001 comment-greedy-wrap: line is 28 columns wide and the next line's first word `is` (2 columns) fits within 100; move it up
```

Among the attributes and doc comments of a fn:

```rust
/// Runs the job.
///
/// # Errors
///
/// Returns an error if:
///
/// * Forwards [`Store::insert`]'s return values on failure.
// yscope-rust-linter: allow-item(errors-doc): the audit log documents its own errors
#[inline]
fn run(&self, job: Job) -> Result<(), Error> {
    self.store.insert(&job)?;
    self.audit.record(&job)?;
    Ok(())
}
```

Above one method of an impl:

```rust
impl Runner {
    // yscope-rust-linter: allow-item(comment-greedy-wrap): kept as in the upstream source
    /// Starts the runner and
    /// waits.
    fn start(&self) {}

    /// Stops the runner and
    /// waits.
    fn stop(&self) {}
}
```

```console
$ yscope-rust-linter example.rs
example.rs:7:5: DOC001 comment-greedy-wrap: line is 28 columns wide and the next line's first word `waits.` (6 columns) fits within 100; move it up
```

## `allow-start` and `allow-end`: a region

A region covers the lines strictly between an `allow-start` and the next `allow-end` that names the
same rule.

* Regions are matched per rule. Each rule of an `allow-start` opens a region, and the next
  `allow-end` that names the rule closes it.
* An `allow-end` needs no reason.
* Regions of different rules may overlap.

```rust
// yscope-rust-linter: allow-start(comment-greedy-wrap): copied verbatim from the protocol spec
// The frame starts with a
// header.
fn a() {}
// The header holds the
// length.
fn b() {}
// yscope-rust-linter: allow-end(comment-greedy-wrap)
// This comment is after the
// region.
fn c() {}
```

```console
$ yscope-rust-linter example.rs
example.rs:9:1: DOC001 comment-greedy-wrap: line is 28 columns wide and the next line's first word `region.` (7 columns) fits within 100; move it up
```

One `allow-start` can open regions for both rules, which separate `allow-end`s close:

```rust
// yscope-rust-linter: allow-start(errors-doc, comment-greedy-wrap): generated by the build script
// A comment that ends
// early.
// yscope-rust-linter: allow-end(comment-greedy-wrap)
// Another comment that ends
// early.
/// Loads.
///
/// # Errors
///
/// Returns an error if:
///
/// * Forwards [`open`]'s return values on failure.
fn load() -> Result<(), Error> {
    read()
}
// yscope-rust-linter: allow-end(errors-doc)
```

```console
$ yscope-rust-linter example.rs
example.rs:5:1: DOC001 comment-greedy-wrap: line is 28 columns wide and the next line's first word `early.` (6 columns) fits within 100; move it up
```

## `allow-file`: the whole file

An `allow-file` covers the whole file. It must be in the file header, before the first item. Only
these may come before it:

* Blank lines and plain comments, including block comments.
* Inner doc comments, `//!` and `/*! */`.
* Inner attributes, such as `#![allow(clippy::all)]`.

An outer doc comment, `///`, or an outer attribute, `#[..]`, belongs to the first item, so it ends
the header.

```rust
//! Bindings generated from the protocol schema.
#![allow(clippy::all)]
// yscope-rust-linter: allow-file(comment-greedy-wrap): the bindings are generated

/// A comment that ends
/// early.
fn f() {}
```

## Invalid suppressions

A plain comment that starts with `yscope-rust-linter:` but isn't a valid suppression is an
`invalid-suppression`. It suppresses nothing.

```rust
fn f() {}
// yscope-rust-linter: allow(errors-doc)
// yscope-rust-linter: allow(errors-doc):
// yscope-rust-linter: allow(): reason
// yscope-rust-linter: allow(errors-doc,): reason
// yscope-rust-linter: allow-block(errors-doc): reason
// yscope-rust-linter: allow(errors_doc): reason
// yscope-rust-linter: allow(parse-error): reason
// yscope-rust-linter: allow-end(errors-doc) reason
```

```console
$ yscope-rust-linter example.rs
example.rs:2:1: LNT002 invalid-suppression: malformed suppression; expected `// yscope-rust-linter: allow(<rule-id>[, <rule-id>]...): <reason>`
example.rs:3:1: LNT002 invalid-suppression: suppression needs a reason after `):`; expected `// yscope-rust-linter: allow(<rule-id>[, <rule-id>]...): <reason>`
example.rs:4:1: LNT002 invalid-suppression: suppression names no rule; expected `// yscope-rust-linter: allow(<rule-id>[, <rule-id>]...): <reason>`
example.rs:5:1: LNT002 invalid-suppression: malformed suppression; expected `// yscope-rust-linter: allow(<rule-id>[, <rule-id>]...): <reason>`
example.rs:6:1: LNT002 invalid-suppression: unknown suppression form `allow-block`; expected one of `allow`, `allow-item`, `allow-start`, `allow-end`, `allow-file`
example.rs:7:1: LNT002 invalid-suppression: unknown rule `errors_doc`; expected a rule name or code, one of comment-greedy-wrap (DOC001), errors-doc (DOC002), symbol-order (ORG001), trait-params-doc (DOC004), type-params-doc (DOC003), or a set, one of DOC, ORG
example.rs:8:1: LNT002 invalid-suppression: `parse-error` is not a rule and can't be suppressed
example.rs:9:1: LNT002 invalid-suppression: malformed suppression; expected `// yscope-rust-linter: allow-end(<rule-id>[, <rule-id>]...)`
```

A suppression in the wrong place is invalid too:

```rust
fn f() {} // yscope-rust-linter: allow-item(errors-doc): reason
// yscope-rust-linter: allow-item(errors-doc): reason

fn g() {}
fn h() {
    // yscope-rust-linter: allow-item(errors-doc): reason
    let a = 1;
}
// yscope-rust-linter: allow-file(errors-doc): reason
```

```console
$ yscope-rust-linter example.rs
example.rs:1:11: LNT002 invalid-suppression: an `allow-item` suppression can't follow code on its line; put it on its own line directly above an item, or among the item's attributes and doc comments
example.rs:2:1: LNT002 invalid-suppression: no item directly follows this `allow-item` suppression; put it on its own line directly above an item, or among the item's attributes and doc comments
example.rs:6:5: LNT002 invalid-suppression: no item directly follows this `allow-item` suppression; put it on its own line directly above an item, or among the item's attributes and doc comments
example.rs:9:1: LNT002 invalid-suppression: an `allow-file` suppression must be in the file header, before the first item and its attributes and doc comments
```

So are unmatched regions:

* An `allow-start` that no `allow-end` closes is reported at the `allow-start`.
* An `allow-end` that names a rule without an open region is reported at the `allow-end`.
* A second `allow-start` for a rule whose region is still open is reported. The first region stays
  open.

```rust
// yscope-rust-linter: allow-start(comment-greedy-wrap): reason
// yscope-rust-linter: allow-start(comment-greedy-wrap): reason
// A comment that ends
// early.
// yscope-rust-linter: allow-end(comment-greedy-wrap)
// yscope-rust-linter: allow-end(comment-greedy-wrap)
// yscope-rust-linter: allow-start(errors-doc): reason
fn g() {}
```

```console
$ yscope-rust-linter example.rs
example.rs:2:1: LNT002 invalid-suppression: `comment-greedy-wrap` already has a region open from line 1; end it with `allow-end` first
example.rs:6:1: LNT002 invalid-suppression: this `allow-end` of comment-greedy-wrap closes no open region; open it with `allow-start` first
example.rs:7:1: LNT002 invalid-suppression: this `allow-start` of errors-doc has no matching `allow-end`; close it with `// yscope-rust-linter: allow-end(errors-doc)`
```

When a file fails to lex or parse, its items can't be located, so every `allow-item` in it is
invalid:

```rust
// yscope-rust-linter: allow-item(errors-doc): reason
fn f( {}
```

```console
$ yscope-rust-linter example.rs
example.rs:1:1: LNT002 invalid-suppression: can't locate the item of this `allow-item` suppression: syn failed to parse the file (cannot parse string into token stream)
example.rs:2:5: LNT001 parse-error: syn failed to parse the file (cannot parse string into token stream); errors-doc, symbol-order, trait-params-doc, type-params-doc are skipped
```

A `///` or `//!` comment that looks like a suppression is a doc line, not an invalid suppression. It
suppresses nothing, and `comment-greedy-wrap` checks it as prose.

## Unused suppressions

A valid suppression that covers no finding is an `unused-suppression`. Remove it, or move it to the
code it was meant for.

```rust
// yscope-rust-linter: allow-file(errors-doc): reason
// yscope-rust-linter: allow(comment-greedy-wrap): reason
// A comment that is already filled.
// yscope-rust-linter: allow-item(comment-greedy-wrap): reason
fn f() {
}
// yscope-rust-linter: allow-start(comment-greedy-wrap): reason
fn g() {}
// yscope-rust-linter: allow-end(comment-greedy-wrap)
```

```console
$ yscope-rust-linter example.rs
example.rs:1:1: LNT003 unused-suppression: this `allow-file` suppression of errors-doc covers no finding in the file; remove it
example.rs:2:1: LNT003 unused-suppression: this suppression of comment-greedy-wrap covers no finding on line 3; remove it
example.rs:4:1: LNT003 unused-suppression: this `allow-item` suppression of comment-greedy-wrap covers no finding in its item on lines 5-6; remove it
example.rs:7:1: LNT003 unused-suppression: this region suppression of comment-greedy-wrap covers no finding between lines 7 and 9; remove it
```

* A region is reported at its `allow-start`.
* A suppression is reported as unused only when every rule it names is enabled. So the run below,
  with only `comment-greedy-wrap` enabled, doesn't report the `allow-file(errors-doc)`.
* A suppression that names two rules is used when it covers a finding of either.
* A set prefix names every rule of the set. So `allow(DOC)` is reported as unused only when every
  `DOC` rule is enabled.

```rust
// yscope-rust-linter: allow-file(errors-doc): reason
// yscope-rust-linter: allow(comment-greedy-wrap): reason
// A comment that is already filled.
// yscope-rust-linter: allow-item(comment-greedy-wrap): reason
fn f() {
}
// yscope-rust-linter: allow-start(comment-greedy-wrap): reason
fn g() {}
// yscope-rust-linter: allow-end(comment-greedy-wrap)
```

```console
$ yscope-rust-linter --rule comment-greedy-wrap example.rs
example.rs:2:1: LNT003 unused-suppression: this suppression of comment-greedy-wrap covers no finding on line 3; remove it
example.rs:4:1: LNT003 unused-suppression: this `allow-item` suppression of comment-greedy-wrap covers no finding in its item on lines 5-6; remove it
example.rs:7:1: LNT003 unused-suppression: this region suppression of comment-greedy-wrap covers no finding between lines 7 and 9; remove it
```

### Deprecated rules

A deprecated rule never reports a finding, so a suppression can't cover anything for it. Naming one,
by name or by code, is still valid, and each deprecated rule a suppression names is an
`unused-suppression` at the suppression comment, whatever rules are enabled:

```text
example.rs:3:1: LNT003 unused-suppression: `old-rule` (DOC009) is deprecated and never reports a finding; remove it from this suppression
```

The suppression still covers the findings of the other rules it names. A set prefix never names a
deprecated rule. See the [deprecation policy](rules/README.md#deprecation-policy).

## Meta codes

These codes belong to no rule. They are always on, and no suppression can silence them:

* `LNT001 parse-error`: a file that can't be lexed, or that `syn` can't parse. For a lex error, only
  the comments before it are checked. For a `syn` error, `errors-doc` skips the file.
* `LNT002 invalid-suppression`: a malformed or misplaced suppression.
* `LNT003 unused-suppression`: a suppression that covered no finding, or a deprecated rule that a
  suppression names.

Naming a meta code in a suppression, by name, by code or by the `LNT` prefix, is an
`invalid-suppression`.

## Suppressions with `--diff` and `FILE:RANGES`

`--diff` and `FILE:RANGES` select lines. Suppressions apply first, and the line filter applies after
them:

* A suppression on an unselected line still silences the findings it covers.
* An `unused-suppression` or `invalid-suppression` is reported only when the suppression's own line
  is selected.
* A rule's finding is kept when its anchor touches a selected line. For `comment-greedy-wrap`, the
  anchor is the under-filled line and the next line. For `errors-doc`, it is the whole fn.

In this file, the suppression on line 2 is unused, and the comment on lines 4 and 5 is flagged:

```rust
fn f() {}
// yscope-rust-linter: allow(errors-doc): this one is unused
fn g() {}
// A comment that ends
// early.
```

```console
$ yscope-rust-linter example.rs:5
example.rs:4:1: DOC001 comment-greedy-wrap: line is 22 columns wide and the next line's first word `early.` (6 columns) fits within 100; move it up
```

Selecting only line 5 reports the finding on line 4, since line 5 is part of its anchor. The unused
suppression on line 2 isn't selected, so it isn't reported.
