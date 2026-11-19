# `symbol-order`

Code: `ORG001`, in the `ORG` set. See the [rule index](README.md) for the codes of all rules.

The items of each module must follow the symbol ordering convention of the y-scope Rust coding
style:

* Items are grouped by visibility, in the order `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`,
  private.
* Within a group, consts come first, then statics, then types, then fns.
* Each struct, enum or union is directly followed by its impl blocks: its inherent impls first, then
  its impls of traits from the same crate, then its impls of other traits.
* Inside an inherent impl block, associated consts come before fns, and fns are ordered by
  visibility, `pub` first and private last.

```rust
use std::fmt;

const DEFAULT_LIMIT: usize = 10;

pub struct Runner {
    limit: usize,
}

impl fmt::Display for Runner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "runner({})", self.limit)
    }
}

impl Runner {
    fn clamped(&self) -> usize {
        self.limit.min(DEFAULT_LIMIT)
    }

    pub fn new(limit: usize) -> Self {
        Self { limit }
    }
}

pub fn run(runner: &Runner) -> usize {
    runner.clamped()
}
```

```console
$ yscope-rust-linter example.rs
example.rs:3:1: ORG001 symbol-order: private const `DEFAULT_LIMIT` should come after pub fn `run`: pub items come before private items
example.rs:15:1: ORG001 symbol-order: impl `Runner` should come after pub struct `Runner`: inherent impls come before impls of external traits
example.rs:20:5: ORG001 symbol-order: pub fn `new` should come before private fn `clamped`: pub fns come before private fns
```

Each diagnostic names the item to move, where it should go, and the part of the convention it
breaks. Moving the three flagged items as they say gives:

```rust
use std::fmt;

pub struct Runner {
    limit: usize,
}

impl Runner {
    pub fn new(limit: usize) -> Self {
        Self { limit }
    }

    fn clamped(&self) -> usize {
        self.limit.min(DEFAULT_LIMIT)
    }
}

impl fmt::Display for Runner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "runner({})", self.limit)
    }
}

pub fn run(runner: &Runner) -> usize {
    runner.clamped()
}

const DEFAULT_LIMIT: usize = 10;
```

## Contents

