# `trait-params-doc`

A documented method declaration in a `trait` block must have a `# Parameters` section with one
bullet for each of its parameters. No other item may have a `# Parameters` section: the parameters
of regular fns, inherent methods and trait impl methods are self-documenting.

```rust
/// A store of jobs.
trait JobStore {
    /// Submits a job.
    ///
    /// # Parameters
    ///
    /// * `job_id` - The ID of the job.
    fn submit(&self, job_id: JobId, inputs: Vec<u8>);
}

impl JobStore for MemoryStore {
    /// Submits a job to memory.
    ///
    /// # Parameters
    ///
    /// * `job_id` - The ID of the job.
    /// * `inputs` - The inputs of the job.
    fn submit(&self, job_id: JobId, inputs: Vec<u8>) {}
}
```

```console
$ yscope-rust-linter example.rs
example.rs:5:5: trait-params-doc: `# Parameters` doesn't document `inputs`; add "* `inputs` - ..."
example.rs:14:5: trait-params-doc: `# Parameters` belongs only in trait method declarations; remove it from trait impl method `submit`
```

The declaration documents every parameter, and the impl doesn't repeat them:

```rust
/// A store of jobs.
trait JobStore {
    /// Submits a job.
    ///
    /// # Parameters
    ///
    /// * `job_id` - The ID of the job.
    /// * `inputs` - The inputs of the job.
    fn submit(&self, job_id: JobId, inputs: Vec<u8>);
}

impl JobStore for MemoryStore {
    /// Submits a job to memory.
    fn submit(&self, job_id: JobId, inputs: Vec<u8>) {}
}
```

## Contents

* [Which items are checked](#which-items-are-checked)
* [Which parameters are documented](#which-parameters-are-documented)
* [The section format](#the-section-format)
* [Diagnostics](#diagnostics)
* [Known limits and when to suppress](#known-limits-and-when-to-suppress)

## Which items are checked

Trait method declarations are checked when they have a docstring, with or without a default body.

Every other documented item is checked for a `# Parameters` section it must not have: free fns,
fns nested in other fns, inherent methods, trait impl methods, structs, enums, unions, traits and
type aliases. The bullets of such a section aren't checked.

```rust
/// Starts the runner.
///
/// # Parameters
///
/// * `runner` - The runner to start.
fn start(runner: &Runner) {}
```

```console
$ yscope-rust-linter example.rs
example.rs:3:1: trait-params-doc: `# Parameters` belongs only in trait method declarations; remove it from fn `start`
```

These are not checked:

* An item without a docstring, or with only blank doc lines. The rule checks the docstrings that
  exist; it doesn't require one. This is the policy of [`errors-doc`](errors-doc.md), which checks
  only the fns that have an `# Errors` section.
