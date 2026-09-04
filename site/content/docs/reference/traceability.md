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
under **CC BY-ND 4.0**, so this repository does not redistribute it — it *pins* it.

```console
$ cargo run -p xtask -- spec-sync --fetch
ocpi-2.3.0 <- 2.3.0/release/core @ 3979887793
ocpi-2.3.0-payments <- 2.3.0/release/payments @ 774e23a925
ocpi-2.3.0-bookings <- 2.3.0/release/bookings @ 6ad981f428
ocpi-2.2.1 <- release-2.2.1-bugfixes @ ca8c04c5c8
ocpi-2.1.1 <- release-2.1.1-bugfixes @ 28ea45329a
```

`spec-sources.toml` names each release, its branch and its commit; `spec-sources.lock` records a
SHA-256 for each of the 479 files the censuses read. `--check` verifies the checkout against the
lock and names every file that was added, removed or changed; `--update` re-records it, so a
specification change arrives as a reviewable diff.

**Why a pin.** OCPI edits released branches in place, with no version bump: a module can move
between 2.3.0 core and a release branch under an unchanged version number. A crate written against
the earlier layout compiles and passes its tests while being wrong about which release defines
what, so `spec-sync --latest` runs weekly and fails when a pinned branch moves.

### `spec-coverage`

```console
$ cargo run -p xtask -- spec-coverage
=== OCPI 2.3.0 ===
58 object(s) match the specification's property tables exactly

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

### `enum-coverage`

```console
$ cargo run -p xtask -- enum-coverage
=== OCPI 2.3.0 enums ===
33 enum(s) match the specification's value tables exactly
…
Every enum the specification defines has exactly the values it lists.
```

The same idea one level down. A field census compares *names* and says nothing about what may go
in the field, and a missing enum value is the same defect hidden deeper:

* on a **closed** enum, a conformant peer's object stops decoding — a whole page of Locations lost
  over one plug type;
* on an **open** enum it is quieter and worse. The value survives, in `Custom("MCS")`, so nothing
  fails. It simply never matches `ConnectorType::Mcs` in any `match` you write, and your megawatt
  chargers are invisible.

Neither shows up in a fixture round-trip unless the specification happens to ship an example using
that value, which for a 42-value enum it does not.

### `field-shapes`

```console
$ cargo run -p xtask -- field-shapes
=== OCPI 2.3.0 field shapes ===
286 field(s) match the specification's cardinality and length
…
Every field's cardinality and length matches the specification's tables.
```

A property table has four columns. `spec-coverage` reads the name and `enum-coverage` reads the
values; this reads the other two — the `Card.` marker (`1`, `?`, `*`, `+`) and the length in
`CiString(36)`.

Both decide behaviour. A field the table marks `?` and the crate makes required rejects a
conformant peer's object outright; one marked `1` and modelled `Option` lets this crate emit an
object missing a mandatory field. A length that is too small reports a conformant value as
`TooLong` — and since outgoing validation is on by default, refuses to send it.

`SignedData.url` is the case it pins down: a `string(512)`, not the `string(255)` `URL` type, so
modelling it as a `Url` would reject a conformant link. See
[Spec errata](@/docs/reference/errata.md).

The crate's semantic aliases are resolved before comparing — `PartyId` *is* `CiString(3)` — and the
alias table is itself checked against `src/types/ids.rs`, so a `PartyId` that quietly became a
`CiString<4>` fails the run rather than silently excusing every `party_id` in the crate.

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
66 file(s) scanned; no floats in the wire models or the pricing engine
```

Enforces the no-`f64` guarantee. It is a repository check rather than a clippy lint because
`clippy::disallowed_types` also fires on serde's generated `visit_f64`, which cannot be allowed
selectively. There is exactly one permitted exception, at the JSON number boundary, and the tool
prints it with its justification on every run.

### `dead-config`

```console
$ cargo run -p xtask -- dead-config
24 configuration field(s) across 6 struct(s); every one is read somewhere
```

Fails the build when a public field of `Quirks`, `ClientConfig`, `RetryPolicy`, `ServerConfig`,
`PricingPolicy` or `UrlPolicy` is only ever *assigned* and never read. A setting that does nothing
is worse than a missing feature: somebody reads the doc comment, sets the flag, believes the
problem is handled, and ships.

It also refuses a field name **shared** by two of those structs: it matches a read by name and has
no type information, so a shared name would make it blind to a dead field on either.

### `endpoints`

```console
$ cargo run -p xtask -- endpoints
[+] /LOC1/3256/1
[+] /NL/TNM/012345678?type=RFID
[+] /terminals/TERM-042/deactivate
…
16 endpoint URL(s) match the structures the specification documents
```

The three censuses compare *objects*. This compares **URLs**, which is where an OCPI integration
breaks: a missing owner segment, a query parameter with the wrong name, a sub-path invented rather
than discovered.

Each case carries the endpoint structure verbatim as the chapter writes it — say
`{locations_endpoint_url}/{country_code}/{party_id}/{location_id}[/{evse_uid}][/{connector_id}]` —
and checks two things: that the pattern is still in that chapter character for character, and that
expanding it with canned values gives exactly what the crate's builder produces. One half is
anchored in the specification, the other in the code.

It is also where this crate's reading of the Payments endpoint erratum is written down: the case
states, in a field a reviewer can disagree with, that `{payments_terminals_endpoint_url}` is the
discovered `payments` endpoint plus `/terminals`.

### `errata`

```console
$ cargo run -p xtask -- errata
[ok] E13 the TIME/PARKING_TIME `step_size` sentence and its own worked example justify the same
        answer differently — still in ocpi-2.3.0/mod_cdrs.asciidoc
…
19 erratum/errata re-derived from the specification; every one is still real
```

The crate is *shaped* around several entries on the [spec errata
page](@/docs/reference/errata.md): a type that exists only because 2.1.1 renamed a field, a URL
builder that exists only because one module identifier is addressed through two endpoint variables,
a tariff reading that exists only because a sentence and its own worked example disagree.

Each is expressed as a predicate over the specification — text that must still be there, or must
still be absent — so an erratum upstream **fixes** makes this check fail, naming the entry that
needs re-reading. A documented workaround for a problem that no longer exists is worse than none,
because it is invisible.

### `validate-coverage`

```console
$ cargo run -p xtask -- validate-coverage
728 field(s) across 114 object(s); every one reaches the validator
```

`Validate` is written by hand, one `validate_fields!` call per object. This compares each wire
struct's fields with the ones its impl passes to the validator, so a field added to the struct and
not to the call fails the build rather than accepting a peer's value without ever reporting it.

### `fuzz-corpus`

Seeds `fuzz/corpus/<target>/` from the specification's own examples, pairing a CDR with a Tariff
and a Location with a patch for the two targets that read two documents from one input. See
[How this crate is verified](@/docs/reference/verification.md).

## In CI

`spec-sync --fetch` and `--check` run first, so `spec-coverage`, `enum-coverage`, `field-shapes`,
`endpoints` and `errata` run **for real** on every push rather than being skipped when no checkout
is present. `no-floats`, `dead-config` and `validate-coverage` need no sources at all. `sync-fixtures` is a maintenance task — the
fixtures it produces are committed, so `tests/fixtures.rs` runs everywhere. `spec-sync --latest`
runs weekly, and `fuzz` nightly.
