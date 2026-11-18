# `type-params-doc`

A documented item with generic parameters must have a `# Type Parameters` section with one bullet
for each of them: each type parameter, lifetime parameter and const generic. An item without generic
parameters must not have the section.

```rust
/// A cache of loaded values.
///
/// # Type Parameters
///
/// * `Value` - The type of the cached values.
struct Cache<'store, Value, const CAPACITY: usize> {
    store: &'store Store,
    values: [Option<Value>; CAPACITY],
}
```

```console
$ yscope-rust-linter example.rs
example.rs:3:1: type-params-doc: `# Type Parameters` doesn't document `'store`; add "* `'store` - ..."
example.rs:3:1: type-params-doc: `# Type Parameters` doesn't document `CAPACITY`; add "* `CAPACITY` - ..."
```

The section needs a bullet for the lifetime and for the const generic too:

```rust
/// A cache of loaded values.
///
/// # Type Parameters
///
/// * `'store` - The lifetime of the store the values are loaded from.
/// * `Value` - The type of the cached values.
/// * `CAPACITY` - The number of values the cache holds.
struct Cache<'store, Value, const CAPACITY: usize> {
    store: &'store Store,
    values: [Option<Value>; CAPACITY],
}
```

## Contents

* [Which items are checked](#which-items-are-checked)
* [Which parameters are documented](#which-parameters-are-documented)
* [The section format](#the-section-format)
* [Diagnostics](#diagnostics)
* [Known limits and when to suppress](#known-limits-and-when-to-suppress)

## Which items are checked

These items are checked when they have a docstring:

* Free fns, including fns nested in other fns.
* Inherent methods, in an `impl Type` block.
* Trait method declarations, with or without a default body.
* Structs, enums, unions, traits and type aliases.

These are not checked:

* Trait impl methods, in an `impl Trait for Type` block. The trait's declaration documents the
  method.
* An item without a docstring, or with only blank doc lines. The rule checks the docstrings that
  exist; it doesn't require one. This is the policy of [`errors-doc`](errors-doc.md), which checks
  only the fns that have an `# Errors` section.
