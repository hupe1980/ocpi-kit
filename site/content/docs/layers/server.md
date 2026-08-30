+++
title = "Server"
weight = 30
description = "An axum router driven by one trait per OCPI module and interface, with discovery generated from what you mounted."
+++

An OCPI server: one trait per module and interface, mounted onto an `axum::Router`.

```rust
let app = OcpiRouter::new(VersionNumber::V2_3_0, base_url, tokens)
    .credentials(credentials_handler)
    .locations_sender(locations)
    .cdrs_receiver(cdrs)
    .build();

axum::serve(listener, app).await?;
```

You implement the traits; the router does the protocol.

## What the router takes care of

* **The status code rules.** Only five situations get an HTTP error status; everything that reached
  the OCPI layer is a `200 OK` with a four-digit code in the body. A handler returns `OcpiError`
  and the mapping happens once, correctly.
* **Authentication and the `CREDENTIALS_TOKEN_A` scope.** A bootstrap token used on any module
  other than `credentials` and `versions` gets a 401, as the specification requires.
* **Ownership of client-owned objects.** A platform writing under a `country_code`/`party_id` that
  is not one of its own roles gets a 404 — *"this way blocking client access to objects that do not
  belong to them"* — and your handler is never called.
* **`X-Request-ID` and `X-Correlation-ID`.** Echoed on every response, generated when the peer
  forgot them.
* **Version details.** `/versions` and the version-details endpoint are generated from exactly what
  was mounted, so discovery cannot disagree with reality.
* **The PATCH rule.** A patch without `last_updated` never reaches a handler.

## What it deliberately leaves to you

Persistence, and the two credentials 405 rules — only your implementation knows whether a peer is
already registered. `PeerState` has the predicates.

## Sender and Receiver on one router

This one is worth knowing before it bites you. The Locations **Sender** interface addresses objects
by id:

```text
GET /locations/{location_id}/{evse_uid}/{connector_id}
```

and the **Receiver** interface addresses client-owned objects by party:

```text
PUT /locations/{country_code}/{party_id}/{location_id}
```

Both are three path segments. A platform that is both CPO and eMSP and mounts both interfaces of
the same module on one router has an ambiguous route, and no amount of ordering fixes it.

`ServerConfig::receiver_path_prefix` picks between the two ways out:

* `Some("receiver")` — the default. One router, one `/versions`, and the Receiver interfaces
  published one segment deeper. The generated version details say so, which is the whole point of
  generating them.
* `None` — the conventional split: one `OcpiRouter` per role, each nested under its own base URL.

Mounting both interfaces of a module on one router with no prefix is a configuration error, and it
panics at start-up with an explanation rather than producing a router that misroutes in production.

## Extractors

The request-side vocabulary is available as axum extractors if you want to write handlers by hand:
`Auth` (the authenticated peer), `Ids` (request and correlation), `Routing` (the `OCPI-to-*`
headers), `Page` (the pagination query), `Owner` (the `country_code`/`party_id` path pair),
`OcpiJson` (a validated body) and `OcpiPatch` (a merge patch with its `last_updated` rule already
enforced).

## Testing your server

The `testkit` feature has validated sample objects for every module and in-memory stores with
spec-accurate pagination, including the `X-Total-Count`/`X-Limit`/`Link` headers. This repository's
own end-to-end test drives the `ocpi-kit` server with the `ocpi-kit` client over a real TCP socket,
which is what catches the things unit tests cannot: that the router's paths match the client's URL
builders, and that the headers one side writes are the ones the other side reads.
