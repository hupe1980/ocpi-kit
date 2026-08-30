+++
title = "Hub"
weight = 40
description = "Routing, broadcast push, open routing, GET All, and bridging messages between OCPI versions."
+++

Routing, broadcast push, open routing, GET All, and version bridging — the thing that sits between
many CPOs and eMSPs, translating versions and fanning messages out.

## The four routing arrangements

A hub can tell which one it is looking at from the headers and the method alone, which is what
`Forwardable::scenario()` does:

| The `OCPI-to-*` headers say | Method and interface | It is |
|---|---|---|
| another party | any | **Direct** — relay it |
| the hub itself | `GET` on a **Sender** interface | **GetAllViaHub** — merge every party's objects |
| the hub itself | a write on a **Receiver** interface | **BroadcastPush** — fan out to the opposite roles |
| nothing | any | **OpenRoutingRequest** — decide from the content |
| the hub itself | anything else | **refused**, `2001` |

That last row matters. Addressing the hub is the one ambiguous case, and two of its four
combinations are not scenarios at all — a `GET` on a Receiver interface is not a Broadcast Push,
because *"GET SHALL NOT be used in combination with Broadcast Push"*, and a write on a Sender
interface is neither a push to the connected parties nor a read to merge. `scenario()` therefore
returns a `Result`, and the `OcpiError::NotRoutable` it produces carries the advice the
specification itself gives: omit the `OCPI-to-` headers and make it an Open Routing Request.

Guessing the nearest scenario would mean a hub quietly broadcasting a read, which is exactly the
sentence above.

## The rules a hub must not break

* **A new `X-Request-ID`, the same `X-Correlation-ID`.** `RequestIds::forwarded()`.
* **`last_updated` is never touched.** *"When OCPI Objects are sent via Hubs, the `last_updated`
  fields SHALL NOT be updated by the Hub."* Nothing in this module writes it.
* **`GET` is never broadcast.** *"GET SHALL NOT be used in combination with Broadcast Push."*
* **Configuration modules are never routed.** `credentials`, `versions` and `hubclientinfo` are
  platform-to-hub conversations, not traffic.
* **Vendor data survives.** Because every object keeps its unknown fields and every open enum keeps
  values it does not know, a hub built on this crate forwards an extension it has never seen without
  damaging it. That is what OCPI 2.3.0's extensibility chapter asks for, and it is the single most
  common way a hub loses data.

## The pieces

* **`RoutingTable`** — which platforms are connected, what roles they hold, and what version each
  speaks. It produces the hub-specific `4001`/`4002`/`4003` status codes: unknown receiver,
  timeout, connection problem.
* **`Forwarder`** — takes a request and a routing decision and produces the onward request, with
  the header rules already applied.
* **`OpenRouter` / `BodyOwnerRouter`** — for an Open Routing Request, works out the destination from
  the object in the body.
* **`aggregate`** — turns a fan-out's results into the one status code the sender is told, with an
  `AggregatePolicy` for the choice the specification leaves open: surface the first failure (the
  default — the sender learns something went wrong), accept any success, or always succeed because
  the hub owns delivery from here. A broadcast that reached *nobody* is `4003` under every policy.
* **`bridge`** — the classification a crossing between two versions needs, for a hub that wants to
  translate an object itself. `Forwarder` already does it for the traffic it relays.

## Version bridging

A 2.2.1 CPO and a 2.3.0 eMSP talk through a hub built on this crate **without either of them knowing**.
The routing table knows which version each connected platform speaks; `Forwardable` carries the version
its body arrived in. `Forwarder::relay` translates the request on the way out, translates the response
on the way back, and appends what the crossing cost to the `status_message`:

```rust
let relayed = forwarder.relay(&request, &to, routing).await;
let response = relayed.outcome?;

// response.data is written in the requesting party's version.
// response.status_message:
//   "version bridged with 1 loss(es): /help_phone: OCPI 2.2.1 has no Location.help_phone"
```

Three of the four cases cost nothing: the two platforms are on the same version, the request has no
body, or the endpoint carries an object whose wire format did not change between the versions. Only
the fourth decodes and re-encodes.

### When it cannot

A crossing this build has no conversions for — anything involving OCPI 2.1.1, or a version this crate
does not model — is **refused** by default, with a `2001` naming both versions:

```rust
let forwarder = Forwarder::new(transport, &table, hub)
    .on_unbridgeable(Unbridgeable::RelayVerbatim);   // the other choice
```

Refusing is the default because handing a 2.1.1 object to a 2.3.0 party produces a document the
receiver misreads rather than an error it rejects: a 2.1.1 CDR has no owner fields and its
`total_cost` is a bare number, not a `Price`. `RelayVerbatim` is for a hub that is deliberately a
pipe between parties that understand each other by some arrangement outside this crate.

`Forwarder::report_losses(false)` switches off the `status_message` annotation, for a peer that treats
that field as machine-readable.

See [Versions and conversion](@/docs/concepts/versions.md) for what a bridge costs and which crossings
exist.

## Who goes in the headers

The specification gives five tables of who belongs in `OCPI-to-` and `OCPI-from-`. Getting them
wrong fails nothing and surfaces weeks later as a partner's complaint, so `RoutingScenario` encodes
all five: pick a scenario rather than filling headers by hand. The test suite runs a real hub
against real downstream servers and asserts on the headers **they received**, not on what the
forwarder meant to send.

One row the specification does not give is the leg from the hub onward during a GET All; its table
covers only the two legs between the requester and the hub. This crate uses the ordinary relay
headers there (*"Direct request | Hub to receiving platform | Receiving-party | Requesting-party"*),
which also means the answering party can see who actually asked, and authorise accordingly.
