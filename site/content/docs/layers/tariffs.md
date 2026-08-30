+++
title = "Tariffs"
weight = 50
description = "An auditable pricing engine: what a charging session cost, per dimension, and exactly why."
+++

OCPI is the only protocol that carries both the tariff and the metering data, which means the cost
of a charging session is computable from what crosses the wire. That is how an eMSP checks a CPO's
invoice, how a CPO checks its own, and how the Payments module's financial advice confirmations get
reconciled against the CDRs they belong to.

```rust
let breakdown = PricingEngine::new().price(&session, &tariffs)?;
assert_eq!(breakdown.total_excl_vat.to_string(), "5.00");
assert_eq!(breakdown.total_incl_vat.to_string(), "5.50");
```

## What makes this engine different

**The answer is auditable.** `CostBreakdown` does not just say `12.28`. It says, for each
dimension, which quantity was billed, what `step_size` did to it, which Tariff Element and which
Price Component priced it, and *why that element was selected*:

```text
CostBreakdown
├── dimensions: Vec<DimensionCost>          one per ENERGY / TIME / PARKING_TIME / FLAT …
│   ├── measured, billed, cost, vat         what was metered, what step_size made of it, what it cost
│   └── segments: Vec<PricedSegment>        the split where the applicable element changed
│       └── applied: AppliedComponent       tariff id, element index, component index, and `because`
├── total_excl_vat / total_incl_vat
├── taxes: Vec<TaxLine>                     name, percentage, taxable base, amount
├── limit_applied: Option<PriceLimitApplied>   min_price / max_price, if either bit
└── notes: Vec<String>                      anything the engine wants you to know
```

`AppliedComponent::because` is a sentence, not a code: *why* that Tariff Element matched this
segment. That is the field that ends an invoice dispute.

It serialises, so a disputed invoice becomes a diff of two JSON documents rather than an argument.

**The arithmetic is exact.** Every value is a decimal. There is no `f64` anywhere in this module.
See [Numbers and money](@/docs/concepts/numbers.md).

**The undefined parts are parameters.** The specification says nothing about rounding, on purpose,
and OCPI 3.0 removes `step_size` altogether. Both are settings on `PricingPolicy` rather than
assumptions baked into the code.

## `step_size`, precisely

This is the rule most implementations get wrong, and the spec states it carefully:

* an `ENERGY` `step_size` is applied **once per session**, to the total, not per period
* `TIME` and `PARKING_TIME` `step_size` is applied **once, to the two combined**, on the last
  time-based dimension of the session
* a `step_size` of `0` means no quantisation — the specification's own free-of-charge example uses
  it — while a `step_size` of `1` is meaningful and is applied

`Quantisation` is a pluggable stage: `StepSize` for OCPI 2.x, off for 3.0-style full-precision
metering.

## Restrictions in local time

`TariffRestrictions` evaluate `start_time`/`end_time` and `day_of_week` in the **Location's local
time**, not UTC — which is the difference between a night tariff starting at 22:00 and starting at
20:00 in summer. The engine takes an IANA time zone (`TimeZone::named("Europe/Berlin")`) and
resolves it with a bundled `tzdb`, so there is no dependency on the host's zoneinfo files.

Also evaluated: kWh, current, power and duration windows, `reservation`, and element switching
mid-period.

## Multiple tariffs

A Connector can list several `tariff_ids` in preference order, each with its own validity window.
The engine selects per period, records which tariff it chose in the breakdown, and falls back
through the list the way the specification describes. `min_price` and `max_price` (2.3.0's
`PriceLimit`, with `after_taxes`) are applied to the session as a whole.

## Conformance

Twelve of the specification's own worked tariff examples are tests in this repository, and they
pass — including the free-of-charge case, the ad-hoc payment cases, and the ones with combined
time and parking `step_size`.

## Checking a CDR against itself

```console
$ ocpi price cdr.json --tariff tariff.json --time-zone Europe/Berlin
```

prints the breakdown and then compares the total with the one the CDR claims. That is the invoice
check, and it is one command.