* Test code, unless you pass `--include-tests`: fns with a `test` attribute such as `#[test]` or
  `#[tokio::test]`, items marked `#[cfg(test)]` with everything inside them, and module files whose
  parent declares them with `#[cfg(test)] mod tests;`. See
  [`errors-doc`](errors-doc.md#which-fns-are-checked) for how such a parent is found.

The rule needs the file's syntax tree. A file that `syn` can't parse gets one `parse-error`
diagnostic that names every skipped rule, and the rule skips the file.

## Which parameters are documented

A trait method's parameters are its inputs after the receiver. The receiver (`self`, `&self`, `&mut
self`, `self: Box<Self>`) is never documented.

* A parameter bound to a name, such as `job_id: JobId` or `mut buffer: Vec<u8>`, needs a bullet. So
  does a name with a leading underscore, such as `_context: Context`.
* A `_` parameter is skipped.
* A destructuring parameter, such as `(x, y): (u8, u8)`, is skipped. A bullet may document a name
  it binds, such as `x`, but none is required.

```rust
/// A canvas.
trait Canvas {
    /// Draws a point.
    fn draw(&mut self, (x, y): (u8, u8), _: Color, _layer: Layer) {}

    /// Clears the canvas.
    ///
    /// # Parameters
    ///
    /// * `self` - The canvas.
    fn clear(&mut self);
}
```

```console
$ yscope-rust-linter example.rs
example.rs:3:5: trait-params-doc: trait method `draw` has parameters but no `# Parameters` section; add one with a bullet for each of `_layer`
example.rs:8:5: trait-params-doc: trait method `clear` has no parameters; remove the `# Parameters` section
```

## The section format

The section follows the y-scope Rust style guide, with one top-level bullet per parameter:

```text
/// # Parameters
///
/// * `name` - <What the parameter is.>
```

* The header is `# Parameters`, exactly. `# Arguments` and `# Params` aren't recognized.
* The section starts at the `Parameters` header of the smallest level and ends at the next header of
  the same or a smaller level, as the `# Errors` section does in
  [`errors-doc`](errors-doc.md#section-selection).
* Each top-level bullet starts with the parameter's name as a code span, then ` - `, then a
  description. The description may continue on the next lines, and may have sub-bullets.
* A bullet that starts with a code link, such as ``* [`job_id`] - ...``, or uses another separator,
  such as ``* `job_id`: ...``, still documents its parameter, but is reported.
* The bullets may come in any order. Prose in the section is ignored.

```rust
/// A store of jobs.
trait JobStore {
    /// Cancels a job.
    ///
    /// # Parameters
    ///
    /// * `job_id`: The ID of the job.
    /// * `job_id` - The ID again.
    /// * The reason for the cancellation.
    /// * `force` - Whether to cancel running tasks.
    ///
    /// # Parameters
    ///
    /// * `reason` - The reason for the cancellation.
    fn cancel(&self, job_id: JobId, reason: String);
}
```

```console
$ yscope-rust-linter example.rs
example.rs:5:5: trait-params-doc: `# Parameters` doesn't document `reason`; add "* `reason` - ..."
example.rs:7:5: trait-params-doc: write this `# Parameters` bullet as "* `job_id` - description"
example.rs:8:5: trait-params-doc: `job_id` is already documented on line 7; remove this bullet
example.rs:9:5: trait-params-doc: this `# Parameters` bullet doesn't start with the parameter it documents; write "* `name` - description"
example.rs:10:5: trait-params-doc: `# Parameters` documents `force`, which isn't a parameter of trait method `cancel`; remove the bullet
example.rs:12:5: trait-params-doc: the docstring has a second `# Parameters` section at the same level; merge them
```

## Diagnostics

Every diagnostic has the code `trait-params-doc`.

| Message | Cause | Fix |
| ------- | ----- | --- |
| `` trait method `f` has parameters but no `# Parameters` section; add one with a bullet for each of `a`, `b` `` | The trait method has parameters that need a bullet, and its docstring has no section. Reported at the docstring's first line. | Add the section with a bullet per parameter. |
| `` trait method `f` has no parameters; remove the `# Parameters` section `` | The trait method has no parameters besides its receiver, `_` and patterns that bind no name. Reported at the header. | Remove the section. |
| `` `# Parameters` belongs only in trait method declarations; remove it from I `` | The item isn't a trait method declaration. Reported at the header. | Remove the section. |
| `` `# Parameters` doesn't document `a`; add "* `a` - ..." `` | No bullet names `a`. Reported at the header. | Add a bullet for `a`. |
| `` `# Parameters` documents `x`, which isn't a parameter of trait method `f`; remove the bullet `` | The bullet names something that isn't a parameter. | Remove the bullet, or fix the name. |
| `` `a` is already documented on line N; remove this bullet `` | Two bullets name `a`. | Merge them. |
| `` write this `# Parameters` bullet as "* `a` - description" `` | The bullet names `a`, but not with a bare code span followed by ` - ` and a description. | Rewrite the bullet. |
| `` this `# Parameters` bullet doesn't start with the parameter it documents; write "* `name` - description" `` | The bullet starts with prose. | Start it with the parameter's name. |
| `` the docstring has a second `# Parameters` section at the same level; merge them `` | Two `Parameters` headers of the same level. | Merge the sections. |

`I` is the item, such as ``fn `start` ``, ``method `new` ``, ``trait impl method `submit` `` or
``struct `Job` ``.

With `--diff` or `FILE:RANGES`, a diagnostic is kept when any line of its item is selected, from its
first attribute or doc line to its last line.

## Known limits and when to suppress

* Only the exact title counts. A misspelled header, such as `# Arguments`, isn't reported as such:
  a trait method with parameters gets the missing-section finding, and any other item gets none.
* A docstring built only from `#[doc = include_str!(..)]` counts as undocumented.
* Items generated by macros are not seen.

When the rule is wrong for an item, suppress it with a reason:

```rust
/// A store of jobs.
trait JobStore {
    // yscope-rust-linter: allow(trait-params-doc): the parameters are documented on `Job::new`
    /// Creates a job.
    fn create(&self, name: String, inputs: Vec<u8>);
}
```

An own-line `allow` above the item's docstring, or anywhere from its first attribute or doc line
through the item's first line, covers every finding of the item. `allow-item` above the item covers
it too. See [Suppression comments](../suppression.md).
