+++
title = "Install and features"
weight = 10
description = "Add ocpi-kit to a Cargo project and choose the feature set your role needs."
+++

```toml
[dependencies]
ocpi-kit = "0.1"
```

The default features are `v2_3_0`, `v2_2_1` and `transport` — enough to decode, validate and
envelope OCPI messages without pulling in an HTTP stack.

## The feature map

| Feature | Pulls in | What it gives you |
|---|---|---|
| `v2_3_0` | — | the OCPI 2.3.0 wire model, 59 objects. The canonical model of this crate |
| `v2_2_1` | `v2_3_0` | the OCPI 2.2.1 wire model, 53 objects, defined as a delta |
| `v2_1_1` | `v2_3_0` | the OCPI 2.1.1 wire model, 33 objects, for legacy peers |
| `bookings` | `v2_3_0` | the Bookings module of the 2.3.0 `bookings` release branch |
| `invoice-reconciliation` | `v2_3_0` | the Invoice Reconciliation module of the `payments` branch |
| `transport` | — | envelope, status codes, headers, credentials tokens, pagination, routing, PATCH |
| `client` | `transport`, `v2_3_0`, `convert`, `reqwest`, `tokio` | async client and the registration handshake |
| `server` | `transport`, `v2_3_0`, `convert`, `axum`, `tokio` | the router and its module traits |
| `hub` | `client`, `server` | routing table, forwarder, aggregation, version bridging |
| `convert` | `v2_2_1`, `v2_3_0` | `Upgrade`/`Downgrade` with loss accounting, and the JSON-level bridge |
| `tariffs` | `tzdb`, `tz-rs` | the pricing engine |
| `testkit` | `transport` | validated samples and in-memory stores |
| `schema` | `schemars` | `JsonSchema` for every wire type |
| `full` | all of the above | everything except the CLI |
| `cli` | `full`, `clap` | the `ocpi` binary |

The [conformance runner](@/docs/layers/conformance.md) comes with `client`; it needs no feature of its
own.

`client`, `server` and `hub` enable `convert` for the same reason they enable `v2_3_0`: they speak
the canonical model and translate the peer onto it, so a peer on OCPI 2.2.1 — most of the market —
is readable at all. See [Versions and conversion](@/docs/concepts/versions.md).

Feature selection is real dependency isolation: with the default features the crate has no
async runtime, no HTTP client and no TLS stack. `types` alone (`--no-default-features`) compiles
for `wasm32`, so the same models can power browser tooling.

## Version support at a glance

A version number this build does not model is still *parsed* — `VersionNumber::Custom("3.0")` —
so discovery against a future peer never fails outright. Only the wire models are gated.

Two questions that look alike and are not:

* `VersionNumber::is_supported()` — does this build have a **wire model** for that version?
* `convert::wire::bridgeable(&from, &to)` — can this build carry a **document between** two of
  them? Today that is 2.2.1 ↔ 2.3.0 and nothing else; OCPI 2.1.1 is modelled and deliberately not
  bridged. See [Versions and conversion](@/docs/concepts/versions.md).

## Minimum supported Rust version

1.96, edition 2024. Raising the MSRV is a minor-version change.

## Installing the CLI

```console
cargo install ocpi-kit --features cli
```
