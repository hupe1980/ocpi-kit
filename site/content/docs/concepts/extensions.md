+++
title = "Extensions"
weight = 40
description = "Undocumented JSON fields survive a round-trip verbatim, which is what keeps a hub from shredding vendor data."
+++

OCPI 2.3.0 has a chapter about extensibility, and it asks implementations to tolerate values and
fields they do not know. Most libraries do not. `ocpi-kit` does, on both axes:

* an unknown **enum value** keeps its text in a `Custom` variant — see [Enums](@/docs/concepts/enums.md)
* an unknown **field** lands in an `Extensions` map on the object it came from

```rust
let json = r#"{"id":"LOC1", "…":"…", "acme_grid_operator":"Stedin"}"#;
let location: Location = serde_json::from_str(json)?;

assert_eq!(location.extensions.get("acme_grid_operator").unwrap(), "Stedin");
assert_eq!(serde_json::to_string(&location)?, json);   // written back verbatim
```

Every wire object carries `extensions: Extensions`, a `#[serde(flatten)]` map. Round-tripping an
object you do not fully understand is byte-stable.

## Why this is the difference between a working hub and a broken one

A hub sits between fifty parties. If a CPO sends a Location with a vendor field and the hub decodes
it into a strict struct and re-encodes it, the field is gone by the time it reaches the eMSP that
needed it. Nobody notices for weeks, and then somebody's grid-capacity integration silently stops
working.

This is the single most common way real hubs lose data, and it is a *type system* problem, not an
operations problem. A crate that cannot represent the unknown cannot forward it.

## Writing extensions of your own

```rust
let mut location = Location::builder()./* … */.build();
location.extensions.insert("acme_grid_operator", "Stedin");
```

The 2.3.0 specification does not define a naming rule for extension fields beyond "do not modify
the standard ones, do propose additions". A vendor prefix is this crate's recommendation, not a
requirement — it keeps two parties' extensions from colliding in the same object.

## What extensions are not

`Extensions` holds JSON values, not typed data, and it is deliberately not validated. If you find
yourself reading structured data out of it in more than one place, you probably want your own
struct with a `#[serde(flatten)]` field of your own on top of the OCPI object.
