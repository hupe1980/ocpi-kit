+++
title = "Security"
weight = 40
description = "Token handling, the SSRF guard on peer-supplied URLs, ownership checks, and limits."
+++

OCPI's security model is bearer tokens over TLS, and the specification says relatively little
beyond that. The gap between "little" and "nothing" is where the interesting failures live, so
`ocpi-kit` makes several decisions for you — all of them overridable, none of them silent.

## Tokens

`CredentialsToken` is not a `String`:

* **Constant-time comparison** (`subtle`), so validating an incoming token is not a timing oracle.
* **Redacted `Debug` and `Display`**, so a token cannot reach a log line through a struct dump.
* **Zeroised on drop** (`zeroize`).
* **Never in a URL.** No API in this crate puts a token in a query string.
* `TokenRole` distinguishes `A`, `B` and `C`, and the server enforces that `CREDENTIALS_TOKEN_A` is
  usable only on `credentials` and `versions`. Using it anywhere else is a 401, as the
  specification requires.

## SSRF: the one that catches everyone

`Credentials.url`, every `Endpoint.url` in a version-details response, and every `response_url` in
an asynchronous command are **supplied by a peer**. A client or hub that fetches them
unconditionally is an SSRF proxy into your infrastructure, and a hub is an especially good one
because it has network access to fifty other parties.

`UrlPolicy` is applied to every outgoing URL, and by default it refuses:

* plain `http://`
* loopback (`127.0.0.0/8`, `::1`, `localhost`, `*.localhost`, `*.local`)
* private and link-local address ranges

```rust
// For local development against a test peer, and nothing else:
let client = OcpiClient::with_config(ClientConfig::for_testing())?;

// Or relax exactly one thing, and nothing else:
let config = ClientConfig { url_policy: UrlPolicy::default().allowing_http(), ..Default::default() };
```

`UrlPolicy` also has `with_allowed_hosts` for a per-peer allow-list, `allowing_private_networks`,
and `permissive()` for tests.

The CLI exposes the same decision as `--insecure`, with the reason in its help text.

## Ownership

Client-owned objects live under `{country_code}/{party_id}`. The server checks that a platform
writing there holds that role and answers **404** otherwise — the status the specification chooses
precisely so that a probe cannot distinguish "not yours" from "not there" — and your handler is
never called.

## Limits

Pagination is capped by default (`ServerConfig::max_page_limit`, 100), so a peer cannot ask for a
million objects in one page — the specification allows a server to return fewer than requested, and
the response says how many it actually applied.

Request body size is left to the `axum`/`tower` layer you already have; the router does not add a
limit of its own, so add `tower_http::limit::RequestBodyLimitLayer` if nothing upstream of you does.

## The supply chain

* `#![forbid(unsafe_code)]` — there is no `unsafe` in this crate.
* `cargo deny` runs in CI over bans, licenses, sources and advisories.
* TLS is `rustls`, not OpenSSL, so there is no system C dependency to keep patched.
* The dependency set is deliberately small, and each optional feature pulls only what it needs —
  with the default features there is no HTTP stack and no async runtime in your build at all.

## What is still yours to do

Terminating TLS, rotating tokens on a schedule, and deciding what your `UrlPolicy` allows in
production. `ocpi-kit` gives you a token rotation API and a policy type; it cannot know your network.
