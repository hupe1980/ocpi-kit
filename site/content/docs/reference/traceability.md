+++
title = "Spec traceability"
weight = 50
description = "Every public item names the specification anchor it implements, and xtask checks that it still matches."
+++

## Every item names its source

Every public item in `ocpi-kit` carries a line naming the AsciiDoc anchor in the OCPI source it
implements:

```rust
/// Spec: 2.3.0 §mod_locations_location_object
pub struct Location { /* … */ }
```

A reviewer — or a partner's compliance team — can go from a Rust type straight to the sentence that
defines it, rather than taking "we implemented the spec" on trust.

## It is checkable, not decorative

`xtask` compares the crate against the specification sources. The OCPI specification is published
under **CC BY-ND 4.0**, so this repository does not redistribute it: put a checkout of
[`ocpi/ocpi`](https://github.com/ocpi/ocpi) under `specs/src/ocpi-<version>/` (2.1.1, 2.2.1, 2.3.0
and the `bookings` and `payments` release branches) and the tools will find it. `specs/` is
gitignored, and CI skips the coverage check with a notice when it is absent.

### `spec-coverage`

```console
$ cargo run -p xtask -- spec-coverage
=== OCPI 2.3.0 ===
59 object(s) match the specification's property tables exactly

=== OCPI 2.2.1 ===
53 object(s) match the specification's property tables exactly

=== OCPI 2.1.1 ===
33 object(s) match the specification's property tables exactly

=== OCPI 2.3.0 bookings branch ===
70 object(s) match the specification's property tables exactly

=== OCPI 2.3.0 payments branch ===
60 object(s) match the specification's property tables exactly
```

It parses the property tables out of the AsciiDoc and the field names out of the Rust source, per
release, and reports anything missing, extra or renamed. `--check` makes a difference fail the
build; `--list` prints the matrix.

The two extension branches carry the whole 2.3.0 core plus their own module, and this crate models
them in the same `v2_3_0` module behind a feature flag. Comparing them separately covers the
Booking and Payments objects and re-checks each branch's copy of the core tables against the core
release.

A field one release adds to an object another release also defines is declared in a
`BRANCH_ONLY_FIELDS` table, so a typo stays distinguishable from a deliberate cross-release
addition.

### `sync-fixtures`

```console
$ cargo run -p xtask -- sync-fixtures
```

Extracts every JSON example the specification ships — including the ones embedded inline in the
2.1.1 Markdown — into `fixtures/<version>/`. Each one is then round-tripped byte-for-byte by
`tests/fixtures.rs`. An upstream errata release becomes a failing test rather than a support
ticket.

Every fixture needs a recorded expectation, and one with none fails the suite — see
[How this crate is verified](@/docs/reference/verification.md).

### `no-floats`

```console
$ cargo run -p xtask -- no-floats
63 file(s) scanned; no floats in the wire models or the pricing engine
```

Enforces the no-`f64` guarantee. It is a repository check rather than a clippy lint because
`clippy::disallowed_types` also fires on serde's generated `visit_f64`, which cannot be allowed
selectively. There is exactly one permitted exception, at the JSON number boundary, and the tool
prints it with its justification on every run.

## In CI

`no-floats` runs on every push. `spec-coverage` runs whenever a spec checkout is present.
`sync-fixtures` is a maintenance task — the fixtures it produces are committed, so
`tests/fixtures.rs` runs everywhere.
