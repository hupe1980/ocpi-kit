+++
title = "Interop notes and quirks"
weight = 20
description = "What real OCPI peers actually do, and the per-peer flags that let you accept it."
+++

The specification describes what peers should do. This page is about what they actually do.

`Quirks` turns that knowledge into documented, testable flags instead of tribal knowledge and
`if peer_name == "…"` branches. Set them per peer on the client, or per registered platform on the
server.

| Flag | What it tolerates |
|---|---|
| `accept_unencoded_token` | A peer sending the credentials token without base64 encoding (very common with pre-2.2 implementations) |
| `send_unencoded_token` | A peer that only accepts it unencoded |
| `omit_routing_headers` | 2.1.1 peers, where the `OCPI-to-*` headers do not exist |
| `case_insensitive_module_ids` | `Booking` vs `bookings`, and other casing drift in module ids |
| `peer_max_page_limit` | Clamps our outgoing `limit` to what **the peer** tolerates. Named for the side it describes: `ServerConfig::max_page_limit` is the cap *this* process puts on the pages it serves |
| `lenient_content_type` | A peer sending something other than `application/json` |

Six flags, and **every one of them changes behaviour**. That is enforced rather than hoped:
`cargo run -p xtask -- dead-config` fails the build if a public field of `Quirks` — or of
`ClientConfig`, `ServerConfig`, `PricingPolicy` or `UrlPolicy` — is only ever assigned and never
read.

A setting that does nothing is worse than a missing feature: you read the description, set the flag,
believe the problem is handled, and ship.

## Things that are not quirks, because they are unconditional

These are not flags, because there is no coherent way to switch them off:

* **A trailing slash on a discovered URL** is normalised by `Url::join` whatever anyone
  configures. `…/locations/` and `…/locations` reach the same place.
* **An explicit `null`** decodes to `None`, because that is what `Option<T>` means in serde.
* **An over-long identifier** is always accepted and always reported. That is the crate's
  governing rule — parse permissively, validate explicitly — not a per-peer setting. See
  [Parse, validate, construct](@/docs/concepts/parse-validate-construct.md).

* **`#NA`** is recognised by `CiString::is_not_available()` wherever a caller asks. Whether a `#NA`
  in a given field is acceptable depends on the field, not on the peer, so the decision belongs at
  the call site.

## Things that are not quirks, because the peer is simply wrong

Some behaviour that looks like a quirk is actually the specification, and `ocpi-kit` implements it
unconditionally:

* **A `2001` is a `200`.** If a peer answers your malformed request with an HTTP 400, *they* are
  wrong. Only five situations get an HTTP error status.
* **`CREDENTIALS_TOKEN_A` stops working after registration.** A peer that keeps accepting it is
  being permissive, not correct.
* **A PATCH without `last_updated` is invalid.** The specification uses exactly this as its example
  of a `2001`.
* **A hub must not touch `last_updated`.** If objects arrive with a hub's timestamp on them, that
  hub has a bug.

## Version drift you will meet

* A **2.2.1 peer sending a 2.3.0 enum value** — `MCS`, `SAE_J3400`, `EMAID`, the new parking
  restrictions. `ocpi-kit` decodes them into `Custom` and reports them from `validate()`, so you
  get the object and you get told. See [Enums](@/docs/concepts/enums.md).
* A **2.1.1 peer with 39-character Location ids** where the spec says 36. The value arrives
  intact and `validate()` reports it; no flag needed, and none would help.
* **`Price` shape confusion.** Objects written against 2.2.1's `{excl_vat, incl_vat}` arriving on a
  2.3.0 endpoint that expects `{before_taxes, taxes[]}`. The specification's own payments examples
  make this mistake — see [Spec errata](@/docs/reference/errata.md).

## What actually goes wrong: CDRs

The wire types are the part everybody gets right. Published accounts from hub operators and
integrators agree on where the real trouble is, and it is the CDR: *"CDR inconsistencies are the
single most common source of OCPI integration issues, arising from timezone handling differences,
rounding behavior, tariff interpretation mismatches, and session ID propagation failures across
hubs."* Two 2.2.1-conformant platforms disagree because one rounded at a different point, or read
a tariff component differently.

`ocpi-kit` attacks that from the pricing side rather than the modelling side:

* **Time zones** are resolved per instant against a bundled IANA database, so a session either
  side of a daylight-saving change is evaluated against the wall clock that was actually showing.
* **Rounding** is a parameter with a documented default, and the point at which it is applied is
  part of the parameter — not something you have to infer from behaviour.
* **A CDR that is itself malformed is reported**, not silently absorbed. A Charging Period that
  outlasts the Price Component pricing it produces a plausible total and a defect; see
  [Tariffs](@/docs/layers/tariffs.md).

## Sender and Receiver path collision

If you are a platform that is both CPO and eMSP, read the note in [Server](@/docs/layers/server.md)
about `receiver_path_prefix` before you mount both interfaces of a module on one router. The two
interfaces have the same path arity and the ambiguity is not resolvable by route ordering.