* [Which scopes are checked](#which-scopes-are-checked)
* [The order in a module](#the-order-in-a-module)
  * [Visibility groups](#visibility-groups)
  * [Kinds](#kinds)
  * [Impl blocks](#impl-blocks)
* [The order in an inherent impl block](#the-order-in-an-inherent-impl-block)
* [Exempt items](#exempt-items)
* [Which items are reported](#which-items-are-reported)
* [Diagnostics](#diagnostics)
* [Known limits and when to suppress](#known-limits-and-when-to-suppress)

## Which scopes are checked

The rule checks each of these scopes on its own:

* The items at the root of each file.
* The items in the body of each inline module, `mod name { .. }`, nested ones included.
* The items in each inherent impl block, `impl Type { .. }`, at the root of a file or in an inline
  module.

The order of items in a trait declaration or a trait impl block isn't checked, and neither are items
declared inside a fn body.

Test code is skipped unless you pass `--include-tests`, as for [`errors-doc`](errors-doc.md):

* Any item marked `#[cfg(test)]` or `#[cfg(all(test, ..))]`, such as a `mod tests` block, and
  everything inside it.
* A fn with an attribute whose last path segment is `test`, such as `#[test]` or `#[tokio::test]`.
* A module file whose parent declares it with `#[cfg(test)] mod tests;`.

A skipped item is neither checked nor counted, like an [exempt item](#exempt-items). With
`--include-tests`, test items are ordered like any other item, and the body of a `#[cfg(test)] mod
tests` block is checked as a module.

## The order in a module

Every counted item of a module gets a place in the order by its visibility group first, then by its
kind, then, for a type and its impl blocks, by the type it belongs to.

### Visibility groups

The groups are, in order: `pub`, `pub(crate)`, `pub(super)`, `pub(in path)` and private.
`pub(in crate)` counts as `pub(crate)`, `pub(in super)` as `pub(super)`, and `pub(self)` as private.

```rust
fn parse(text: &str) -> usize {
    text.len()
}

pub fn load(text: &str) -> usize {
    parse(text)
}

pub(crate) fn reload(text: &str) -> usize {
    load(text)
}
```

```console
$ yscope-rust-linter example.rs
example.rs:1:1: ORG001 symbol-order: private fn `parse` should come after pub(crate) fn `reload`: pub(crate) items come before private items
```

### Kinds

Within a visibility group, the kinds are, in order:

1. Consts.
2. Statics.
3. Types: structs, enums, unions, traits and type aliases, each with its impl blocks. Types may come
   in any order among themselves.
4. Fns.

```rust
pub fn version() -> u32 {
    VERSION
}

pub const VERSION: u32 = 1;

pub struct Config {
    pub version: u32,
}
```

```console
$ yscope-rust-linter example.rs
example.rs:1:1: ORG001 symbol-order: pub fn `version` should come after pub struct `Config`: types and their impls come before fns
```

### Impl blocks

An impl block has no visibility of its own. It belongs to its type, and it must come directly after
the type or after the type's earlier impl blocks, in this order:

1. Inherent impls: `impl Type { .. }`.
2. Impls of traits from the same crate.
3. Impls of other traits, including the standard library's.

A trait is from the same crate when the file can tell so from its syntax: its path starts with
`crate`, `self` or `super`, or its first segment is a trait or module that the file defines or
declares, or a name that the file imports with a `use` that starts with `crate`, `self`, `super` or
one of the file's modules. Any other trait is external, including a trait of another crate in the
same workspace.

```rust
use std::fmt;

use crate::storage::Storage;

pub struct Cache {
    capacity: usize,
}

impl Clone for Cache {
    fn clone(&self) -> Self {
        Self {
            capacity: self.capacity,
        }
    }
}

impl Storage for Cache {}

pub struct Entry;

impl fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Cache({})", self.capacity)
    }
}
```

```console
$ yscope-rust-linter example.rs
example.rs:17:1: ORG001 symbol-order: impl `Storage for Cache` should come after pub struct `Cache`: impls of crate traits come before impls of external traits
example.rs:21:1: ORG001 symbol-order: impl `fmt::Debug for Cache` should come after impl `Clone for Cache`: a type's impl blocks directly follow it
```

An impl block belongs to a type only when its self type is a plain name, with or without generic
arguments, of a struct, enum or union defined in the same module: `impl Cache`, `impl<T> Wrapper<T>`
or `impl Display for Cache`. Any other impl block is [exempt](#exempt-items), such as an impl for a
type of another module (`impl super::Cache`), for a reference (`impl Trait for &Cache`), for a type
alias, or a blanket impl (`impl<T: Bound> Trait for T`).

An impl block takes its type's visibility group. So the impl blocks of a private type come with the
private items, after every `pub` item.

## The order in an inherent impl block

In an inherent impl block, associated consts come before fns, and fns are ordered by visibility
group: `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`, private. Consts aren't ordered among
themselves by visibility.

```rust
pub struct Window {
    width: u32,
}

impl Window {
    fn clamp(width: u32) -> u32 {
        width.min(Self::MAX_WIDTH)
    }

    pub fn new(width: u32) -> Self {
        Self {
            width: Self::clamp(width),
        }
    }

    pub const MAX_WIDTH: u32 = 1920;
}
```

```console
$ yscope-rust-linter example.rs
example.rs:10:5: ORG001 symbol-order: pub fn `new` should come before private fn `clamp`: pub fns come before private fns
example.rs:16:5: ORG001 symbol-order: pub const `MAX_WIDTH` should come before private fn `clamp`: associated consts come before fns
```

Associated types and macro invocations in an impl block are exempt.

## Exempt items

These items are neither checked nor counted. They may sit anywhere, and the items around them are
compared as if they weren't there:

* `use` and `extern crate` items.
* Modules, both `mod name;` declarations and inline modules. An inline module's body is still
  checked as its own scope.
* `macro_rules!` definitions and macro invocations in item position.
* `extern` blocks.
* Impl blocks that don't belong to a struct, enum or union of the same module. See [Impl
  blocks](#impl-blocks).

```rust
pub fn first() {}

use std::collections::HashMap;

mod helpers;

macro_rules! define {
    () => {};
}

define!();

pub fn second() -> HashMap<u32, u32> {
    HashMap::new()
}
```

This module is in order.

## Which items are reported

The rule reports as few items as it can: the smallest set of items such that moving each of them to
the place its diagnostic names puts the scope in order. Every other item stays where it is. So one
misplaced item is one diagnostic, even if it sits above many items that should come before it:

```rust
fn helper() -> u32 {
    1
}

pub fn first() -> u32 {
    helper()
}

pub fn second() -> u32 {
    helper()
}

pub fn third() -> u32 {
    helper()
}
```

```console
$ yscope-rust-linter example.rs
example.rs:1:1: ORG001 symbol-order: private fn `helper` should come after pub fn `third`: pub items come before private items
```

When several sets of the same size would work, the rule keeps the earlier items in place and reports
the later ones. Here, moving either `helper` or `second` would put the module in order, and
`second` is reported:

```rust
pub fn first() -> u32 {
    helper()
}

fn helper() -> u32 {
    1
}

pub fn second() -> u32 {
    helper()
}
```

```console
$ yscope-rust-linter example.rs
example.rs:9:1: ORG001 symbol-order: pub fn `second` should come after pub fn `first`: pub items come before private items
```

## Diagnostics

Each diagnostic is at the first token of the misplaced item after its attributes and docstring: its
visibility, or its keyword. Its message has this form:

```text
<item> should come <after|before> <reference>: <reason>
```

* `<item>` and `<reference>` describe an item by its visibility group, its kind and its name, such
  as ``pub(crate) fn `load` `` or ``private struct `State` ``. An impl block is described by what it
  implements, such as ``impl `State` `` or ``impl `fmt::Display for State` ``.
* `<reference>` is an item that stays in place. The item should come right after the last such item
  that may come before it, or, if there is none, before the first item that stays in place.
* `<reason>` is the part of the convention that the item breaks:
  * Visibility groups: `pub items come before private items`, with the two groups.
  * Kinds: `consts come before fns`, with `consts`, `statics`, `types and their impls` and `fns`.
  * A type's impl blocks: `a type's impl blocks directly follow it`, for an impl block that is away
    from its type.
  * The order of impl blocks: `inherent impls come before impls of crate traits`, `impls of crate
    traits come before impls of external traits` or `inherent impls come before impls of external
    traits`.
  * In an inherent impl block: `associated consts come before fns`, or `pub fns come before private
    fns`, with the two groups.

With `--diff` or `FILE:RANGES`, a diagnostic is kept when any line from the misplaced item through
its reference item is selected. So inserting an item that puts an existing item out of order
reports that item, even though its own lines didn't change.

## Known limits and when to suppress

* Traits from the same crate are recognized by syntax only. A trait imported with a glob (`use
  super::*`), or re-exported under another name in another file, counts as external.
* An impl block for a qualified path, a type alias or a generic parameter is exempt, even when it
  is far from its type.
* Items generated by macros are invisible to the rule, and macro invocations are exempt.
* When an inserted item and an existing item could each be moved, the later one is reported, which
  may be the existing item.

To keep an item where it is, put an `allow` right above it, or above its docstring:

```rust
pub fn first() -> u32 {
    1
}

// yscope-rust-linter: allow(symbol-order): kept next to the fn it describes
fn first_helper() -> u32 {
    first()
}

pub fn second() -> u32 {
    2
}

pub fn third() -> u32 {
    3
}
```

This module has no diagnostic. See [suppression comments](../suppression.md) for the other forms.
