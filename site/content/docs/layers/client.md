+++
title = "Client"
weight = 20
description = "An async OCPI client: the credentials handshake as a typestate, typed module clients and paginated crawls."
+++

An async OCPI client over `reqwest`: the registration handshake as a typestate, typed module
clients, and paginated crawls.

## The handshake is a typestate

The credentials handshake is where most OCPI integrations break, and every failure mode is a state
mistake: using `CREDENTIALS_TOKEN_A` after registering, POSTing credentials twice and getting a
405, forgetting to re-fetch the endpoints after a PUT. So the states are types:

```text
Registration  --discover-->  Discovered  --select_best-->  Selected  --register-->  Peer
```

```rust
let peer = Registration::new(versions_url, token_a)
    .discover(client.transport()).await?
    .select_best(client.transport()).await?;

// Everything the peer said it implements is known here, before you commit to anything.
peer.require(&[(ModuleId::Locations, InterfaceRole::Sender)])?;

let peer = peer.register(client.transport(), &my_credentials).await?;
```

You cannot call a module endpoint on a `Registration`, because the endpoints are not known yet.
`PeerState` carries the predicates for the two 405 rules the server side needs, since only the
implementation knows whether a peer is already registered.

## Typed module clients

```rust
let locations = peer.locations(client.transport(), me);
let one = locations.get("LOC1").await?;
let mut all = locations.list(PageQuery::new())?;
```

Sender and Receiver interfaces are separate types — `LocationsSender` and `LocationsReceiver` —
because they are different interfaces with different URL shapes and different methods, and mixing
them up is a class of bug worth removing.

Every one takes and returns `v2_3_0` objects, **whatever version the peer speaks**: a 2.2.1 CPO
answers `GET {locations}` with 2.2.1 Locations and what arrives here is a
`v2_3_0::locations::Location`; a `PUT` goes back out as 2.2.1. Load-bearing rather than convenient —
2.3.0 made `Tariff.tax_included` required and 2.2.1 does not have it, so without the translation a
typed pull from most peers in the field fails on the first object. The handshake is bridged too.

A field that cannot cross down logs a `tracing` warning with its JSON Pointer; a `PATCH` writing a
field the versions disagree about is refused before it is sent, with the spec's GET → PUT recovery.
See [Versions and conversion](@/docs/concepts/versions.md).

`ModuleClient`'s own `get`/`put`/`post`/`patch`/`list` are the escape hatch: they decode exactly the
type you name and translate nothing.

There is a typed client for every module the crate models a protocol for:

| | |
|---|---|
| `peer.locations(…)` / `peer.locations_receiver(…)` | pull Locations, push Locations |
| `peer.sessions(…)` / `peer.sessions_receiver(…)` | pull Sessions and set charging preferences, push Sessions |
| `peer.cdrs(…)` | pull CDRs, and `POST` one (the module returns its URL in a `Location` header) |
| `peer.tariffs(…)` / `peer.tariffs_receiver(…)` | pull Tariffs, push and `DELETE` them |
| `peer.tokens(…)` / `peer.tokens_receiver(…)` | pull Tokens and authorize in real time, push Tokens |
| `peer.commands(…)` | send a command, and await its result at a URL you serve |
| `peer.charging_profiles(…)` | set, read and clear a profile on a session |
| `peer.hub_client_info(…)` | who a hub says is connected |
| `peer.payments(…)` | terminals and financial advice confirmations, from either side. Needs the `payments` feature: Payments is a 2.3.0 release branch, not core |

Anything else — the `bookings` and `invoicereconciliation` extension modules, or a peer's vendor
module — goes through `peer.module(…)`, the untyped `ModuleClient`, which still gives you the
envelope, the routing headers, the retry rule and the URL policy.

Each of these calls a `Peer`'s **discovered** endpoints. A module the peer never advertised is an
error before a request is made, rather than a 404 from a URL this crate invented.

## Crawling

`PageStream` follows every `Link: rel="next"` to the end:

```rust
let mut stream = locations.list(PageQuery::new())?;
while let Some(location) = stream.next().await? { /* … */ }
```

It respects `DEFAULT_MAX_PAGES`, so a peer whose every page links to itself cannot spin the crawl
forever — a real and recurring interop failure.

It also applies the concurrency correction the specification asks for:

> *When there are for example 1000 objects matching a query … while crawling over the pages one of
> these objects is updated. The client detects this: `X-Total-Count` will be lower in the next
> request. It is advised to redo the previous GET with the `offset` lowered by 1 (if the `offset`
> was not 0) and after that continue crawling the 'next' page links.*

The GET to redo is the one that **noticed** the drop, and its objects are discarded: an object
before the crawl's window is gone, so everything after it slid down by one and the object now at
`offset - 1` would otherwise be skipped. Redoing the *previous* page instead — the other reading of
that sentence — re-emits a whole page the caller has already been handed. `PageStream::corrections()`
reports how many times it happened, so a pull over a result set that keeps shifting is visible rather
than silent.

## Retries

Only `GET` is retried, because the specification says so:

> *OCPI messages SHOULD NOT be queued. When a client does a POST, PUT or PATCH request and that
> request fails or times out, the client should not queue the message and retry the same message
> again later.*

The delay is exponential with **equal jitter** — drawn from the upper half of the interval — and
the draw is seeded from the request's own `X-Request-ID`, which is a fresh UUID. That detail is
the point of jitter: a schedule computed from the attempt number alone is identical on every
client in a fleet, so a peer that has just come back from an outage is hit by all of them at the
same instant. Jitter that every client computes the same way is not jitter.

## Getting back in sync after an outage

The specification is explicit that you must **not** queue and replay:

> When a client does a POST, PUT or PATCH request and that request fails or times out, the client
> should not queue the message and retry the same message again later. When the connection is
> re-established, it is up to the target-server of a connection to GET the current status from the
> source-server.

`Resync` builds that pull. It overlaps the window backwards by 15 minutes by default, because a
peer's clock is not yours and an object can be written a moment before its `last_updated` is read.
It also computes a **splayed** poll interval — the spec asks clients to think in hours, not
minutes, and to randomise, so a fleet of clients does not stampede a partner on the hour.

```rust
let plan = Resync::new().plan(last_success, now);
let page = locations.list(plan.query)?;
// … then sleep plan.next_poll_after
```

The splay is derived from a seed rather than an RNG, so a given client polls at a stable,
uncorrelated offset and the behaviour is reproducible in tests.

## What the client does that a hand-rolled one usually does not

* **It refuses to call a URL it should not.** `Credentials.url`, `Endpoint.url` and every
  `response_url` are attacker-influenced inputs. A `UrlPolicy` rejects plain HTTP, loopback and
  private addresses by default; a client that fetches them unconditionally is an SSRF proxy.
* **It validates what it sends.** On by default, so a non-conformant object is caught here rather
  than at the partner's support desk.
* **It only retries `GET`.** Never a POST, PUT or PATCH, for the reason above.
* **It never logs the token.** Spans carry the request and correlation IDs and the routing parties;
  the token redacts itself in any case.

## Checking a partner

The `client` feature also carries the [conformance runner](@/docs/layers/conformance.md): point it at a peer
and it reports, check by check with the spec anchor for each, where that peer disagrees with the
specification. Read-only, so it is safe against production.
