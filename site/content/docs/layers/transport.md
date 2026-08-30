+++
title = "Transport"
weight = 10
description = "The OCPI envelope, status-code rules, headers, credentials tokens, pagination, routing and PATCH — with no HTTP stack."
+++

Everything between the JSON and the HTTP: the envelope, status codes, headers, credentials tokens,
pagination, routing, endpoint URLs, PATCH and per-peer quirks.

This layer has **no HTTP client and no async runtime**. It is the shared vocabulary that the
client, server and hub are written in, and it is deliberately usable on its own if you want to keep
your own HTTP stack.

## The envelope

Every OCPI response body is the same shape:

```rust
pub struct OcpiResponse<T> {
    pub data: Option<T>,
    pub status_code: StatusCode,
    pub status_message: Option<String>,
    pub timestamp: DateTime,
}
```

## Status codes are almost never HTTP status codes

This is the rule that OCPI implementations get wrong most often:

> Only five situations get an HTTP error status. Everything that reached the OCPI layer is
> `200 OK` with a four-digit code in the body.

A `2001 Invalid or missing parameters` is a **200**. A `2003 Unknown Location` is a **200**. An
authentication failure is a real 401, and an unroutable request is a real 404, and that is nearly
the whole list. `OcpiError::http_status()` encodes the complete rule, so the mapping happens once
and correctly rather than at every handler.

`StatusClass` groups the codes the way the specification does: `1xxx` success, `2xxx` client error,
`3xxx` server error, `4xxx` hub error.

## Credentials tokens

`CredentialsToken` is not a `String`:

* comparison is **constant-time** (`subtle`), so a token check is not a timing oracle
* `Debug` and `Display` are **redacted**, so it cannot leak into a log line by accident
* the value is **zeroised on drop**
* `TokenRole` distinguishes `A`, `B` and `C`, and `may_access()` enforces that
  `CREDENTIALS_TOKEN_A` is scoped to the `credentials` and `versions` modules only

## Headers

`RequestIds` carries `X-Request-ID` and `X-Correlation-ID`. The rule a hub must not break has its
own constructor:

```rust
let onward = incoming.forwarded();   // new X-Request-ID, same X-Correlation-ID
```

`RoutingHeaders` carries the four `OCPI-{to,from}-{country-code,party-id}` headers, and
`applies_to()` knows that they belong on functional modules and not on `credentials`, `versions` or
`hubclientinfo`.

## Pagination

`PageQuery` builds `date_from`/`date_to`/`offset`/`limit`; `PageMeta` reads `X-Total-Count`,
`X-Limit` and the `Link: rel="next"` header; `Page<T>` is the two together.

`crawl_adjustment()` handles the case every crawler hits: the server capped your `limit` to
something smaller than you asked for, and your offset arithmetic is now wrong. It tells you what to
do about it instead of leaving you to find out from a partner's duplicate-object complaint.

## PATCH

OCPI PATCH is **RFC 7396 JSON Merge Patch**, with one extra rule:

> A patch must carry `last_updated`.

`Patch::apply` enforces it — a patch without it is the specification's own example of a `2001`,
and it never reaches your handler.

## Endpoint URLs

> The URLs of the endpoints in this document are descriptive only. The exact URL can be found by
> fetching the endpoint information from the API info endpoint.

So `SenderEndpoint` and `ReceiverEndpoint` take the **discovered** base URL and append only the
parts the specification does define — the client-owned object path
(`{base}/{country_code}/{party_id}/{object_id}`), the nested Location path, the command name.
Nothing here invents a base path.

## Quirks

Interop knowledge that is otherwise tribal, turned into documented, testable flags on a `Quirks`
struct: `accept_unencoded_token`, `send_unencoded_token`, `omit_routing_headers`,
`trailing_slash`, `null_means_absent`, `na_sentinel`, `case_insensitive_module_ids`,
`max_page_limit`, `lenient_id_length`, `lenient_content_type`. See
[Interop notes](@/docs/reference/interop.md).
