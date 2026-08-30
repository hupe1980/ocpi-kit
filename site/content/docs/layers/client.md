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

## Crawling

`PageStream` follows every `Link: rel="next"` to the end:

```rust
let mut stream = locations.list(PageQuery::new())?;
while let Some(location) = stream.next().await? { /* … */ }
```

It respects `DEFAULT_MAX_PAGES` so a misbehaving peer cannot spin forever, and it applies
`crawl_adjustment` when the server caps your page size.

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
