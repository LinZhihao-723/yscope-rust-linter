# yscope-rust-linter

`yscope-rust-linter` checks the y-scope Rust style rules that neither rustfmt nor clippy can check:

* Comment paragraphs are filled greedily, so no line leaves room for the next line's first word.
* A fn's `# Errors` docstring section matches its body: it lists every error the fn returns directly
  and every function whose errors it forwards, and nothing else.
* The items of a module and of an inherent impl block follow the symbol ordering convention.
* A `# Type Parameters` section documents every generic parameter of an item.
* A `# Parameters` section documents every parameter of a trait method declaration, and appears
  nowhere else.

It reads the source only, without building the crate, so it can lint a whole repository or only the
lines a pull request changes.

## Rules

| Rule ID | What it checks |
| ------- | -------------- |
| [`comment-greedy-wrap`](docs/rules/comment-greedy-wrap.md) | Every `//`, `///` and `//!` comment paragraph is filled greedily to the width (100 columns by default). |
| [`errors-doc`](docs/rules/errors-doc.md) | The `# Errors` section of a fn's docstring documents every error the fn returns directly and every function whose errors it forwards, and nothing it doesn't. |
| [`symbol-order`](docs/rules/symbol-order.md) | The items of each module are grouped by visibility (`pub` first, private last), and within a group ordered consts, statics, types, then fns, with each type directly followed by its impl blocks. Inside an inherent impl, associated consts come before fns, and `pub` fns before private ones. |
| [`trait-params-doc`](docs/rules/trait-params-doc.md) | A documented trait method declaration has a `# Parameters` section with one bullet per parameter, and no other item has a `# Parameters` section. |
| [`type-params-doc`](docs/rules/type-params-doc.md) | A documented item with generic parameters has a `# Type Parameters` section with one bullet per type, lifetime and const parameter, and nothing else. |

A rule can be silenced for a line, a comment block, an item, a region or a file with a suppression
comment that gives a reason. See [Suppression comments](docs/suppression.md).

The design and the internals are in [`docs/design.md`](docs/design.md).

## Install

```text
cargo install --locked --git https://github.com/LinZhihao-723/yscope-rust-linter yscope-rust-linter
```

## Usage

```text
yscope-rust-linter [OPTIONS] [TARGET]...
```

| Argument             | Meaning                                                                                  |
| -------------------- | ---------------------------------------------------------------------------------------- |
| `TARGET`             | A file, a directory, or `FILE:RANGES` (for example `src/a.rs:10-20,30,40-45`).           |
| `--rule <ID>`        | Enable only this rule. Repeatable. By default, every rule runs.                          |
| `--width <N>`        | The fill width for `comment-greedy-wrap`. Default: 100.                                  |
| `--include-tests`    | Also check `#[test]` fns and `#[cfg(test)]` items in the rules that skip them.           |
| `--exclude <PATH>`   | Skip this file or directory, matched as a path prefix by whole components. Repeatable.   |
| `--diff <FILE>`      | Lint only the lines changed by a unified diff. `-` reads the diff from stdin.            |
| `--format <FORMAT>`  | `text` (default) or `github` (GitHub Actions `::error` workflow commands).               |

* Directories are walked recursively for `.rs` files, skipping `target/` and hidden directories.
* A file named explicitly is always checked.
* A file with a plain `//` comment holding the word `@generated` in its first five lines is skipped.
  The summary on stderr counts it separately.

Each diagnostic is one line on stdout, sorted by path, line and column:

```text
<path>:<line>:<column>: <code>: <message>
```

Besides the rule IDs, these meta codes are always on and can't be suppressed:

* `invalid-suppression`: a malformed or misplaced suppression comment.
* `unused-suppression`: a suppression comment that covered no finding.
* `parse-error`: a file that can't be lexed, or that `syn` can't parse.

Exit codes:

* `0`: no diagnostics.
* `1`: at least one diagnostic.
* `2`: a usage error, an invalid target or diff, or an I/O failure. Nothing is printed to stdout.

## Using it in CI

To lint only the lines a branch changes, pipe a zero-context diff into `--diff -` from the
repository root:

```text
git diff -U0 origin/main -- '*.rs' | yscope-rust-linter --diff -
```

On GitHub Actions, `--format github` turns each diagnostic into an annotation on the pull request:

```text
git diff -U0 origin/main -- '*.rs' | yscope-rust-linter --diff - --format github
```

* The diff's `+++` paths must have the `b/` prefix. If your Git config changes the prefixes, add
  `--no-ext-diff --src-prefix=a/ --dst-prefix=b/` to the `git diff` command.
* A diff with context lines (without `-U0`) is accepted too, but its context lines are linted as if
  they had changed.
* A finding is reported when it touches a changed line. For `errors-doc`, that is any line of the
  fn, so deleting a bullet reports the forward it documented.
* Suppressions apply before the line filter. See [Suppressions with `--diff` and
  `FILE:RANGES`](docs/suppression.md#suppressions-with---diff-and-fileranges).
