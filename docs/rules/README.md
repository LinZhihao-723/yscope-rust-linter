# Rules

Every rule has a name, such as `type-params-doc`, and a stable code, such as `DOC003`. Each
diagnostic prints both, code first:

```text
src/cache.rs:12:1: DOC003 type-params-doc: struct `Cache` has generic parameters but no `# Type Parameters` section; add one with a bullet for each of `Value`
```

## Contents

* [Index](#index)
* [Meta codes](#meta-codes)
* [Codes and sets](#codes-and-sets)
* [Selecting rules by name, code or set](#selecting-rules-by-name-code-or-set)
* [Deprecation policy](#deprecation-policy)

## Index

| Code | Name | Set | What it checks |
| ---- | ---- | --- | -------------- |
| `DOC001` | [`comment-greedy-wrap`](comment-greedy-wrap.md) | `DOC` | Every `//`, `///` and `//!` comment paragraph is filled greedily to the width (100 columns by default). |
| `DOC002` | [`errors-doc`](errors-doc.md) | `DOC` | The `# Errors` section of a fn's docstring documents every error the fn returns directly and every function whose errors it forwards, and nothing it doesn't. |
| `DOC003` | [`type-params-doc`](type-params-doc.md) | `DOC` | A documented item with generic parameters has a `# Type Parameters` section with one bullet per type, lifetime and const parameter, and nothing else. |
| `DOC004` | [`trait-params-doc`](trait-params-doc.md) | `DOC` | A documented trait method declaration has a `# Parameters` section with one bullet per parameter, and no other item has a `# Parameters` section. |
| `ORG001` | [`symbol-order`](symbol-order.md) | `ORG` | The items of each module and of each inherent impl block follow the symbol ordering convention. |

## Meta codes

The linter's own diagnostics have codes too. They belong to no rule, are always on, and can't be
selected or suppressed. See [Meta codes](../suppression.md#meta-codes).

| Code | Name | What it reports |
| ---- | ---- | --------------- |
| `LNT001` | `parse-error` | A file that can't be lexed, or that `syn` can't parse. |
| `LNT002` | `invalid-suppression` | A malformed or misplaced suppression comment. |
| `LNT003` | `unused-suppression` | A suppression comment that covered no finding, or that names a deprecated rule. |

## Codes and sets

A code is a three-letter uppercase set prefix followed by a three-digit number. The set says what
kind of rule it is:

| Set | What its rules check |
| --- | -------------------- |
| `DOC` | Comments and docstrings. |
| `ORG` | Code organization: ordering, layout and placement. |
| `LNT` | Nothing: it holds the linter's own diagnostics, the [meta codes](#meta-codes). |

* Codes are numbered from `001` in each set, in the order the rules were added.
* A code is never renumbered, and never reused by another rule, even after its rule is
  [deprecated](#deprecation-policy).
* A new rule takes the next free number in its set. So the next `DOC` rule is `DOC005`, and the next
  `ORG` rule is `ORG002`.

## Selecting rules by name, code or set

`--rule` and every [suppression comment](../suppression.md) accept three spellings:

* A rule name, such as `type-params-doc`.
* A rule code, such as `DOC003`.
* A set prefix, such as `DOC`, which names every rule of the set that isn't deprecated.

```text
yscope-rust-linter --rule DOC003 src
yscope-rust-linter --rule DOC --rule symbol-order src
// yscope-rust-linter: allow(DOC003): the parameters are documented on the trait
// yscope-rust-linter: allow-file(DOC): the bindings are generated
```

Matching is case-sensitive. Names are lowercase, and codes and set prefixes are uppercase, as
written in the tables above. A lowercase code such as `doc003` is an unknown rule, and the error
says to write `DOC003`. Every code and set has one spelling, so a search for `DOC003` finds every
suppression of `type-params-doc` that uses its code.

A meta code can't be selected, whether by name, by code or by the `LNT` prefix. An unknown value is
a usage error for `--rule` and an `invalid-suppression` in a suppression.

## Deprecation policy

If we decide to remove a rule, we mark it deprecated and never reuse its code.

A deprecated rule stays registered, with its name and code, but it never runs and never reports a
finding:

* A set prefix skips it. `--rule DOC` and `allow(DOC)` name only the `DOC` rules that aren't
  deprecated.
* Naming it in `--rule`, by name or by code, is accepted with a warning on stderr. The run goes on
  without the rule:

  ```text
  yscope-rust-linter: warning: `old-rule` (DOC009) is deprecated and never runs; remove it from `--rule`
  ```

* Naming it in a suppression, by name or by code, is accepted, and is not an `invalid-suppression`.
  The suppression covers nothing for that rule, and each deprecated rule it names is an
  `unused-suppression` at the suppression comment:

  ```text
  src/a.rs:3:1: LNT003 unused-suppression: `old-rule` (DOC009) is deprecated and never reports a finding; remove it from this suppression
  ```

  The suppression still covers the findings of the other rules it names.

No rule is deprecated yet. A deprecated rule keeps its row in the [index](#index), marked
deprecated, and its page, which says what replaced it.
