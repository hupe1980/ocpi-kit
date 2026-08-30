+++
title = "How this crate is verified"
weight = 10
description = "Field census, fixture round-trips, property tests, misbehaving mock peers, and what the guarantees cost."
+++

Five techniques, each catching a class of problem the others cannot.

## Field census against the specification's tables

`cargo run -p xtask -- spec-coverage --check` parses the `Property | Type | Card. | Description`
tables out of the AsciiDoc and the field lists out of the Rust source, per release, and reports
anything missing, extra or renamed.

```text
=== OCPI 2.3.0 ===                 59 object(s) match exactly
=== OCPI 2.2.1 ===                 53 object(s) match exactly
=== OCPI 2.1.1 ===                 33 object(s) match exactly
=== OCPI 2.3.0 bookings branch === 70 object(s) match exactly
=== OCPI 2.3.0 payments branch === 60 object(s) match exactly
```

A field one release adds to an object another release also defines is declared in the tool's
`BRANCH_ONLY_FIELDS` table, so it is checked where it belongs and ignored where it does not.

## Round-trip of every example the specification ships

`tests/fixtures.rs` decodes each of the 218 JSON examples into the type the specification says it
is, validates it, re-encodes it, and asserts *canonical* equality with the source — same keys,
same values, with only key order and number formatting normalised.

| Corpus | Examples |
|---|---|
| 2.3.0 | 72 |
| 2.2.1 | 59 |
| 2.1.1 | 18 (extracted from the inline Markdown) |
| 2.3.0 bookings branch | 69 |

Every file must have a recorded expectation, so a newly synced example cannot pass unexamined:

* **`Ok`** — decodes, validates and round-trips.
* **`Erratum`** — the specification's own example is wrong and cannot decode, with the reason
  written down. The test asserts it still *fails*, so an upstream fix surfaces as a failure.
* **`Tolerated`** — decodes and validates, but does not round-trip byte-for-byte because the
  example is written in a way this crate accepts and normalises. The 2.1.1 CDR writes `price` as
  the JSON string `"2.00"`; it is parsed exactly and emitted unquoted.

See [Spec errata](@/docs/reference/errata.md).

## Property tests

`tests/properties.rs` proves the laws the rest of the crate relies on, for every input rather than
for the ones somebody wrote an example of:

* `CiString`'s `Eq`, `Hash` and `Ord` agree — the law every `HashMap` keyed on an identifier needs
* a length limit is reported, never enforced on ingest, and counts characters rather than bytes
* every realistic decimal survives the JSON boundary exactly, and integers stay integers
* decimal addition is order-independent
* a timestamp round-trips and its text form is idempotent
* a merge patch is idempotent, `null` removes exactly one key, and a patch without `last_updated`
  never applies
* a page query survives the URL, and clamping a limit only ever lowers it
* a `Price` crossing to 2.3.0 and back is unchanged, and every reported loss names a field and a
  reason
* every violation carries a well-formed RFC 6901 pointer

## The client against peers that behave badly

`tests/end_to_end.rs` drives this crate's client against its server over a real TCP socket, which
proves the two agree — not that the client survives a peer that *disagrees*.

`tests/mock_peer.rs` covers that: peers each wrong in one specific way, and what the client does
about it. A peer offering only OCPI 3.0, one missing a required module, one wanting an unencoded
token, one whose pagination points at itself forever, one answering `2003` inside a `200`, one
sending a three-character `country_code`, one failing once with a 503.

## The conformance runner, pointed at ourselves

The [conformance runner](@/docs/layers/conformance.md) runs against this crate's own server in the
test suite. Every check it makes is a rule the router is supposed to follow, so the two keep each
other honest.

## Everything else

414 tests across seven targets, under three feature sets on three platforms. Clippy at `pedantic`
with `-D warnings` under three feature sets; every feature and every pair of layer features
compiled. `cargo deny` over bans, licences, sources and advisories. `cargo run -p xtask --
no-floats` enforces the no-`f64` guarantee across 63 files. The benchmarks compile in CI so they
cannot rot.

```console
cargo test --all-features
cargo bench --bench wire            # what the guarantees cost
cargo run -p xtask -- no-floats
cargo run -p xtask -- spec-coverage --check
```

## What the guarantees cost

Every `number` is a `Decimal` rather than an `f64`, and every object carries a `#[serde(flatten)]`
map so unknown fields survive. Both have a price; `cargo bench --bench wire` measures it.

Indicative figures from one developer machine — the ratios are the point:

| | |
|---|---|
| decode one Location (2.5 KB) | 3.7 µs — about 650 MiB/s |
| encode one Location | 2.3 µs |
| decode a page of 200 Locations | 235 µs |
| validate a page of 200 Locations | 182 µs |
| parse / write one `Number` | 58 ns / 70 ns |
| **sum 1000 `Decimal`s** | **4.4 µs** |
| **sum 1000 `f64`s** | **0.46 µs** |
| apply a merge patch to a Location | 12.7 µs |
| price one session against a complex tariff | 0.8 µs |

Exact arithmetic is about **9× slower** than binary floating point, and a thousand decimal
additions still cost under 2% of decoding the page they arrived on. The arithmetic is not where
the time goes.

`from_slice` is *slower* than `from_str` here, not faster: the input is already valid UTF-8, so
`from_str` skips a check `from_slice` must perform. If your bytes come off a socket, measure.
