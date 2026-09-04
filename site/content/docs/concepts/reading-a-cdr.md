+++
title = "Reading a CDR"
weight = 60
description = "Period boundaries, signed metering data, token identity and delivery latency — the four things a CDR consumer has to get right."
+++

A CDR is the billing record: *"Because a CDR is for billing purposes, it cannot be changed or
replaced once sent to the eMSP."* Four things about reading one are not obvious from the types.

## Energy over time

A `ChargingPeriod` carries only its `start_date_time` — *"a period ends when the next one
starts"*, and the last one ends at the CDR's `end_date_time`. `Cdr::period_spans()` closes the
intervals for you:

```rust
for span in cdr.period_spans() {
    if let Some(kwh) = span.volume(CdrDimensionType::Energy) {
        println!("{} → {}: {kwh} kWh", span.start, span.end);
    }
}
```

The spans partition the session exactly: the first starts at `start_date_time`, each ends where
the next begins, and the last ends at `end_date_time`.

**A period is a total, not a curve.** It says 4.3 kWh flowed between two instants and nothing
about how. Re-cutting these onto a finer grid — quarter hours for a settlement process, say —
needs an assumption the CDR does not carry, and the specification declines to make it: it puts the
obligation on the CPO to start a new period *"every moment/event that has relevance for the total
costs"* instead. Apportioning by elapsed time is the usual choice and usually close, but it is
your assumption to make and to record. [Tariffs](@/docs/layers/tariffs.md) takes the same position
and reports a `PeriodSpansPriceChange` note when a period outlasts the price that governs it.

Periods come back in the order the CDR gives them. `validate()` reports a CDR whose periods are
out of order — check it first, because for an interval the ordering *is* the meaning.

`Session` carries the same periods and deliberately has no equivalent. A running session has no
`end_date_time`, so its final period has no honest end, and the whole list is provisional — *"any
`charging_periods` from the existing object SHALL be replaced by the `charging_periods` from the
newly received Session object"*. A CDR is the record that stops changing, which is what an interval
needs.

## Signed metering data

`signed_data` is not an opaque blob. It is a `SignedData` object — `encoding_method`,
`public_key`, `url` and a list of `SignedValue { nature, plain_data, signed_data }`. The opaque
part is the strings inside, and those are **carried verbatim, whatever their length**:

```rust
let end = cdr.signed_data.as_ref().and_then(SignedData::end_value);
```

A signed record is evidence; it is worth nothing if a byte moves. Real OCMF payloads routinely run
past the `string(5000)` the specification gives, so the crate's governing rule matters here more
than anywhere: the value arrives intact and `validate()` reports the length as `TooLong` rather
than the decoder truncating or rejecting it. A decode and re-encode reproduces the original bytes
exactly, and a test asserts it against a 6000-character blob.

`value_for(nature)` finds a reading by name and compares case-insensitively; `Start` and `End` have
shortcuts. The nature is deliberately a string, not an enum — *"others might be added later"*.

## Token identity

`CdrToken` identifies the driver with `country_code`, `party_id`, `uid`, `token_type` and
`contract_id`. Two things decide whether a whitelist matches.

**`uid` is already case-folded.** It is a `CiString`, so OCPI itself defines the comparison as
case-insensitive — `Eq` and `Hash` follow, and an RFID UID keyed in a `HashMap` matches
deterministically with no work on your part. See [Parse, validate,
construct](@/docs/concepts/parse-validate-construct.md).

**`contract_id` needs normalising, and case-folding is not enough.** The recommended format is an
eMI3/IDACS eMAID, whose separators are optional and all-or-nothing: `DE-8AA-CA2B3C4D5-N` and
`DE8AACA2B3C4D5N` are the same contract, and comparing the strings says they are not.

```rust
let key = ContractIdParts::normalise(cdr.cdr_token.contract_id.as_str());
```

`None` means the id does not follow the format, which an eMSP is free to do — treat that as "not
comparable", not as "no match". The format has no marker, so any string of the right shape parses;
[the API docs](https://docs.rs/ocpi-kit/latest/ocpi_kit/types/struct.ContractIdParts.html) are
blunt about what that does and does not tell you.

**`auth_method` is not part of the identity.** It records *how* the session was authorised —
`AUTH_REQUEST`, `COMMAND`, `WHITELIST` — not what the token is. The discriminator you want is
`token_type`: `RFID`, `APP_USER`, `AD_HOC_USER`, `EMAID`.

## Delivery latency

A CDR may arrive well after the session it records. `last_updated` is the moment to measure from,
because a CDR has no later one — it cannot be changed once sent, so on a CDR alone `last_updated`
is when it was created.

```rust
match cdr.delivery_latency_seconds() {
    Some(seconds) => …,
    None => …,   // placeholder timestamps; not a measurement
}
```

`None` guards the case that would otherwise poison an average: the specification lets a CPO send
`1970-1-1T00:00:00Z` for `start_date_time` and `end_date_time` when both parties agree, which
would report half a century of latency. Negative values are returned as they are — they mean the
CPO's clock disagrees with its own session, which is worth seeing.

OCPI sets no deadline for delivering a CDR, so there is nothing here to enforce against. If your
process has one, `delivery_latency_seconds()` is the number to build the alert on, and the
[conformance runner](@/docs/layers/conformance.md) will tell you whether a partner's `date_from`
filter works well enough to poll for late arrivals incrementally.

## Local time

Every time-of-day rule in OCPI — tariff restrictions, opening hours — is written in the Location's
local time. `DateTime::local_parts(offset_seconds)` converts, returning `LocalParts { date, time,
iso_weekday }` in the crate's own types:

```rust
let local = cdr.start_date_time.local_parts(3600)?;  // CET
```

The offset has to come from somewhere. A `Location` carries an IANA `time_zone` name, and
`tariffs::TimeZone` resolves it against the zone database — which is what you need for anything
spanning a daylight-saving change, where a fixed offset is wrong for half the year.

It is fallible for one reason: `9999-12-31T23:59:59Z` is a conformant OCPI `DateTime`, a peer can
send one, and shifting it by the offset of a Location in `Pacific/Kiritimati` lands outside the
range a date can hold. Returning an error is the alternative to a panic in the middle of pricing
somebody else's CDR.
