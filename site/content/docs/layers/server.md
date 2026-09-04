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

There is a trait — usually two, one per interface — for every module the crate models a protocol
for: Credentials, Locations, Sessions, CDRs, Tariffs, Tokens, Commands, Charging Profiles,
Payments (with the `payments` feature) and Hub Client Info. `/versions` and the version-details
endpoint are generated, not
implemented.

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
* **The version it publishes.** See below.
* **The callback URLs the specification leaves open.** See further below.

## Serving 2.2.1 from 2.3.0 handlers

A router is built for one OCPI version because that version is part of its base URL:
`/ocpi/cpo/2.3.0` and `/ocpi/cpo/2.2.1` are different endpoints with independently discovered URLs.
The handlers are the same objects — every trait in `server` speaks
[`v2_3_0`](@/docs/concepts/versions.md), and a 2.2.1 router upgrades each request body and downgrades
each response `data` at the edge:

```rust
let cpo = MyCpo::new(store);            // implements LocationsSender, TariffsSender, …

let modern = OcpiRouter::new(VersionNumber::V2_3_0, base.join("2.3.0"), tokens.clone())
    .locations_sender(cpo.clone())
    .tariffs_sender(cpo.clone())
    .build();

let legacy = OcpiRouter::new(VersionNumber::V2_2_1, base.join("2.2.1"), tokens)
    .locations_sender(cpo.clone())      // the same handler
    .tariffs_sender(cpo)
    .build();

let app = Router::new().nest("/ocpi/cpo/2.3.0", modern).nest("/ocpi/cpo/2.2.1", legacy);
```

Serving both versions is mounting the same handlers twice, not writing the modules twice. A `Tariff`
served to a 2.2.1 partner must **not** carry `tax_included`, which 2.3.0 made required — a router
that only *labelled* itself 2.2.1 would be sending a document that version does not define.

The middleware is installed only when there is a translation to make, so a canonical router pays
nothing. A PATCH writing a field the versions disagree about is refused with the specification's
GET → PUT recovery rather than misapplied, and `build()` panics on a version this build cannot
write — today, anything but 2.3.0 or 2.2.1.

## Asynchronous results, and the URL that has to carry the distinction

Commands and Charging Profiles answer twice: the method returns the Receiver's own immediate
verdict, and — if that was `ACCEPTED` — the Charge Point's eventual answer arrives later as a POST
to a `response_url` the Sender chose. The specification is explicit that the shape of that URL is
yours:

> *No structure defined. This is open to the eMSP to define, the URL is provided to the Receiver by
> the Sender.*

For Commands that is merely open. For Charging Profiles it is **load-bearing**, because the three
result bodies are not distinguishable from one another: `ChargingProfileResult` and
`ClearProfileResult` are both `{"result": …}` and nothing else. An endpoint that sniffed the body
could not tell a rejected `PUT` from a rejected `DELETE`.

So `charging_profiles_sender` mounts one path per result kind, and `CallbackUrls` builds the
matching URLs:

```rust
let callbacks = CallbackUrls::new(my_base_url);

// Reaches ChargingProfilesSender::clear_profile_result with unique_id = "req-3".
let response_url = callbacks.clear_profile_result("req-3");

// Reaches CommandsSender::command_result — the shape the spec's own example uses,
// `.../commands/RESERVE_NOW/1234`.
let response_url = callbacks.command_result("RESERVE_NOW", &request_id);
```

Nothing forces you to use these; a Sender that publishes its own URLs and routes them itself is
perfectly conformant. What the pair buys you is that the mount and the URL cannot drift apart —
and a result posted into a 404 is a bug you find minutes later, from a Charge Point, in
production.

Generate the unique id unguessably. See [Security](@/docs/reference/security.md).

## What it deliberately leaves to you

Persistence, and the two credentials 405 rules — only your implementation knows whether a peer is
already registered. `PeerState` has the predicates.

## Sender and Receiver on one router

The Locations **Sender** interface addresses objects by id:

```text
GET /locations/{location_id}/{evse_uid}/{connector_id}
```

and the **Receiver** interface addresses client-owned objects by party:

```text
PUT /locations/{country_code}/{party_id}/{location_id}
```

Both are three path segments. A platform that is both CPO and eMSP and mounts both interfaces of
the same module on one router has an ambiguous route, and no amount of ordering fixes it. Locations
is the clearest case; Charging Profiles (`{session_id}` on both sides) and Payments
(`terminals/{terminal_id}` on both sides) have the same problem.

`ServerConfig::receiver_path_prefix` picks between the two ways out:

* `Some("receiver")` — the default. One router, one `/versions`, and the Receiver interfaces
  published one segment deeper. The generated version details say so, which is the whole point of
  generating them.
* `None` — the conventional split: one `OcpiRouter` per role, each nested under its own base URL.
  Set it with `ServerConfig::default().one_router_per_role()`; `with_receiver_path_prefix`,
  `with_max_page_limit` and `with_quirks` are there too.

Mounting both interfaces of one of those modules on a prefix-less router is a configuration error,
and it panics at start-up with an explanation — in either mount order — rather than producing a
router that misroutes in production.

## Extractors

The request-side vocabulary is available as axum extractors if you want to write handlers by hand:
`Auth` (the authenticated peer), `Ids` (request and correlation), `Routing` (the `OCPI-to-*`
headers), `Page` (the pagination query), `Owner` (the `country_code`/`party_id` path pair),
`OcpiJson` (a validated body) and `OcpiPatch` (a merge patch with its `last_updated` rule already
enforced).

Note that `OcpiPatch` is for `PATCH` specifically. Payments' `POST .../terminals/activate` carries
a partial object too — *"the terminal_id is optional in the activation request"* — but nothing is
being merged into anything there, so the rule that a patch must carry `last_updated` does not
apply, and the router decodes it separately. The handler receives a `Patch<Terminal>` because that
is this crate's type for "an object with fields left out"; read it with `as_value`, do not call
`apply`.

## Testing your server

The [testkit](@/docs/layers/testkit.md) gives you validated sample objects, in-memory stores with
spec-accurate pagination, and `MockPeer` to point your client at.

Drive your server with the `ocpi-kit` client over a real socket, as this repository's own tests do.
A router mount and a URL builder are two independent statements about one path, in different files;
unit tests on either side pass happily while the two disagree.
