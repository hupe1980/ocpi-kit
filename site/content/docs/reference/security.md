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
* private, link-local, carrier-grade-NAT and documentation ranges
* the same addresses spelled as IPv6: `::ffff:169.254.169.254`, the deprecated
  `::169.254.169.254`, and the NAT64 prefix `64:ff9b::169.254.169.254` — an allow-list that only
  knew the dotted-quad form would wave all three through

```rust
// For local development against a test peer, and nothing else:
let client = OcpiClient::with_config(ClientConfig::for_testing())?;

// Or relax exactly one thing, and nothing else:
let config = ClientConfig { url_policy: UrlPolicy::default().allowing_http(), ..Default::default() };
```

`UrlPolicy` also has `with_allowed_hosts` for a per-peer allow-list, `allowing_private_networks`,
and `permissive()` for tests.

The CLI exposes the same decision as `--insecure`, with the reason in its help text.

### What URL inspection cannot do

A `UrlPolicy` sees the URL, and only the URL. It cannot see where a **host name** resolves, so
`https://ptp.example.com/cb` passes even when that name has an `A` record for `169.254.169.254` —
and a name that resolves differently between the check and the connection defeats it outright, the
classic DNS rebind. Closing that needs a resolver in the connection path, which belongs to the
HTTP client rather than to a URL type.

So this is the first of two layers, not the whole defence. The literal-IP rules above stop the
careless cases; two other things stop the deliberate ones:

* `with_allowed_hosts` — an explicit list per peer is subject to neither problem. You already know
  which host your partner's endpoints live on; discovery does not have to be trusted to tell you.
* An **egress policy on the network** that refuses the link-local and private ranges outright,
  regardless of what any application resolved.

A test asserts the limitation, so it cannot quietly turn into a false sense of safety.

## Identifiers in URLs

An OCPI identifier is a `CiString(36)`, and the specification puts **no restriction** on which
characters it may contain. Every id this crate puts in a path therefore goes through
`Url::join_segment`, which percent-encodes anything that would change the URL's structure —
`/`, `?`, `#`, `%`, space, every non-ASCII byte — while leaving the shapes real identifiers have
(`BE*BEC*E041503001`) readable.

Without that, the *value* decides the request target. A `token_uid` of `../credentials` addresses
a different endpoint; one containing `?` starts a query string; one containing `#` truncates the
path. All three are reachable from data a peer sent — a Token uid an eMSP pushed, a Location id a
CPO published — which makes this an injection surface rather than a formatting detail. Query
parameter values are encoded the same way by `Url::with_param`.

`Url::join` is the raw counterpart, documented as taking an **already-encoded** path, because that
is what a hub forwards with: the path arrived encoded by the party that sent it, and re-encoding
it would break the request.

## Ownership

Client-owned objects live under `{country_code}/{party_id}`. The server checks that a platform
writing there holds that role and answers **404** otherwise — the status the specification chooses
precisely so that a probe cannot distinguish "not yours" from "not there" — and your handler is
never called.

## Limits

Pagination is capped by default (`ServerConfig::max_page_limit`, 100), and the cap is **enforced,
not merely advertised**: the `Page` extractor clamps an incoming `limit` before a handler sees it,
so `?limit=100000` reaches your store as `100`. `X-Limit` on the way back says the same number. A
cap that only appears in a response header is not a cap — it is a note attached to the page your
handler already built.

Request body size is left to the `axum`/`tower` layer you already have; the router does not add a
limit of its own, so add `tower_http::limit::RequestBodyLimitLayer` if nothing upstream of you does.

## The supply chain

* `#![forbid(unsafe_code)]` — there is no `unsafe` in this crate.
* `cargo deny` runs in CI over bans, licenses, sources and advisories.
* TLS is `rustls`, not OpenSSL, so there is no system C dependency to keep patched.
* The dependency set is deliberately small, and each optional feature pulls only what it needs —
  with the default features there is no HTTP stack and no async runtime in your build at all.

## Callback URLs you publish

The asynchronous halves of Commands and Charging Profiles are reached at a `response_url` you
choose and hand to the peer, and *"It is advised to make this URL unique for every request"*. That
uniqueness is not only for correlation: the URL is a capability you have handed out, so make the
id unguessable rather than a counter, and treat a result arriving at one as authenticated by the
credentials token like any other request — which is what `OcpiRouter` does, because these routes
sit behind the same `Auth` extractor as everything else.

`server::CallbackUrls` builds the URLs that reach the router's own mounts; the unique id is yours
to generate.

## What is still yours to do

Terminating TLS, rotating tokens on a schedule, generating unguessable callback ids, and deciding
what your `UrlPolicy` allows in production — including whether an egress policy backs it up.
`ocpi-kit` gives you a token rotation API and a policy type; it cannot know your network.
