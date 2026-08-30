+++
title = "How this crate is verified"
weight = 10
description = "Field census, fixture round-trips, property tests, misbehaving mock peers, and what the guarantees cost."
+++

Nine techniques, each catching a class of problem the others cannot.

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

## Enum census against the specification's value tables

`cargo run -p xtask -- enum-coverage --check` does the same thing one level down: the **values** of
every enum, against the tables that define them.

```text
=== OCPI 2.3.0 enums ===   35 enum(s) match exactly
=== OCPI 2.2.1 enums ===   31
=== OCPI 2.1.1 enums ===   20
=== bookings branch ===    38
=== payments branch ===    35
```

A field census compares *names*. It says nothing about what may go in the field, and a missing
enum value is the same defect hidden deeper — on a closed enum a conformant peer's object stops
decoding; on an open one the value survives in `Custom(_)`, fails nothing, and silently never
matches the variant somebody wrote a `match` arm for. A fixture round-trip cannot find it unless
the specification happens to ship an example using that value, which for a 42-value enum it does
not.

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

and eight that assert **panic-freedom** for every parser a peer controls the input of — the
`Authorization` and `Link` headers, the pagination headers, the envelope from arbitrary bytes,
`DateTime`/`Number`/`PartyRef`/`Url` from arbitrary text, RFC 7396 merge over values that are not
objects, and the version bridge over a document that is not the object its endpoint claims. This
crate forbids `unsafe`, so a hostile peer's leverage is not memory corruption: it is a panic, and
one inside a hub's forwarder kills a task holding somebody else's message.

Plus three the pricing engine's output has to satisfy for **any** tariff and **any** session:

* the tax lines account for exactly the difference between the two totals
* the inclusive total is never below the exclusive one, and neither is ever negative
* the whole breakdown round-trips through JSON

The first of those found a defect within seconds of being written, on a code path older than the
audit that added it: tax lines were accumulated at four decimal places while the totals rounded to
two, so they had never quite added up. No assertion on a total would have shown it.

## Snapshots of what a person reads

Round-trip and equality tests prove that a value survives. They say nothing about whether what
comes out is *legible* — and a `Link` header, an error envelope and a priced session are all read
by people: a partner's integration engineer, an on-call responder, a driver disputing an invoice.
Their shape is part of the contract even where the specification does not pin it down.

`tests/snapshots.rs` covers the pagination headers of a middle page and a last page, the crate's
**whole error vocabulary** rendered as the JSON a peer is shown, and two priced sessions: the
specification's own `step_size` worked example in full, and the same session under the OCPI 3.0
policy that has no `step_size`. The second pair is the clearest statement of what block billing
costs a driver.

When one changes, the diff is the review — `cargo insta review`.

This is not a formality. Rendering a `CostBreakdown` in full is what surfaced
`"measured": 0.13333333333333333`: a quantity carrying more significant digits than a JSON number
holds, which this crate's own `Number::json_round_trips` reports as imprecise. No assertion on a
total would ever have shown it, and an audit artefact that does not survive being written down is
not an audit artefact.

## The client against peers that behave badly

`tests/end_to_end.rs` drives this crate's client against its server over a real TCP socket, which
proves the two agree — not that the client survives a peer that *disagrees*.

`tests/mock_peer.rs` covers that: peers each wrong in one specific way, and what the client does
about it. A peer offering only OCPI 3.0, one missing a required module, one wanting an unencoded
token, one whose pagination points at itself forever, one whose result set shrinks mid-crawl, one
answering `2003` inside a `200`, one sending a three-character `country_code`, one failing once with
a 503.

`tests/modules.rs` covers a third thing again: agreement between a **router mount and a URL
builder**. Those are two independent statements about one path, written in different files, and
unit tests on either side pass happily while the two disagree. Every test there drives this
crate's client against this crate's server over a real socket, for the modules whose callback URLs
the specification leaves to the implementation — Charging Profiles, Payments, Hub Client Info, and
the asynchronous half of Commands.

## The hub, against downstream servers that record what they got

The hub is where the specification's five routing tables become code, and where getting
`OCPI-to-`/`OCPI-from-` wrong is invisible until a partner complains.

`tests/hub.rs` therefore runs a real hub against real downstream servers over real sockets and
asserts on **the headers the downstream party actually received**, not on what the forwarder
intended to send. All four scenarios' rows, the new-`X-Request-ID`/same-`X-Correlation-ID` rule,
broadcast reaching opposite roles only and never its own sender, a suspended platform, an
unreachable one, and the `4001`/`4002`/`4003` mapping.

## Two versions on one wire

A library that models several OCPI versions and tests each of them against *itself* has tested
nothing about the crossing. `tests/versions_on_the_wire.rs` asserts on the JSON that actually left
the socket:

* a `Tariff` from a router published as 2.2.1 must **not** carry `tax_included`, and the same
  handler on a 2.3.0 router must;
* a `Location`'s `help_phone` is dropped at the 2.2.1 edge and is still there in the handler's own
  store, so the translation is at the boundary and nowhere else;
* a client whose peer registered as 2.2.1 reads canonical `v2_3_0` objects and writes 2.2.1 ones;
* a PATCH that both versions read the same way goes through, and one that writes `help_phone` is
  refused with the GET → PUT recovery;
* a hub relays between a 2.3.0 requester and a 2.2.1 CPO in both directions, and the loss report
  reaches the requester in the `status_message`;
* a router asked to publish a version this build cannot write refuses to start.

`tests/version_bridging.rs` carries every 2.2.1 spec example to 2.3.0 and back, and checks the one
claim `ObjectKind::divergent_fields` makes — that no top-level field outside its list moves across
the boundary. That list is what decides whether a merge patch may cross, and a patch is the one
document this crate cannot verify by decoding it, so it is verified against the corpus instead.

## The conformance runner, pointed at ourselves

The [conformance runner](@/docs/layers/conformance.md) runs against this crate's own server in the
test suite, and against the [reference peer](@/docs/layers/testkit.md) that `ocpi serve-mock`
serves. Every check it makes is a rule the router is supposed to follow, so the three keep each
other honest.

## Everything else

516 tests across twelve targets, under three feature sets on three platforms.

* Clippy at `pedantic` with `-D warnings`, under three feature sets.
* Every feature, and every pair of layer features, compiled — this is what catches a layer using a
  wire model it never declared a dependency on, invisible while that model is a default feature.
* `cargo deny` over bans, licences, sources and advisories.
* `xtask no-floats` across 64 files, and `xtask dead-config` over every public configuration field.
* The benchmarks compile in CI, so they cannot rot.

```console
cargo test --all-features
cargo bench --bench wire            # what the guarantees cost
cargo run -p xtask -- no-floats
cargo run -p xtask -- spec-coverage --check
cargo run -p xtask -- enum-coverage --check
cargo run -p xtask -- dead-config

# The docs are built the way docs.rs builds them: nightly, with `--cfg docsrs` so the
# `doc(cfg(…))` feature badges are compiled in. Plain `cargo doc` on stable does not see
# them, and so does not see the warnings they can produce.
RUSTDOCFLAGS='-D warnings --cfg docsrs' cargo +nightly doc --no-deps --all-features
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
