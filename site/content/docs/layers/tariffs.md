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
│       ├── quantity, price, cost, tax      and tax_basis: how tax_included was read here
│       └── applied: AppliedComponent       tariff id, element index, component index, and `because`
├── total_excl_vat / total_incl_vat
├── taxes: Vec<TaxLine>                     name, percentage, taxable base, amount
├── tax_basis: TaxBasis                     the reading that produced these totals
├── limit_applied: Option<PriceLimitApplied>   min_price / max_price, if either bit
└── notes: Vec<PricingNote>                 code, moment and message — countable, not greppable
```

`AppliedComponent::because` is a sentence, not a code: *why* that Tariff Element matched this
segment. That is the field that ends an invoice dispute.

It serialises, so a disputed invoice becomes a diff of two JSON documents rather than an argument.

**The arithmetic is exact.** Every value is a decimal. There is no `f64` anywhere in this module.
See [Numbers and money](@/docs/concepts/numbers.md).

**It reads `tax_included`.** One field of the parent Tariff decides what `quantity × price` means:

> *`price`: Price per unit for this dimension. This is including or excluding taxes according to
> the `tax_included` field of the Tariff that this PriceComponent is contained in.*

`NO` — the prices are net and the `vat` of each component is added on top. `YES` — they are gross,
and where a rate is named the tax is taken back **out** so the breakdown still has both totals.
`N/A` — no tax applies at all, and a `vat` beside it is a contradiction the Tariff-level field
wins.

Ignoring the field reads a gross amount as a net one and adds the tax a second time, overcharging
every session under a tax-inclusive tariff by exactly the tax rate. `CostBreakdown::tax_basis` says
which reading was used; where the prices include tax and no component names a rate — *"tax rates
are not typically known beforehand to the CPO"* — a `TaxIncludedWithoutRate` note says the two
totals cannot be told apart, rather than passing the gross figure off as a net one.

**The undefined parts are parameters.** The specification says nothing about rounding, on purpose,
and OCPI 3.0 removes `step_size` altogether. Both are settings on `PricingPolicy` rather than
assumptions baked into the code.

**And the breakdown survives being written down.** A duration in hours is a repeating decimal —
eight minutes is 0.1333… — so a report that carried its measurements verbatim would hold values a
JSON number cannot, which this crate's own validator flags as imprecise. `quantity_decimals`
(default 6, enough for a second and a tenth of a watt hour) decides what the breakdown *says* was
measured; money is computed from the exact quantity and rounded separately by `component_decimals`.
Rounding the report and rounding the charge are different decisions, so they are different
settings. A test asserts that a whole breakdown round-trips: an audit artefact that does not
survive being stored is not an audit artefact.

**And it audits the CDR it is pricing.** See below.

## When the CDR is the problem

A Charging Period carries totals, not a curve. There is no way to know how much of its energy fell
before a tariff switched and how much after — so the specification puts the obligation on the CPO:

> *A CPO SHALL at least start (and add) a ChargingPeriod every moment/event that has relevance for
> the total costs of a CDR. … When an energy changes in price after 17:00. The CPO has to start a
> new Charging Period at 17:00.*

Every implementation assumes that holds and prices each period at the rate that applied when it
began. `ocpi-tariffs` documents the assumption plainly: *"No attempt will be made to subdivide or
interpolate data inside a single provided period."* The assumption is right — there is nothing
better to do with the data. The silence is the problem.

This engine re-evaluates the restrictions at the moment each period **ends**. If a different Price
Component would apply by then, the period outlasted its price, and the breakdown says so:

```text
[period_spans_price_change] the ENERGY Charging Period starting here outlasts the Price Component
that prices it: element 1 applies at the start and element 0 by the time the period ends. A CPO
SHALL start a new Charging Period at a price change, so this one should have been split; its
ENERGY is billed in full at the earlier rate, because nothing in the period says how it divides
```

The total beside it is unchanged — nothing is guessed or interpolated. A defect that produces a
*plausible* number becomes a line somebody can act on. A CDR can total correctly by luck and still
be malformed, which is why `ocpi price` exits non-zero on a note as well as on a mismatch.

Two other things are checked the same way: Charging Periods that are not in chronological order
(`step_size` is defined in terms of *"the last relevant PriceComponent"*, so out of order it is
quietly wrong), and dimensions the tariff prices nothing for.

Notes carry a **machine-readable code**, not just a sentence, because a reconciliation pipeline
has to be able to count how many of this month's CDRs span a price change:

```rust
let breakdown = PricingEngine::new().price(&session, &tariffs)?;
if breakdown.needs_review() {
    for note in breakdown.notes_with(PricingNoteCode::PeriodSpansPriceChange) {
        tracing::warn!(at = %note.at.unwrap(), "{}", note.message);
    }
}
```

## The breakdown adds up

The tax lines of a breakdown always sum to exactly `total_incl_vat - total_excl_vat`. That sounds
free and is not — three things break it, and none is visible in an assertion on a total:

* **Precision.** Lines accumulated at `component_decimals` (four places) beside totals rounded to
  `currency_decimals` (two) leave a 2% VAT printing as `500.4720` next to totals differing by
  `500.47`. Half a cent, and an audit finding. Lines are rounded to currency precision, and the
  last one absorbs the residue so they sum to the difference exactly.
* **A price limit.** A `min_price` clamp raises the pre-tax total; leaving the tax lines describing
  what was metered makes a €0.50 session under a €5.00 minimum come out as €5.00 net with **€0.00
  VAT** and a line still claiming €0.105. The clamp moves the lines with the base, in proportion.
* **A malformed tariff.** A negative `vat` percentage describes a session that costs less with tax
  than without. The inclusive total is held at the exclusive one and a `NegativeTax` note names the
  cause, rather than publishing a bill nobody can use.

A property test asserts the invariant over generated tariffs and sessions, and the generator
produces **malformed** tariffs deliberately: one that only produces well-formed input proves the
engine works on well-formed input, which is not the interesting half.

Where a `min_price.after_taxes` demands tax that no rate in the session accounts for, the line is
emitted with `percentage: None` — the amount is a fact, the rate is not knowable, and inventing one
would be a lie in a document somebody files.

## `step_size`, precisely

This is the rule most implementations get wrong, and the spec states it carefully:

* an `ENERGY` `step_size` is applied **once per session**, to the total, not per period
* `TIME` and `PARKING_TIME` `step_size` is applied **once, to the two combined**, and
  `PARKING_TIME` is what absorbs it whenever the session has any — *"In the cases that `TIME` and
  `PARKING_TIME` Tariff Elements are both used, `step_size` is only taken into account for the
  total parking duration"*. The specification's own worked example reaches the same answer by a
  different route (*"the charging duration is not rounded up, as it is followed by another time
  based period"*); the two readings agree on every session that charges and then parks, and this
  engine follows the sentence
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

Two of those need a decision the specification does not make.

**`start_time == end_time` is the whole day.** The spec is silent, and the two readings are far
apart: as an empty interval the element never matches, through the wrap-around rule it always
does. This crate takes the whole day, because it is what the wrap rule produces with no special
case *and* because it fails safe. An element that never matches leaves its dimension with no Price
Component, and the specification's answer to that is that the dimension is free — so the other
reading silently gives the energy away.

**Reserved time is priced separately from charging time, and only by an element that says so.** A
`ChargingPeriod` can carry `TIME` and `RESERVATION_TIME` at once, and the two are priced by
different Tariff Elements. Summing them and looking the total up once would bill the charging
minutes at the reservation rate, usually the dearer of the two. Each is looked up in the context
that describes it, and both appear as their own segment in the breakdown — with
`AppliedComponent::reservation` saying which is which, so a CDR's `total_reservation_cost` can be
checked against something. They still share the `TIME` dimension, and so the one `step_size`
budget the specification allows.

Which element may price reserved time is the specification's one explicit statement on the matter:
*"When this field is present, the TariffElement describes reservation costs."* So an element
**without** a `reservation` restriction — including an unrestricted fallback — does not price it,
and reserved time that nothing prices is free, with a `NoPriceComponent` note. That is what the
specification says an unpriced dimension costs, and it errs towards not billing a driver for
something the CPO never published a price for.

## Multiple tariffs

A Connector can list several `tariff_ids` in preference order, each with its own validity window.
The engine selects per period, records which tariff it chose in the breakdown, and falls back
through the list the way the specification describes. `min_price` and `max_price` (2.3.0's
`PriceLimit`, with `after_taxes`) are applied to the session as a whole.

## Conformance

Ten of the specification's own worked tariff examples are tests in this repository, and they
pass — including the free-of-charge case, the ad-hoc payment cases, and the ones with combined
time and parking `step_size`.

Two more are [snapshots](@/docs/reference/verification.md): the `step_size` example rendered in
full, next to the same session priced under the OCPI 3.0 policy that has no `step_size` at all.
Side by side, that pair is the clearest statement of what block billing costs a driver.

## Checking a CDR

```console
$ ocpi price cdr.json --tariff tariff.json --time-zone Europe/Berlin
```

prints the breakdown and then `verify_cdr`'s report. That comparison is three separate things, and
they fail for different reasons:

1. **The two totals**, against `total_cost`.
2. **The five per-dimension costs** — `total_energy_cost`, `total_time_cost`,
   `total_parking_cost`, `total_fixed_cost`, `total_reservation_cost` — each against the dimension
   it names. Two implementations that disagree on a total usually agree on three dimensions out of
   four, and the fourth is the answer.
3. **The CDR against itself**: `total_energy` and `total_parking_time` against what its own
   Charging Periods add up to, and `total_time` against its own `start_date_time` and
   `end_date_time` — because `total_time` is *"the total duration of the charging session
   (including the duration of charging and not charging)"*, which is the session, not the `TIME`
   dimension.

The third group needs no tariff and admits no interpretation. A CDR whose headline quantities do
not match its own periods or its own clock is malformed whoever prices it, and a total that comes
out right anyway does not make it less so — `CdrVerification::is_self_inconsistent` is the question
to ask when a partner disputes the interpretation but not the data.

One thing is deliberately *not* compared. `total_fixed_cost` excludes *"fixed price components of
parking and reservation"* and `total_parking_cost` and `total_reservation_cost` each include
theirs — and OCPI gives no way to say which `FLAT` component belongs to which. When a session has
a `FLAT` and any parking or reserved time, those three comparisons are skipped rather than
reported as a disagreement this crate cannot adjudicate.

`verify_cdr_within` takes a tolerance, for a contract that allows a rounding difference. The
default is exact, because a cent that appears from nowhere is the thing this exists to find.

## Linting a Tariff

`validate()` answers *"does this object conform to the specification?"*. `lint()` answers the one
that costs money: **does this Tariff say what its author meant?**

```console
$ ocpi lint tariff.json
[unreachable_element] /elements/1/price_components/0: element 0 prices ENERGY and its restrictions
always match, so this Price Component can never apply. The unrestricted element is the fallback
and belongs last
```

Every finding is a Tariff that decodes, validates and prices sessions — just not the way it reads.
The fifteen codes cover:

| Finding | Why it matters |
|---|---|
| `unreachable_element` | An unrestricted element before a restricted one makes the restricted one dead. The commonest way a tiered tariff is written backwards |
| `unused_time_step_size` | A `TIME` `step_size` on a tariff that also prices `PARKING_TIME`, which absorbs all of it. It is present, it looks like it does something, and it does not |
| `impossible_restriction` | `min_kwh` at or above `max_kwh`, and its four siblings: the element can never apply |
| `empty_restrictions` | A `TariffRestrictions` with nothing in it — a fallback that does not read like one |
| `whole_day_window` | `start_time == end_time`, which the specification does not define and this crate reads as the whole day |
| `zero_step_size` | A `step_size` of `0` on a dimension with a unit: no quantisation, where `1` was probably meant |
| `reservation_dimension` | A reservation element pricing `ENERGY` or `PARKING_TIME`, which a reservation cannot have |
| `tax_rate_without_tax` | A `vat` percentage on a Tariff that says no tax applies |
| `tax_included_without_rate` | Prices include tax and nothing says at what rate, so no party can derive a pre-tax total |
| `price_limits_cross`, `never_active`, `implausible_vat`, `duplicate_dimension`, `flat_step_size`, `nothing_priced_by_use` | Each one a statement the Tariff makes that it cannot mean |

`ocpi lint` exits non-zero, so publishing a tariff can be gated on it.

It reports one in the specification's **own** `tariff_13` example: a `TIME` `step_size` of 60 on a
tariff that also prices `PARKING_TIME`. Harmless there — the session charges for exactly 150
minutes — and exactly the shape on which two readings of the `step_size` sentence disagree about a
real one.
