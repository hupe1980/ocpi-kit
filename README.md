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

## The four properties that decide quality

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

**One model, whatever the peer speaks.** Your handlers, your client calls and your hub are written
against OCPI 2.3.0. The wire speaks whatever the other party runs — which, in 2026, is usually 2.2.1.
`ocpi-kit` translates at the edge and tells you what the crossing cost:

```rust
// The peer is on 2.2.1. What comes back is a v2_3_0::tariffs::Tariff.
let mut tariffs = peer.tariffs(client.transport(), me).list(PageQuery::new())?;
while let Some(tariff) = tariffs.next().await? { /* tariff.tax_included is TaxIncluded::No */ }
```

A 2.2.1 `Tariff` has no `tax_included`, which 2.3.0 made **required** — so a library that models both
versions and translates neither cannot decode most of the market. Every other OCPI library, in every
language, leaves that translation to you.

## What is in the box

| Layer | Feature | What it gives you |
|---|---|---|
| `types` | *(always)* | `CiString`, `DateTime`, `Number`, `Url`, `Extensions`, RFC 6901 validation |
| `v2_3_0` | `v2_3_0` | the OCPI 2.3.0 wire model — 59 objects, all modules |
| `v2_2_1` | `v2_2_1` | the OCPI 2.2.1 wire model — 53 objects, as a delta from 2.3.0 |
| `v2_1_1` | `v2_1_1` | the OCPI 2.1.1 wire model — 33 objects (legacy peers) |
| `bookings` / `invoice-reconciliation` | same | the 2.3.0 `bookings` and `payments` release branches |
| `convert` | `convert` | `Upgrade`/`Downgrade` between versions, with loss accounting — and the JSON-level bridge the client, server and hub use |
| `transport` | `transport` | envelope, status codes, headers, credentials tokens, pagination, routing, PATCH |
| `client` | `client` | async client over `reqwest`, registration handshake, paginated crawls, a typed client per module — canonical objects whatever the peer runs |
| `server` | `server` | `axum` router driven by one trait per module and interface; publish 2.2.1 and 2.3.0 from one set of handlers |
| `hub` | `hub` | routing table, broadcast push, open routing, GET All, version bridging |
| `tariffs` | `tariffs` | auditable pricing engine over CDRs and Sessions |
| `testkit` | `testkit` | validated samples, in-memory stores with spec-accurate pagination, and `MockPeer` — a complete, conformant OCPI party to point a partner at |
| conformance runner | `client` | drive a live peer through the spec's rules, read-only |
| `schema` | `schema` | `JsonSchema` for every wire type |
| `ocpi` CLI | `cli` | `validate`, `versions`, `pull`, `price`, `convert`, `conformance`, `serve-mock`, `schema` |

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
addresses by default — with an [explicit note on what URL inspection alone cannot
do](https://hupe1980.github.io/ocpi-kit/docs/reference/security/), because a policy that oversells
itself is worse than none. It **validates what it sends** (`ClientConfig::validate_outgoing`, on by
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

One trait per module and interface — Locations, Sessions, CDRs, Tariffs, Tokens, Commands,
Charging Profiles, Payments, Hub Client Info and Credentials — and the router handles the rest:

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
* **The version it publishes.** A router is built for one OCPI version, because that version is part
  of its base URL — `/ocpi/cpo/2.3.0` and `/ocpi/cpo/2.2.1` are different endpoints with
  independently discovered URLs. The handlers behind them are the same: they are written against
  `v2_3_0`, and a router published as 2.2.1 upgrades each request body and downgrades each response
  at the edge. Serving both versions is mounting the same handlers twice, not writing the modules
  twice. A version this build cannot write is refused at start-up rather than answered wrongly.
* **The callback URLs OCPI leaves to you.** The asynchronous halves of Commands and Charging
  Profiles are posted to a `response_url` whose shape the spec explicitly does not define. For
  Charging Profiles that is load-bearing rather than merely open: `ChargingProfileResult` and
  `ClearProfileResult` are the same JSON object, so nothing but the URL can tell a rejected `PUT`
  from a rejected `DELETE`. The router mounts one path per result kind and `server::CallbackUrls`
  builds the matching URLs, so the two cannot drift apart.

## Hub

The `hub` feature recognises all four routing arrangements from the headers and method alone
(Direct, Broadcast Push, Open Routing Request, GET All via hub) — and **refuses the two
combinations that are not arrangements at all**, rather than picking the nearest one, because a
`GET` addressed to the hub silently becoming a Broadcast Push is the thing the spec forbids in the
same sentence. It keeps the rules a hub must not break (a new `X-Request-ID` with the same
`X-Correlation-ID`; `last_updated` never touched; `GET` never broadcast; configuration modules
never routed).

And it **translates between versions in flight**. The routing table knows what each connected
platform speaks; `Forwardable` carries the version its body arrived in. The forwarder converts the
request on the way out, converts the response on the way back, and appends what the crossing cost to
the `status_message`, so the requesting party can see it:

```rust
let relayed = forwarder.relay(&request, &to, routing).await;   // 2.3.0 eMSP ⇄ 2.2.1 CPO
let response = relayed.outcome?;
// response.data is in the requester's version.
// response.status_message: "version bridged with 1 loss(es): /help_phone: OCPI 2.2.1 has no …"
```

A crossing with no conversions — anything involving 2.1.1 — is **refused** rather than relayed:
handing a 2.1.1 object to a 2.3.0 party produces a document the receiver misreads rather than
rejects. `Forwarder::on_unbridgeable(Unbridgeable::RelayVerbatim)` is there for a hub that is
deliberately a pipe.

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

It also **audits the CDR it is pricing.** A Charging Period is a total, not a curve, so a period
that outlasts the price component pricing it cannot be apportioned after the fact — and the
specification puts the obligation on the CPO instead: *"A CPO SHALL at least start (and add) a
ChargingPeriod every moment/event that has relevance for the total costs of a CDR."* Every
implementation assumes that and prices the period at the rate that applied when it began. This one
does too, and then **says so**:

```console
$ ocpi price cdr.json --tariff tariff.json
ENERGY                10 billed (        10 measured)  =        2.0 excl. VAT,          0 VAT
TOTAL                2.0 excl. VAT,        2.0 incl. VAT

[period_spans_price_change] the ENERGY Charging Period starting here outlasts the Price Component
that prices it: element 1 applies at the start and element 0 by the time the period ends. …

the CDR's own total agrees
error: the CDR did not reconcile; see the breakdown above
```

The CDR's own total agrees — and the CDR is still malformed. Exit code 1, so the invoice check is
a pipeline step rather than something somebody reads. Notes carry a machine-readable code, so a
reconciliation run can count them rather than grep them.

And the breakdown holds together as a document: the tax lines always sum to exactly
`total_incl - total_excl`, including when a `min_price` or `max_price` moved the total, which is a
property test rather than a hope.

Ten of the specification's own worked examples are tests, and two more are snapshots — the
`step_size` example rendered in full, next to the same session under the OCPI 3.0 policy that has
no `step_size`, which is the clearest statement of what block billing costs a driver.

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

Two checks are the reason to run it at all: whether the peer actually applies `offset`, and whether
it applies `date_from`. Neither is visible in any single response, and both are expensive — one
makes a crawl loop, the other turns every incremental pull into a full one.

It is **read-only** — `GET`s plus two deliberately-unauthenticated requests, never a write — so
it is safe against a production partner. Non-zero exit on failure makes it a CI gate, and
`ocpi_kit::client::Conformance` is the same thing as a library type.

## CLI

```console
$ cargo install ocpi-kit --features cli

$ ocpi validate --as location location.json      # every length limit and cross-field rule
$ ocpi versions https://cpo.example.com/ocpi/versions --token "$OCPI_TOKEN"
$ ocpi pull locations https://cpo.example.com/ocpi/versions  # $OCPI_TOKEN, every page
$ ocpi pull payment-terminals https://ptp.example.com/ocpi/versions
$ ocpi price cdr.json --tariff tariff.json      # what it should have cost, and why
$ ocpi convert --as cdr --from 2.2.1 --to 2.3.0 cdr.json
$ ocpi conformance https://cpo.example.com/ocpi/versions   # read-only, exits non-zero on failure
$ ocpi serve-mock                                # a conformant peer on :8080, for a partner to hit
$ ocpi schema location --version 2.3.0
```

`serve-mock` gives a partner something to point a half-written client at: an endpoint that
paginates, applies `date_from`, refuses a write under the wrong party, answers `2004` for an unknown
token and rejects a `PATCH` with no `last_updated`. It runs `testkit::MockPeer`, which the test suite
holds to this crate's own conformance runner. `--version 2.2.1` serves 2.2.1 bytes from the same
handlers.

## Spec traceability

Every public item carries a `Spec: <version> §<anchor>` line naming the AsciiDoc anchor in the OCPI
source it implements, so a reviewer — or a partner's compliance team — can go from a Rust type
straight to the sentence that defines it.

That is checkable, not decorative. Repository automation compares the crate against the
specification:

```console
$ cargo run -p xtask -- spec-coverage --check   # every object's fields vs the spec's property tables
$ cargo run -p xtask -- enum-coverage --check   # every enum's values vs the spec's value tables
$ cargo run -p xtask -- sync-fixtures           # re-import the spec's own JSON examples
$ cargo run -p xtask -- no-floats               # the no-f64 guarantee
$ cargo run -p xtask -- dead-config             # every setting does something
```

`dead-config` fails the build if a public field of `Quirks`, `ClientConfig`, `ServerConfig`,
`PricingPolicy` or `UrlPolicy` is only ever *assigned* and never read, or shares a name with a field
on another of them. A configuration field that does nothing is worse than a missing feature:
somebody sets it, believes the problem is handled, and ships.

`spec-coverage` checks five releases — 2.3.0, 2.2.1, 2.1.1 and the `bookings` and `payments`
branches, 275 object comparisons — and all of them match the property tables exactly.
`enum-coverage` checks the same five releases' 159 enums, and all of them match their value tables
exactly.

## How it is verified

* **Field census** against the specification's own property tables, as above.
* **Enum census** against the specification's own *value* tables. A field census says nothing about
  what may go in the field: a missing enum value stops a conformant peer's object decoding on a
  closed enum, and on an open one it survives in `Custom(_)`, fails nothing, and silently never
  matches the variant you wrote a `match` arm for.
* **Round-trip of every example the specification ships** — all 218, across four corpora. Each
  needs a recorded expectation, so a newly synced example cannot pass unexamined; where the spec's
  own example is wrong the reason is written down and the test asserts it *still* fails, so an
  upstream fix surfaces as a failure.
* **Property tests** for the laws the rest of the crate relies on: `Eq`/`Hash`/`Ord` agreement on
  case-insensitive identifiers, merge-patch idempotence, exact decimal arithmetic across the JSON
  boundary, conversion round-trips, well-formed JSON Pointers — and **panic-freedom** for every
  parser a peer controls the input of. There is no `unsafe` here, so a hostile peer's leverage is a
  panic; one inside a hub's forwarder kills a task holding somebody else's message.
* **Mock peers that misbehave.** The end-to-end test proves client and server agree; `wiremock`
  peers prove the client survives partners that don't — an OCPI 3.0-only peer, pagination that
  points at itself forever, a `2003` inside a `200`, a 503 that must not be retried on a write.
* **One set of handlers served as two versions**, asserting on the JSON that actually crossed the
  socket: a `Tariff` that must not carry `tax_included` on a 2.2.1 router, a `Location` whose
  `help_phone` is dropped at the edge and nowhere else, a client on a 2.2.1 peer, and a hub relaying
  between the two. A version test using the crate's own types on both sides of the wire tests
  nothing.
* **A real hub relaying to real downstream servers**, asserting on the `OCPI-to-`/`OCPI-from-`
  headers the downstream party actually *received* — not on what the forwarder meant to send.
* **The reference peer** `ocpi serve-mock` runs, held to the conformance runner and to every typed
  client call, with five modules mounted on both interfaces at once.
* **Snapshots of the artefacts a person reads** — pagination headers, the whole error vocabulary
  as the JSON a peer is shown, a priced session in full. Round-trip tests prove a value survives;
  they say nothing about whether what comes out is legible.
* **The conformance runner pointed at our own server**, so the two keep each other honest.

516 tests across twelve targets. Clippy at `pedantic` with `-D warnings` under three feature sets,
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
  [how this is verified](https://hupe1980.github.io/ocpi-kit/docs/reference/verification/), the
  [spec errata](https://hupe1980.github.io/ocpi-kit/docs/reference/errata/) and the
  [design decisions](https://hupe1980.github.io/ocpi-kit/docs/reference/decisions/) behind the crate

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
