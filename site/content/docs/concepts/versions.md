+++
title = "Versions and conversion"
weight = 50
description = "OCPI 2.3.0, 2.2.1 and 2.1.1 side by side, and converting between them with loss accounting."
+++

## Three wire models, one canonical one

OCPI 2.3.0 is this crate's canonical model. 2.2.1 and 2.1.1 are defined as **deltas** from it: they
re-export the types that are wire-identical and redefine only the ones that genuinely differ.

That is not a shortcut — it is what keeps them honest. `v2_2_1` redefines exactly the objects whose
shape changed (`Price`, `Role`, `Location`, `Evse`, `Connector`, `Tariff`, `Cdr`, `Session`,
`Token`, `Credentials` and a handful more) and nothing else, so a reader can see the whole
difference between two OCPI versions by reading one module.

2.1.1 is a bigger delta and is modelled separately where it has to be: `Session` uses
`start_datetime`/`end_datetime`, a `Cdr` embeds a whole `Location` and carries `auth_id`,
`Credentials` is flat, and `CommandResponse` doubles as the async result with a `TIMEOUT` value.

## Version numbers this build does not model

`VersionNumber` is an open enum. A peer advertising `3.0` parses as
`VersionNumber::Custom("3.0")` rather than failing, so version discovery against a future or
unknown peer always completes and you can decide what to do about it.

`VersionNumber::is_supported()` says whether *this build* models a version;
`cmp_by_release()` orders them by release rather than by string, so `2.1.1 < 2.2 < 2.2.1 < 2.3.0`
comes out right.

## Converting between versions

The `convert` feature gives you `Upgrade` and `Downgrade`, and — this is the part that matters —
**loss accounting**:

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
against all 59 of the specification's 2.2.1 examples.

### The interesting cases

* **`Price`.** 2.2.1 is `{excl_vat, incl_vat}`; 2.3.0 is `{before_taxes, taxes[]}` with per-line
  tax names and percentages. Upgrading synthesises a single unnamed tax line; downgrading collapses
  the lines and records the names it lost.
* **The HUB role.** 2.2.1 has `Role::Hub`; 2.3.0 replaced it with a `hub_party_id` on the
  credentials. Both directions are handled, and the mapping is documented on the impl.
* **Owner fields.** 2.1.1 Locations have no `country_code`/`party_id`; upgrading needs them
  supplied, which the API makes explicit rather than guessing.

## Bridging in a hub

`hub::bridge(&sender_version, &receiver_version)` tells a hub what a message crossing between two
parties needs: `Passthrough`, `Upgrade`, `Downgrade` or `Unsupported`. `Unsupported` is surfaced
rather than assumed to be an error — two 3.0 parties talking through your hub should have their
bytes relayed unchanged, and that is the operator's decision to make. See [Hub](@/docs/layers/hub.md).
