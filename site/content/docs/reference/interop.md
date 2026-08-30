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
| `trailing_slash` | `…/locations/` in discovered URLs and `Link` headers |
| `null_means_absent` (on by default) | `"data": null`, `"evse_id": null` → `None` rather than a decode error |
| `na_sentinel` | `#NA` string fields treated as a not-available marker where the spec allows it |
| `case_insensitive_module_ids` | `Booking` vs `bookings`, and other casing drift in module ids |
| `max_page_limit` | Clamps our `limit` to what the peer tolerates |
| `lenient_id_length` | Vendor ids longer than the version's limit — accept and report rather than reject |
| `lenient_content_type` | A peer sending something other than `application/json` |

## Things that are not quirks

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
* A **2.1.1 peer with 39-character Location ids** where the spec says 36. Covered by
  `lenient_id_length`.
* **`Price` shape confusion.** Objects written against 2.2.1's `{excl_vat, incl_vat}` arriving on a
  2.3.0 endpoint that expects `{before_taxes, taxes[]}`. The specification's own payments examples
  make this mistake — see [Spec errata](@/docs/reference/errata.md).

## Sender and Receiver path collision

If you are a platform that is both CPO and eMSP, read the note in [Server](@/docs/layers/server.md)
about `receiver_path_prefix` before you mount both interfaces of a module on one router. The two
interfaces have the same path arity and the ambiguity is not resolvable by route ordering.
