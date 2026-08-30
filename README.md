# ocpi-kit

[![CI](https://github.com/hupe1980/ocpi-kit/actions/workflows/ci.yml/badge.svg)](https://github.com/hupe1980/ocpi-kit/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/ocpi-kit.svg)](https://crates.io/crates/ocpi-kit)
[![docs.rs](https://docs.rs/ocpi-kit/badge.svg)](https://docs.rs/ocpi-kit)
[![license](https://img.shields.io/crates/l/ocpi-kit.svg)](#license)

A Rust toolkit for [OCPI](https://evroaming.org/ocpi/) (Open Charge Point Interface), the protocol
that carries EV roaming traffic between Charge Point Operators (CPO), e-Mobility Service Providers
(eMSP) and roaming hubs.

It is not only a set of types. `ocpi-kit` gives you the wire models for **OCPI 2.3.0, 2.2.1 and
2.1.1**, the transport envelope, an async client, an `axum` server, the pieces a **roaming hub**
needs, an **auditable tariff engine**, and a CLI — behind cargo features, in one crate.

```toml
[dependencies]
ocpi-kit = "0.1"
```

## The three properties that decide quality

**Money is never a float.** Every `number` in every object is an exact decimal
(`rust_decimal::Decimal` behind `types::Number`). No public field of any OCPI object in this crate
is an `f32` or `f64`, the pricing engine has none either, and `cargo run -p xtask -- no-floats`
enforces that in CI. Every other Rust OCPI type set models prices as `f64`; a binary float cannot
represent `0.10`, and cents decide real disputes.

**Nothing a peer sent is thrown away.** Undocumented JSON fields land in a `types::Extensions` map
and are written back verbatim. An enum value this crate has never heard of keeps its text in a
`Custom(String)` variant. A hub built on `ocpi-kit` forwards a vendor extension it does not
understand without damaging it — which is what OCPI 2.3.0's extensibility chapter asks for, and the
single most common way real hubs lose data.

**Parsing and conformance are separate questions.** A peer that overruns a `string(45)` cannot make
a whole page of Locations undecodable. The value arrives; `Validate::validate` reports it with an
RFC 6901 JSON Pointer:

```rust
use ocpi_kit::types::Validate;
use ocpi_kit::v2_3_0::locations::Location;

let location: Location = serde_json::from_str(&json)?;      // permissive
for v in location.validate().unwrap_err().iter() {
    println!("{} {:?}: {}", v.pointer, v.code, v.message);  // /evses/0/evse_id TooLong: …
}
```

The rule throughout is: **parse permissively, validate explicitly, construct strictly.**

## What is in the box

| Layer | Feature | What it gives you |
|---|---|---|
| `types` | *(always)* | `CiString`, `DateTime`, `Number`, `Url`, `Extensions`, RFC 6901 validation |
| `v2_3_0` | `v2_3_0` | the OCPI 2.3.0 wire model — 59 objects, all modules |
| `v2_2_1` | `v2_2_1` | the OCPI 2.2.1 wire model — 53 objects, as a delta from 2.3.0 |
| `v2_1_1` | `v2_1_1` | the OCPI 2.1.1 wire model — 33 objects (legacy peers) |
| `bookings` / `invoice-reconciliation` | same | the 2.3.0 `bookings` and `payments` release branches |
| `convert` | `convert` | `Upgrade`/`Downgrade` between versions, with loss accounting |
| `transport` | `transport` | envelope, status codes, headers, credentials tokens, pagination, routing, PATCH |
| `client` | `client` | async client over `reqwest`, registration handshake, paginated crawls |
| `server` | `server` | `axum` router driven by one trait per module and interface |
| `hub` | `hub` | routing table, broadcast push, open routing, GET All, version bridging |
| `tariffs` | `tariffs` | auditable pricing engine over CDRs and Sessions |
| `testkit` | `testkit` | validated samples, in-memory stores with spec-accurate pagination |
| conformance runner | `client` | drive a live peer through the spec's rules, read-only |
| `schema` | `schema` | `JsonSchema` for every wire type |
| `ocpi` CLI | `cli` | `validate`, `versions`, `pull`, `price`, `convert`, `conformance`, `schema` |

Default features are `v2_3_0`, `v2_2_1` and `transport`. `full` turns on everything except the CLI.

## Client

```rust
use ocpi_kit::client::{OcpiClient, Registration};
use ocpi_kit::transport::{CredentialsToken, PageQuery};
use ocpi_kit::types::{PartyRef, Url};
use ocpi_kit::{InterfaceRole, ModuleId};

let client = OcpiClient::new()?;
let me = PartyRef::new("NL", "TNM")?;

// The registration handshake as a typestate: Registration → Discovered → Selected → Peer.
// You cannot call a module endpoint before discovery, or reuse TOKEN_A after registering.
let peer = Registration::new(
        Url::new("https://cpo.example.com/ocpi/versions")?,
        CredentialsToken::new("token-a-received-out-of-band")?,
    )
    .discover(client.transport()).await?
    .select_best(client.transport()).await?;

// Refuse to register with a peer that does not implement what we need — before POSTing.
peer.require(&[(ModuleId::Locations, InterfaceRole::Sender)])?;
let peer = peer.register(client.transport(), &my_credentials()).await?;

// Then pull, following every `Link: rel="next"` header.
let mut locations = peer.locations(client.transport(), me).list(PageQuery::new())?;
while let Some(location) = locations.next().await? {
    println!("{} {}", location.id, location.name.as_deref().unwrap_or(""));
}
```

The client **refuses to call a URL it should not**: `Credentials.url`, `Endpoint.url` and every
`response_url` are attacker-influenced, so a `UrlPolicy` rejects plain HTTP, loopback and private
addresses by default. It **validates what it sends** (`ClientConfig::validate_outgoing`, on by
default). It **only retries `GET`**, because the spec says messages must not be queued and
replayed. And it never logs a token.

## Server

```rust
use ocpi_kit::server::{InMemoryTokenStore, OcpiRouter};
use ocpi_kit::{types::Url, VersionNumber};

let app = OcpiRouter::new(
        VersionNumber::V2_3_0,
        Url::new("https://cpo.example.com/ocpi/cpo/2.3.0")?,
        Arc::new(InMemoryTokenStore::new()),
    )
    .credentials(my_credentials_handler)
    .locations_sender(my_locations)
    .build();

axum::serve(tokio::net::TcpListener::bind("0.0.0.0:8080").await?, app).await?;
```

You implement one trait per module and interface; the router handles the rest:

* **The status-code rules.** Only five situations get an HTTP error status; everything that reached
  the OCPI layer is `200 OK` with a four-digit code in the body.
* **`CREDENTIALS_TOKEN_A` scoping.** A bootstrap token used on any module other than `credentials`
  and `versions` gets a 401.
* **Ownership of client-owned objects.** A platform writing under a `country_code`/`party_id` that
  is not one of its own roles gets a 404, and your handler is never called.
* **`X-Request-ID` / `X-Correlation-ID`**, echoed and generated.
* **`/versions` and version details generated from exactly what was mounted**, so discovery cannot
  disagree with reality.
* **The PATCH rule.** A patch without `last_updated` never reaches a handler.

## Hub

The `hub` feature recognises all four routing arrangements from the headers and method alone
(Direct, Broadcast Push, Open Routing Request, GET All via hub), keeps the rules a hub must not
break (a new `X-Request-ID` with the same `X-Correlation-ID`; `last_updated` never touched; `GET`
never broadcast; configuration modules never routed), and bridges versions through `convert` with
a report of what a translation cost:

```rust
use ocpi_kit::convert::Upgrade;
use ocpi_kit::hub::{bridge, Bridge};

match bridge(&sender_version, &receiver_version) {
    Bridge::Passthrough => forward(cdr),
    Bridge::Upgrade => {
        let converted = cdr.upgrade();
        for loss in &converted.lossy {
            tracing::warn!(%loss.pointer, "did not survive the crossing: {}", loss.reason);
        }
        forward(converted.value)
    }
    Bridge::Downgrade => { /* … */ }
    // Two 3.0 parties can still talk; relaying the bytes unchanged is the operator's call.
    Bridge::Unsupported => relay_verbatim(),
}
```

## Tariffs

OCPI is the only protocol that carries both the tariff and the metering data, so a session's cost
is computable from what crosses the wire. That is how an eMSP checks a CPO's invoice.

```rust
let breakdown = PricingEngine::new().price(&session, &tariffs)?;
assert_eq!(breakdown.total_excl_vat.to_string(), "5.00");
```

The answer is **auditable**: `CostBreakdown` does not just say `12.28`, it says which quantity was
billed for each dimension, what `step_size` did to it, which Tariff Element and which Price
Component priced it, and why that element was selected. The arithmetic is exact. The parts the
specification deliberately leaves open — rounding, and `step_size` itself, which OCPI 3.0 removes —
are settings on `PricingPolicy`, not assumptions baked into the code.

Twelve of the specification's own worked examples are tests.

## Conformance

Drive a live peer through the specification's rules and report where it disagrees.

```console
$ ocpi conformance https://cpo.example.com/ocpi/versions
```

```text
[+] versions.common   the peer offers a version this build speaks
                      2.3.0, 2.2.1
[x] headers.request_id  X-Request-ID is echoed
                      absent from the response
                      spec: 2.3.0 §transport_and_format_request_id
[!] module.xlimit     locations sends an X-Limit header
                      absent, so a client cannot tell whether its limit was reduced

21 passed, 1 failed, 1 warnings, 4 skipped
```

It checks discovery, the endpoint list, header echoing, clock skew, authentication, and one page
from every Sender interface the peer offers — pagination headers, limits, `Link: rel="next"`, and
whether the objects conform. Every check names the specification anchor behind it, so a failing
line pastes straight into a ticket.

It is **read-only** — `GET`s plus two deliberately-unauthenticated requests, never a write — so
it is safe against a production partner. Non-zero exit on failure makes it a CI gate, and
`ocpi_kit::client::Conformance` is the same thing as a library type.

## CLI

```console
$ cargo install ocpi-kit --features cli

$ ocpi validate --as location location.json      # every length limit and cross-field rule
$ ocpi versions https://cpo.example.com/ocpi/versions --token "$OCPI_TOKEN"
$ ocpi pull locations https://cpo.example.com/ocpi/versions  # $OCPI_TOKEN, every page
$ ocpi price cdr.json --tariff tariff.json      # what it should have cost, and why
$ ocpi convert --as cdr --from 2.2.1 --to 2.3.0 cdr.json
$ ocpi conformance https://cpo.example.com/ocpi/versions   # read-only, exits non-zero on failure
$ ocpi schema location --version 2.3.0
```

## Spec traceability

Every public item carries a `Spec: <version> §<anchor>` line naming the AsciiDoc anchor in the OCPI
source it implements, so a reviewer — or a partner's compliance team — can go from a Rust type
straight to the sentence that defines it.

That is checkable, not decorative. Repository automation compares the crate against the
specification:

```console
$ cargo run -p xtask -- spec-coverage --check   # every object's fields vs the spec's property tables
$ cargo run -p xtask -- sync-fixtures           # re-import the spec's own JSON examples
$ cargo run -p xtask -- no-floats               # the no-f64 guarantee
```

`spec-coverage` checks five releases — 2.3.0, 2.2.1, 2.1.1 and the `bookings` and `payments`
branches, 275 object comparisons — and all of them match the property tables exactly.

## How it is verified

* **Field census** against the specification's own property tables, as above.
* **Round-trip of every example the specification ships** — all 218, across four corpora. Each
  needs a recorded expectation, so a newly synced example cannot pass unexamined; where the spec's
  own example is wrong the reason is written down and the test asserts it *still* fails, so an
  upstream fix surfaces as a failure.
* **Property tests** for the laws the rest of the crate relies on: `Eq`/`Hash`/`Ord` agreement on
  case-insensitive identifiers, merge-patch idempotence, exact decimal arithmetic across the JSON
  boundary, conversion round-trips, well-formed JSON Pointers.
* **Mock peers that misbehave.** The end-to-end test proves client and server agree; `wiremock`
  peers prove the client survives partners that don't — an OCPI 3.0-only peer, pagination that
  points at itself forever, a `2003` inside a `200`, a 503 that must not be retried on a write.
* **The conformance runner pointed at our own server**, so the two keep each other honest.

414 tests across seven targets. Clippy at `pedantic` with `-D warnings` under three feature sets,
every feature and every pair of layer features compiled, `cargo deny`, and benchmarks for what the
guarantees cost:

```console
$ cargo bench --bench wire
```

Exact arithmetic is about 9× slower than binary floating point, and a thousand decimal additions
still cost under 2% of decoding the page they arrived on. [The guide has the
numbers](https://hupe1980.github.io/ocpi-kit/docs/reference/verification/#what-the-guarantees-cost).

## Documentation

* [API documentation](https://docs.rs/ocpi-kit) — every item, with the spec anchor it implements
* [The guide](https://hupe1980.github.io/ocpi-kit/docs/) — the protocol in brief, concepts,
  per-layer walkthroughs, [interop notes](https://hupe1980.github.io/ocpi-kit/docs/reference/interop/),
  [how this is verified](https://hupe1980.github.io/ocpi-kit/docs/reference/verification/) and the
  [spec errata](https://hupe1980.github.io/ocpi-kit/docs/reference/errata/)

## Minimum supported Rust version

1.96. Raising it is a minor-version change.

## Contributing

Issues and pull requests are welcome. `cargo test --all-features`, `cargo clippy --all-targets
--all-features` and both `xtask` checks must pass; CI runs them plus a feature-combination sweep,
`cargo deny` and a docs build.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

OCPI is a protocol owned and maintained by the [EVRoaming Foundation](https://evroaming.org/).
This project is not affiliated with the EVRoaming Foundation.
