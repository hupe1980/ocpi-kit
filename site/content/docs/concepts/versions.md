+++
title = "Versions and conversion"
weight = 50
description = "OCPI 2.3.0, 2.2.1 and 2.1.1 side by side, one canonical model above them, and what it costs to cross."
+++

## Three wire models, one canonical one

OCPI 2.3.0 is this crate's canonical model. 2.2.1 and 2.1.1 are defined as **deltas** from it: they
re-export the types that are wire-identical and redefine only the ones that genuinely differ.

`v2_2_1` redefines exactly the objects whose shape changed — `Price`, `Role`, `Location`, `Evse`,
`Connector`, `Tariff`, `Cdr`, `Session`, `Token`, `Credentials` and a handful more — and nothing
else, so the whole difference between two OCPI versions is one module to read.

2.1.1 is a bigger delta and is modelled separately where it has to be: `Session` uses
`start_datetime`/`end_datetime`, a `Cdr` embeds a whole `Location` and carries `auth_id`,
`Credentials` is flat, and `CommandResponse` doubles as the async result with a `TIMEOUT` value.

## Everything above `transport` speaks 2.3.0

`client`, `server`, `hub` and `tariffs` use the `v2_3_0` model and nothing else. Your handlers take
`v2_3_0::locations::Location`; your pulls hand you `v2_3_0::tariffs::Tariff`.

That is only defensible if those layers can reach a peer that is *not* on 2.3.0 — and most are not.
So they translate at the edge:

```rust
// The peer registered as OCPI 2.2.1. Nothing below says so.
let mut tariffs = peer.tariffs(client.transport(), me).list(PageQuery::new())?;
while let Some(tariff) = tariffs.next().await? {
    assert_eq!(tariff.tax_included, TaxIncluded::No);   // a v2_3_0 Tariff
}
```

Load-bearing, not a convenience: OCPI 2.3.0 made `Tariff.tax_included` **required** and a 2.2.1
Tariff does not have it, so a library that models both versions and translates neither cannot decode
a Tariff from most of the market. The handshake is bridged for the same reason — a 2.2.1 hub's
`Credentials` carries a `HUB` role the 2.3.0 `Role` enum does not have.

The plain `ModuleClient::get`/`put`/`post`/`list` are the escape hatch: they decode exactly the type
you name, and translate nothing.

## Converting between versions

The `convert` feature gives you `Upgrade` and `Downgrade`, with **loss accounting**:

```rust
use ocpi_kit::convert::Upgrade;

let converted = cdr_2_2_1.upgrade();          // Converted<v2_3_0::cdrs::Cdr>
let cdr = converted.value;
for loss in &converted.lossy {
    eprintln!("{}: {}", loss.pointer, loss.reason);
}
```

A conversion never silently drops anything. When a 2.3.0 field has no 2.2.1 equivalent, the
downgrade records a `Loss` with the JSON Pointer of what was dropped and why. Round-tripping an
object through the other version and back and comparing it is a test in this repository, run
against all of the specification's 2.2.1 examples.

### The interesting cases

* **`Price`.** 2.2.1 is `{excl_vat, incl_vat}`; 2.3.0 is `{before_taxes, taxes[]}` with per-line
  tax names and percentages. Upgrading synthesises a single tax line named `VAT`; downgrading
  collapses the lines and records the names it lost — which matters in Canada, where a receipt has
  to itemise GST and QST separately.
* **The HUB role.** 2.2.1 has `Role::Hub`; 2.3.0 replaced it with a `hub_party_id` on the
  credentials. Both directions are handled, and the mapping is documented on the impl.
* **`Tariff.tax_included`.** Upgrading picks `NO`, because a 2.2.1 `PriceComponent.price` is *"Price
  per unit (excl. VAT)"* by definition. Downgrading a Tariff that says `YES` is a **loss**, not a
  reinterpretation: the same numbers would mean a different amount of money.
* **Enum values added later.** A 2.3.0 `ConnectorType::Mcs` downgrades to a 2.2.1
  `ConnectorType::Custom("MCS")` — the same string on the wire, nothing lost — and upgrades straight
  back. See [Open enums](@/docs/concepts/enums.md).

## The JSON-level bridge

`Upgrade`/`Downgrade` work on typed objects. A hub, a client talking to an older peer and a server
answering one face the problem one step earlier: they hold **bytes** and know the endpoint those
bytes came from, not the Rust type they will become. `convert::wire::ObjectKind` closes that gap:

```rust
use ocpi_kit::convert::wire::{ObjectKind, Payload};

// Which object does `{locations}/{location_id}/{evse_uid}` carry? An EVSE.
let kind = ObjectKind::for_endpoint(
    &ModuleId::Locations, InterfaceRole::Sender, "LOC1/3256", Payload::Response,
);

// Translate it — one object or a whole page — keeping the loss report.
let converted = kind.unwrap().bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_3_0, value)?;
```

Only the objects whose wire format actually changed are variants. `for_endpoint` returns `None` for
everything else, and the caller forwards those bytes untouched — so a hub pays for a translation
only where there is one.

### Merge patches

A merge patch is not an object, so it cannot be decoded, converted and re-encoded. It does not have
to be: a patch writing only fields the two versions agree about means the same thing in both, which
is what nearly every real patch does — an EVSE status update above all.

`ObjectKind::divergent_fields()` is the list they disagree about. A patch touching none of them
crosses unchanged; one touching `help_phone` or `total_cost` is refused, with the specification's own
recovery in the message — *"call the GET method to check the state of the object … If the object
doesn't exist, the client should do a PUT."* The list is checked against the whole 2.2.1 fixture
corpus.

## What this build can and cannot cross

`convert::wire::bridgeable(&from, &to)` is the single answer, and every layer asks it rather than
guessing:

| Crossing | Supported |
|---|---|
| any version → itself | yes, as a no-op |
| 2.2.1 ↔ 2.3.0 | yes, both directions |
| anything involving 2.1.1 | **no** |
| anything involving 2.0/2.1/2.2/3.0 | **no** |

2.1.1 is modelled and deliberately not bridged. It has no owner fields on objects, no routing headers
and no `Price`, so carrying an object across that boundary is a decision about a deployment — *who
owns this Location, and what does a bare `total_cost` exclude?* — not a translation a library can make
on its own.

## Version numbers this build does not model

`VersionNumber` is an open enum. A peer advertising `3.0` parses as `VersionNumber::Custom("3.0")`
rather than failing, so version discovery against a future or unknown peer always completes and you
can decide what to do about it.

`VersionNumber::is_supported()` says whether *this build* has a wire model for a version — a
different question from `bridgeable`, which asks whether it can carry a document *between* two.
`cmp_by_release()` orders them by release rather than by string, so `2.1.1 < 2.2 < 2.2.1 < 2.3.0`
comes out right.

## Bridging in a hub

A hub does not call any of this by hand: `Forwarder` translates in both directions and reports what
it cost. `hub::bridge(&sender_version, &receiver_version)` exposes the same classification —
`Passthrough`, `Upgrade`, `Downgrade` or `Unsupported` — for a hub that wants to translate something
itself. See [Hub](@/docs/layers/hub.md).
