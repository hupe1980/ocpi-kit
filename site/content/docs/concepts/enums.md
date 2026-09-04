+++
title = "Enums, open and closed"
weight = 30
description = "How ocpi-kit models OCPI closed enums, OpenEnums, and values a peer invented after your last release."
+++

OCPI 2.3.0 distinguishes two kinds of enumeration, and the distinction has real consequences.

> A **closed** enum may only contain the values listed. An **OpenEnum** may be extended with
> values not in the specification.

`ocpi-kit` models them with three macros, producing three behaviours.

## `ocpi_enum!` — closed

The value set is fixed. An unknown value is a **decode error**.

Reserved for the enums a value of which *drives a decision*, where one nobody has seen has no
meaning to act on: the pricing engine's inputs (`TariffDimensionType`, `CdrDimensionType`,
`TaxIncluded`, the restriction types), `DayOfWeek`, the routing layer's `InterfaceRole` and `Role`,
and the outcome enums a caller branches on (`CommandResultType`, `ChargingProfileResultType`,
`ReservationStatus`, `CaptureStatusCode`). Those also stay `Copy`, which the pricing engine's inner
loops care about.

The rule is not "the specification says closed" — it says that about almost everything — it is
**does something here have to know what the value means?**

## `ocpi_open_enum!` — open

The generated type has a `Custom(String)` variant that keeps the unknown value's text verbatim.
Decoding succeeds; validation is silent, because an extension value is *legal here*.

```rust
let connector: ConnectorType = serde_json::from_str("\"ACME_PROPRIETARY\"")?;
assert_eq!(connector, ConnectorType::Custom("ACME_PROPRIETARY".into()));
assert_eq!(serde_json::to_string(&connector)?, "\"ACME_PROPRIETARY\"");  // written back intact
```

This is what lets a hub relay a vendor extension it has never seen. See
[Extensions](@/docs/concepts/extensions.md).

## `ocpi_lenient_enum!` — closed, but survivable

The enums a peer fills in and this crate only carries — `Status`, `SessionStatus`, `AuthMethod`,
`PowerType`, `ConnectorFormat`, `TariffType`, `WhitelistType` and their kind, in **every** version.
An unknown value decodes into `Custom(String)`, keeps its text verbatim, and is **reported** by
`validate()`: you get the object, and you get told it is not conformant.

Two situations make this the right default rather than a concession.

A 2.2.1 peer sending a value 2.3.0 added — `MCS`, `SAE_J3400`, `EMAID`, the new parking
restrictions — is a fact of deployment, and 2.2.1 declares every enum closed, so a strict decoder
loses those objects.

And a peer sending a value *nobody* defined is the same problem with a different cause. The
example that matters is `Evse.status`: a page of a hundred Locations, one EVSE reporting a status
this version does not define, and a strict decoder throws away the page. So the value survives in
`Custom`, `validate()` says it should not have been sent, and the CPO's other ninety-nine EVSEs
arrive.

```rust
let evse: Evse = serde_json::from_str(page_with_one_odd_status)?;   // the page survives
assert_eq!(evse.status.as_str(), "MAINTENANCE");                    // verbatim
assert!(evse.validate().is_err());                                  // and reported
```

## The `Custom` variant, not `Other`

The catch-all is named `Custom` because several OCPI enums have a legitimate spec value called
`OTHER` — `ImageCategory::Other`, `TokenType::Other`. A catch-all named `Other` would have
collided with a real value and made the API ambiguous.

## Equality goes through the wire value

A value that reached `Custom` by one route still equals the variant it names:

```rust
assert_eq!(ConnectorType::Custom("CHADEMO".into()), ConnectorType::Chademo);
```

`Eq`, `Hash` and `Ord` all route through `as_str()`, so a `HashSet` or a `BTreeMap` keyed on an
open enum behaves the way you would expect regardless of which side of the boundary a value came
from.

## What actually differs between versions

Diffing the value sets programmatically: the only enums whose members differ between OCPI 2.2.1 and
2.3.0 are `ConnectorType` (2.3.0 adds `MCS` and `SAE_J3400`), `ParkingRestriction` (adds
`EMPLOYEES`, `TAXIS`, `TENANTS`) and `TokenType` (adds `EMAID`). Everything else is identical, which
is why the 2.2.1 model can re-export most of the 2.3.0 types rather than duplicating them.
