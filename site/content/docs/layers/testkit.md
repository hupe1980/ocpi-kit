+++
title = "Testkit"
weight = 70
description = "Sample objects, in-memory stores, and a conformant OCPI party you can point a partner's client at."
+++

The `testkit` feature is the scaffolding every OCPI integration builds and nobody wants to
maintain: conformant sample objects, stores that paginate the way the specification says, and a
complete party you can run.

It is deliberately dependency-light — no HTTP mocking framework, no test runner — so it works from
a unit test, an integration test, a fuzz target or a demo binary.

## Sample objects

```rust
use ocpi_kit::testkit::sample;

let location = sample::location("LOC1")?;
let tariff = sample::tariff("T1", "0.25")?;
```

Every one validates and round-trips through JSON, asserted by a test — a sample that quietly
stopped conforming would make every test built on it meaningless.

## In-memory stores

```rust
use ocpi_kit::testkit::InMemoryLocations;

let store = InMemoryLocations::with_page_size(10);
store.put(sample::location("LOC1")?);
let page = store.page(&query, &base);   // Link, X-Total-Count and X-Limit already right
```

`page()` is the part worth having: `offset`, `limit`, `date_from` **inclusive**, `date_to`
**exclusive**, `X-Total-Count` over the filtered set rather than the returned page, and a
`Link: rel="next"` carrying the original filters forward. Each is a rule the
[conformance runner](@/docs/layers/conformance.md) checks a real peer for.

## `MockPeer` — the other side of the socket

```rust
use ocpi_kit::server::OcpiRouter;
use ocpi_kit::testkit::MockPeer;

let peer = MockPeer::cpo(base.clone()).seeded();
let app = peer.mount(OcpiRouter::new(VersionNumber::V2_3_0, base, peer.token_store())).build();
```

A complete OCPI party over those stores: both interfaces of Locations, Sessions, CDRs, Tariffs and
Tokens, plus `credentials`. It is what [`ocpi serve-mock`](@/docs/getting-started/cli.md) runs.

Writing this is otherwise the first day of every OCPI project, and the same day for everybody — a
dozen handler traits over a `HashMap`, each an opportunity to get pagination or the ownership rule
slightly wrong and then test against your own mistake.

**Conformant, and checked rather than claimed.** The test suite runs this crate's own
[conformance runner](@/docs/layers/conformance.md) against it and requires a clean report: real-time
authorization answers `2004` for an unknown token, a `PUT` under someone else's party is a `404`, a
`PATCH` without `last_updated` never reaches a handler, a re-POSTed CDR is refused, and
`offset`/`date_from` are applied.

The stores are reachable through it, so a test can seed and inspect:

```rust
peer.locations.put(sample::location("LOC2")?);
assert_eq!(peer.cdrs.len(), 1);
```

`MockPeer::msp(base)` gives the other role. `token_store()` returns a store that accepts
`test_token("c")` as the *opposite* party, so the router's ownership check behaves the way it would
in a real deployment rather than waving everything through.

### What it is not

Not a charge point. Commands, Charging Profiles and Payments are not mounted on purpose: a mock that
answered `ACCEPTED` and never called the `response_url` back would teach a client the wrong lesson,
and one that did call back would need a Charge Point to have an opinion. Version discovery
advertises exactly what is mounted.

### It speaks whichever version you publish

```rust
let app = peer.mount(OcpiRouter::new(VersionNumber::V2_2_1, base, peer.token_store())).build();
```

The handlers are written once, against the canonical 2.3.0 model; a 2.2.1 router answers 2.2.1 bytes,
including a `Tariff` with no `tax_included`. See
[Versions and conversion](@/docs/concepts/versions.md).

## Test parties and tokens

`test_cpo()` is `NL/TNM`, `test_msp()` is `DE/ABC`, `test_hub()` is `NL/HUB`, and `test_token("c")`
is the literal `test-token-c`. Recognisable constants on purpose: one that leaks into a log or a
fixture is obvious at a glance.
