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

| The `OCPI-to-*` headers say | Method | It is |
|---|---|---|
| another party | any | **Direct** — relay it |
| the hub itself | `GET` on a Sender interface | **GetAllViaHub** — merge every party's objects |
| the hub itself | anything else | **BroadcastPush** — fan out to the opposite roles |
| nothing | any | **OpenRoutingRequest** — decide from the content |

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
* **`aggregate`** — merges the pages from every connected party for a GET All, with an
  `AggregatePolicy` for what to do when one of them fails.
* **`bridge`** — what a message crossing between two versions needs.

## Version bridging

A 2.2.1 CPO and a 2.3.0 eMSP can talk through a hub built on this crate:

```rust
use ocpi_kit::hub::{bridge, Bridge};
use ocpi_kit::convert::Upgrade;

match bridge(&sender_version, &receiver_version) {
    Bridge::Passthrough => forward(cdr),
    Bridge::Upgrade => {
        let converted = cdr.upgrade();
        for loss in &converted.lossy {
            tracing::warn!(pointer = %loss.pointer, "did not survive: {}", loss.reason);
        }
        forward(converted.value)
    }
    Bridge::Downgrade => { /* … */ }
    Bridge::Unsupported => relay_verbatim(),
}
```

`Unsupported` is surfaced rather than treated as an error. Two 3.0 parties talking through your hub
should have their bytes relayed unchanged — a hub that refuses to relay a message between two
parties that both understand it is worse than useless — but that is the operator's decision, so the
crate makes it visible instead of assuming.

See [Versions and conversion](@/docs/concepts/versions.md) for what a bridge costs.
