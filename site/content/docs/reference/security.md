+++
title = "Security"
weight = 40
description = "Token handling, the SSRF guard on peer-supplied URLs — every hop, every connection — ownership checks, and limits."
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

A `UrlPolicy` decides where the client may go, and the client enforces it **three times**:

1. **On the URL.** By default it refuses plain `http://`, the names `localhost`, `*.localhost` and
   `*.local`, and any IP literal that is not globally reachable.
2. **On every redirect hop.** The HTTP client never follows a redirect by itself. The client
   follows one only for a `GET`, only within the origin it asked (or an `http` → `https` upgrade
   of it), only to a URL the policy accepts, and at most five times. A redirect on a write is
   never followed: it would be replayed or silently turned into a `GET`. Without this rule, one
   `302` to `http://169.254.169.254/` reaches the cloud metadata service.
3. **On the address connected to.** A resolver in the connection path drops every resolved
   address the policy refuses. A host name whose record says `10.0.0.5` is refused; so is one
   that answers publicly when checked and privately when connected (DNS rebinding), because the
   addresses checked *are* the addresses connected to.

"Not globally reachable" means the IANA special-purpose registries, not a short list: private,
loopback, link-local, carrier-grade NAT, `0.0.0.0/8`, benchmarking, documentation (both
families), multicast, reserved space — and the same addresses spelled inside IPv6
(`::ffff:169.254.169.254`, `::169.254.169.254`, NAT64 `64:ff9b::a9fe:a9fe`). The 6to4 and Teredo
tunnels are refused outright. `UrlPolicy::permits_address` exposes the rule, so code that has to
judge an address itself uses the same one.

Every refusal is `OcpiError::UrlRefused` with the URL (query and fragment removed) and a reason a
person can act on — never a generic transport error, and never retried.

```rust
// For local development against a test peer, and nothing else:
let client = OcpiClient::with_config(ClientConfig::for_testing())?;

// Or relax exactly one thing, and nothing else:
let config = ClientConfig::default().with_url_policy(UrlPolicy::default().allowing_http());
```

`UrlPolicy` also has `with_allowed_hosts` for a per-peer allow-list, `allowing_private_networks`,
and `permissive()` for tests.

The CLI exposes the same decision as `--insecure`, with the reason in its help text.

### Bringing your own HTTP client

Need a proxy, custom TLS roots or a shared connection pool? Start from the guarded builder, so the
redirect rule and the resolver come with it:

```rust
let config = ClientConfig::default();
let http = config.http_builder().pool_max_idle_per_host(32).build()?;
let client = OcpiClient::from_http(http, config);
```

A client built any other way keeps the URL check but loses the address check — and if it follows
redirects itself (`reqwest`'s default), every response it reached by one is refused, loudly, after
the fact.

### What remains a deployment's job

* **Behind an HTTP proxy** the resolver sees the proxy, not the target: the proxy is then where
  egress rules for the target are enforced.
* `with_allowed_hosts` is still the strongest control: you already know which host your partner's
  endpoints live on, and discovery does not have to be trusted to tell you.
* An **egress policy on the network** remains good practice, as a second net under the first.

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
what your `UrlPolicy` allows in production — including whether a proxy or an egress policy backs
it up.
`ocpi-kit` gives you a token rotation API and a policy type; it cannot know your network.