* Test code, unless you pass `--include-tests`: fns with a `test` attribute such as `#[test]` or
  `#[tokio::test]`, items marked `#[cfg(test)]` with everything inside them, and module files whose
  parent declares them with `#[cfg(test)] mod tests;`. See
  [`errors-doc`](errors-doc.md#which-fns-are-checked) for how such a parent is found.

```rust
/// A store of values.
trait Store {
    /// Loads a value.
    fn load<Value: Decode>(&self) -> Value;
}

impl Store for MemoryStore {
    /// Loads a value from memory.
    fn load<Value: Decode>(&self) -> Value {
        todo!()
    }
}

fn decode<Value: Decode>(bytes: &[u8]) -> Value {
    todo!()
}
```

```console
$ yscope-rust-linter example.rs
example.rs:3:5: type-params-doc: trait method `load` has generic parameters but no `# Type Parameters` section; add one with a bullet for each of `Value`
```

The rule needs the file's syntax tree. A file that `syn` can't parse gets one `parse-error`
diagnostic that names every skipped rule, and the rule skips the file.

## Which parameters are documented

An item documents its own generic parameters, the ones in the `<..>` after its name, in this form:

* A type parameter by its name, such as `` `Value` ``.
* A lifetime parameter by its name with the tick, such as `` `'store` ``. A bullet for `` `store` ``
  doesn't document `'store`.
* A const generic by its name, such as `` `CAPACITY` ``.

These are not the item's generic parameters, so they need no bullet, and a bullet for them is
reported:

* The generic parameters of the `impl` block a method is in. A method documents only its own.
* The generic parameters of the trait a method is declared in.
* Elided and anonymous lifetimes, such as `&self` and `'_`.
* The types of `impl Trait` arguments.
* The lifetimes of `for<'a>` bounds.

```rust
impl<Value> Cache<Value> {
    /// Gets a value.
    fn get(&self, key: &'_ str) -> Option<&Value> {
        todo!()
    }

    /// Maps the cached values.
    ///
    /// # Type Parameters
    ///
    /// * `Value` - The type of the cached values.
    /// * `Output` - The type of the mapped values.
    fn map<Output>(&self, mapper: impl Fn(&Value) -> Output) -> Vec<Output> {
        todo!()
    }
}
```

```console
$ yscope-rust-linter example.rs
example.rs:11:5: type-params-doc: `# Type Parameters` documents `Value`, which isn't a generic parameter of method `map`; remove the bullet
```

`get` has no generic parameters of its own, so it needs no section. `Value` belongs to the impl
block, so `map` documents only `Output`.

An item without generic parameters must not have a `# Type Parameters` section:

```rust
/// Clears the cache.
///
/// # Type Parameters
///
/// * `Value` - The type of the cached values.
fn clear(cache: &mut Cache<u8>) {}
```

```console
$ yscope-rust-linter example.rs
example.rs:3:1: type-params-doc: fn `clear` has no generic parameters; remove the `# Type Parameters` section
```

## The section format

The section follows the y-scope Rust style guide, with one top-level bullet per parameter:

```text
/// # Type Parameters
///
/// * `'name` - <What the lifetime is the lifetime of.>
/// * `Name` - <What the type is.>
```

* The header is `# Type Parameters`, exactly. A header such as `# Type parameters` doesn't count, so
  the item has no section.
* The section starts at the `Type Parameters` header of the smallest level and ends at the next
  header of the same or a smaller level, as the `# Errors` section does in
  [`errors-doc`](errors-doc.md#section-selection).
* Each top-level bullet starts with the parameter's name as a code span, then ` - `, then a
  description. The description may continue on the next lines, and may have sub-bullets.
* A bullet that starts with a code link, such as ``* [`Value`] - ...``, or uses another separator,
  such as ``* `Value`: ...``, still documents its parameter, but is reported.
* The bullets may come in any order. Prose in the section is ignored.

```rust
/// Runs the jobs.
///
/// # Type Parameters
///
/// * [`Executor`]: The executor that runs each job.
/// * `Executor` - The executor again.
/// * The type of each job.
/// * `'lock` - The lifetime of the lock the jobs hold.
///
/// # Type Parameters
///
/// * `'lock` - The lifetime of the lock the jobs hold.
fn run<'lock, Executor, Job>(executor: &Executor, jobs: Vec<Job>) {}
```

```console
$ yscope-rust-linter example.rs
example.rs:3:1: type-params-doc: `# Type Parameters` doesn't document `Job`; add "* `Job` - ..."
example.rs:5:1: type-params-doc: write this `# Type Parameters` bullet as "* `Executor` - description"
example.rs:6:1: type-params-doc: `Executor` is already documented on line 5; remove this bullet
example.rs:7:1: type-params-doc: this `# Type Parameters` bullet doesn't start with the generic parameter it documents; write "* `name` - description"
example.rs:10:1: type-params-doc: the docstring has a second `# Type Parameters` section at the same level; merge them
```

## Diagnostics

Every diagnostic has the code `type-params-doc`.

| Message | Cause | Fix |
| ------- | ----- | --- |
| `` I has generic parameters but no `# Type Parameters` section; add one with a bullet for each of `'a`, `T` `` | The item has generic parameters, and its docstring has no section. Reported at the docstring's first line. | Add the section with a bullet per parameter. |
| `` I has no generic parameters; remove the `# Type Parameters` section `` | The item has no generic parameters of its own. Reported at the header. | Remove the section. |
| `` `# Type Parameters` doesn't document `T`; add "* `T` - ..." `` | No bullet names `T`. Reported at the header. | Add a bullet for `T`. |
| `` `# Type Parameters` documents `X`, which isn't a generic parameter of I; remove the bullet `` | The bullet names something that isn't one of the item's own generic parameters. | Remove the bullet, or fix the name. |
| `` `T` is already documented on line N; remove this bullet `` | Two bullets name `T`. | Merge them. |
| `` write this `# Type Parameters` bullet as "* `T` - description" `` | The bullet names `T`, but not with a bare code span followed by ` - ` and a description. | Rewrite the bullet. |
| `` this `# Type Parameters` bullet doesn't start with the generic parameter it documents; write "* `name` - description" `` | The bullet starts with prose. | Start it with the parameter's name. |
| `` the docstring has a second `# Type Parameters` section at the same level; merge them `` | Two `Type Parameters` headers of the same level. | Merge the sections. |

`I` is the item, such as ``fn `load` ``, ``method `get` ``, ``trait method `load` ``, ``struct
`Cache` `` or ``type alias `Map` ``.

With `--diff` or `FILE:RANGES`, a diagnostic is kept when any line of its item is selected, from its
first attribute or doc line to its last line.

## Known limits and when to suppress

* Only the exact title counts. A misspelled header, such as `# Type parameters`, isn't reported as
  such: an item with generic parameters gets the missing-section finding, and one without gets
  none.
* A docstring built only from `#[doc = include_str!(..)]` counts as undocumented.
* Items generated by macros are not seen.

When the rule is wrong for an item, suppress it with a reason:

```rust
// yscope-rust-linter: allow(type-params-doc): the parameters mirror `JobState`'s, documented there
/// A handle to a job's state.
struct JobStateHandle<OutputType, TaskType> {
    state: JobState<OutputType, TaskType>,
}
```

An own-line `allow` above the item's docstring, or anywhere from its first attribute or doc line
through the item's first line, covers every finding of the item. `allow-item` above the item covers
it too. See [Suppression comments](../suppression.md).
