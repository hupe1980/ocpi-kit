+++
title = "Numbers and money"
weight = 20
description = "Every OCPI number is an exact decimal, never a float — and what that costs at the JSON boundary."
+++

Every OCPI `number` in this crate is `types::Number`, a wrapper around `rust_decimal::Decimal`.
There is **no `f32` or `f64`** in any public field of any OCPI object, and none in the pricing
engine.

## Why this matters more than it sounds

Every price, VAT percentage, energy volume and tax amount in OCPI ends up on an invoice. A binary
float cannot represent `0.10`. Adding a column of them drifts. Two implementations that both use
`f64` will disagree about a session's cost by a cent often enough to generate disputes, and neither
can prove it is right.

```rust
let sum: Number = ["0.1", "0.2"].into_iter().map(|s| s.parse().unwrap()).sum();
assert_eq!(sum.to_string(), "0.3");   // exactly, not 0.30000000000000004
```

Every other Rust OCPI type set models `number` as `f64`. This one does not, and
`cargo run -p xtask -- no-floats` fails the build if a float appears in the wire models or the
pricing engine. The single documented exception is the JSON boundary itself.

## The JSON boundary

The specification requires these values to be JSON *numbers*, not strings. `serde_json` represents
a fractional JSON number as an `f64` unless its `arbitrary_precision` feature is on — and that
feature changes `serde_json::Value` globally for every crate in the build, so `ocpi-kit` does not
impose it on you. The boundary therefore behaves like this:

* **Integral values** pass through exactly, as JSON integers. `20` stays `20`, never `20.0`.
* **Fractional values with up to 15 significant digits** pass through exactly, because the shortest
  decimal that round-trips an `f64` *is* the original decimal. This covers OCPI's entire domain of
  prices, energies and percentages with room to spare.
* **Beyond that**, a round-trip would round. `Number::json_round_trips()` says whether a value is
  affected, and `validate()` reports it as `ViolationCode::Imprecise` — so it can never happen
  silently.

```rust
let price: Number = "0.2500".parse()?;
assert_eq!(serde_json::to_string(&price)?, "0.25");   // trailing zeros are not significant

let vat: Number = serde_json::from_str("20")?;
assert_eq!(serde_json::to_string(&vat)?, "20");       // an integer stays an integer
```

A peer that sends a number as a JSON *string* (`"0.25"`) is tolerated on input and parsed exactly;
output is always a JSON number.

### One trap worth naming

`Decimal::try_from(f64)` — the obvious way back from what `serde_json` hands over — is not exact.
For roughly one in two thousand four-decimal values in OCPI's ordinary range it adds a spurious
trailing digit: `4106.9638` becomes `4106.963800000001`. `ocpi-kit` goes through the shortest
decimal string that round-trips the `f64` instead, which is what Rust's `{}` for `f64` produces
and which for anything `serde_json` could have parsed is exactly the decimal the peer wrote.

## What it costs

Summing a thousand decimals takes about 4.4 µs; the same thousand `f64` additions take 0.46 µs. So
exact arithmetic is roughly **9× slower** — and a thousand additions still cost under 2% of
decoding the page of objects they arrived on. The arithmetic is not where the time goes. See
[How this crate is verified](@/docs/reference/verification.md#what-the-guarantees-cost).

## Rounding is a policy, not an accident

The tariffs specification says nothing about rounding, deliberately. `ocpi-kit` does not guess: the
rounding strategy and the number of decimal places are settings on `PricingPolicy`, and the
breakdown records what was applied. See [Tariffs](@/docs/layers/tariffs.md).
