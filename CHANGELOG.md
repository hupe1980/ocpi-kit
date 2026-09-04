# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] — unreleased

An audit release. Every entry below is something the crate got wrong, or something it did not
check, found by re-reading the specification against the code rather than by a bug report.

### Fixed

- **Breaking, and money.** The pricing engine now reads `Tariff.tax_included`. A Tariff whose
  prices already contain the tax was priced as if they did not, and the VAT was then added on top
  a second time — an overcharge of exactly the tax rate on every session under a tax-inclusive
  tariff, which is the ordinary shape in Canada and the United States. `CostBreakdown` carries a
  `tax_basis`, `PricedSegment` carries the `tax` charged on it, and a tariff whose prices include
  tax but name no rate raises a `TaxIncludedWithoutRate` note rather than pretending the gross
  amount is a net one. The specification's own `tariff_19`/`tariff_20` examples are tests.
- **Money.** A quantity that is a repeating decimal is no longer rounded up into an extra
  `step_size` block. 35 minutes is 0.58333…33 h, and `0.58333…33 × 3600` is `2099.999…`, which a
  bare ceiling billed as 36 minutes.
- **Security.** Every identifier this crate puts in a URL path is percent-encoded
  (`Url::join_segment`). OCPI ids are `CiString(36)` with no character restriction and they arrive
  from peers, so a `token_uid` of `../credentials` addressed a different endpoint and one
  containing `?` started a query string. Query parameter values are encoded too
  (`Url::with_param`).
- **Panics.** `DateTime::local_parts` is fallible: shifting a timestamp near the end of the
  representable range by a Location's UTC offset overflowed and panicked, reachable from any CDR
  a peer sends. The pricing engine refuses values it cannot multiply
  (`PricingError::OutOfRange`) rather than panicking inside `rust_decimal`, and `Number` gained
  `checked_add`/`sub`/`mul`/`div`.
- **Panics.** The in-memory token store, the hub's routing table and the testkit stores no longer
  panic on a poisoned lock. One request panicking used to turn a server into one that answered
  every subsequent request with a panic.
- **Breaking.** A router that publishes a non-canonical version refuses at start-up to mount both
  interfaces of any module without a `receiver_path_prefix`. The version-bridging middleware sees
  a path rather than a route, so it could not tell the two apart and translated some bodies with
  the wrong object's rules.
- **Breaking.** Reserved time is priced only by a Tariff Element that carries a `reservation`
  restriction. An unrestricted fallback element used to price it at the charging rate, which
  contradicted the treatment of every *other* non-reservation element and could overcharge a
  driver for time the CPO never published a price for.
- Five fields that never reached the validator now do: `Evse.status` (all three versions), the
  2.1.1 `Session.total_cost`, `Cdr.booking_id`, `ChargingProfile.charging_rate_unit` and
  `TariffRestrictions.booking`. Each was a rule this crate promised and did not keep.
- A `Tariff` downgraded to OCPI 2.2.1 no longer reports a loss it did not suffer. A `PriceLimit`
  and a 2.2.1 `Price` say the same two things, so `min_price`/`max_price` cross with nothing to
  report; the old code recorded a loss whenever `after_taxes` was *absent*, which is the case with
  least to lose. A hub appends its loss report to the `status_message` a partner reads, and a loss
  report that fires on the ordinary case teaches its reader to ignore loss reports.
- `Discovered::best_common_version` prefers a version the typed client can actually use — one
  that bridges to the canonical model — over a merely modelled one, and `select_best` says so
  when only the latter is available. A peer offering nothing but OCPI 2.1.1 used to register
  cleanly and then fail every typed call.

### Added

- **The specification is pinned, and CI reads it.** `spec-sources.toml` names each of the five
  OCPI releases, its branch and its commit; `spec-sources.lock` records a SHA-256 for each of the
  479 files the censuses read. `cargo run -p xtask -- spec-sync` fetches exactly those commits,
  verifies the checkout file by file, re-records the lock, or reports how far upstream has moved
  past the pin. CI now runs the three censuses **for real** on every push — they used to be
  skipped whenever no spec checkout was present, which on a runner was always — and a weekly job
  fails when a pinned branch moves.
- **`cargo run -p xtask -- errata`** re-derives all 19 recorded specification defects from the
  pinned sources: each is a predicate over the text, and one that upstream *fixes* fails the build
  instead of leaving a false claim in the published documentation.
- **Fuzzing.** Six libFuzzer targets under `fuzz/` — `wire`, `envelope`, `patch`, `bridge`,
  `pricing`, `headers` — seeded from the specification's own 279 examples by
  `cargo run -p xtask -- fuzz-corpus`, which also pairs a CDR with a Tariff and a Location with a
  patch for the two targets that read two documents from one input. `pricing` asserts the
  breakdown's own tax invariant on every input that decodes. A nightly workflow runs each target.
- **One test per specification boundary.** `min_kwh` inclusive, `max_kwh` exclusive,
  `min_duration` inclusive, `max_duration` exclusive, `min_power`/`min_current` inclusive,
  `max_power`/`max_current` exclusive, and a price limit that leaves a total sitting exactly on it
  alone. `cargo mutants` found that nothing distinguished any of them from its opposite.
- **`cargo run -p xtask -- validate-coverage`** compares every wire object's fields with the ones
  its `Validate` impl actually checks — 728 fields across 114 objects, type-driven rather than a
  list of names. Its first run found five fields that never reached the validator, `Evse.status`
  among them: the most-watched field in OCPI, and a value a peer sends.
- **`cargo run -p xtask -- endpoints`** compares the URLs this crate builds with the endpoint
  structures the specification writes — the pattern must still be in the chapter character for
  character, and expanding it must give what the builder produces. Sixteen structures; no census
  compared URLs before, and a URL is where an integration breaks.
- **`tariffs::lint`** — the Tariff mistakes that are legal, decode cleanly, validate cleanly and
  still bill the wrong amount: an element made unreachable by an unrestricted one before it, a
  `step_size` that `PARKING_TIME` will always absorb, a restriction window that excludes
  everything, a VAT rate on a tariff that says no tax applies. Fifteen codes, each with an RFC
  6901 pointer. `ocpi lint tariff.json` runs it and exits non-zero, so publishing a tariff can be
  gated on it. It finds one in the specification's own `tariff_13` example.
- **`tariffs::verify_cdr`** — a CDR against what its own Charging Periods and its own Tariff
  produce: the two totals, the per-dimension costs, and the CDR against itself — `total_energy`
  and `total_parking_time` against what its periods add up to, `total_time` against its own two
  timestamps. That second group needs no tariff and admits no interpretation, and it is the check
  that finds a malformed CDR whose total happens to come out right. Where OCPI's own attribution
  is underivable — a `FLAT` fee beside parking or reserved time belongs to one of three fields
  that each claim it — the comparison is skipped rather than reported. `ocpi price` prints all of
  it.
- **`OcpiError::Timeout`**, so a hub answers `4002 Timeout on forwarded request` because
  `reqwest` said it was a timeout rather than because the error message contained the word.
- **A hub fans out concurrently.** A Broadcast Push or a GET All to fifty platforms no longer
  takes the sum of fifty timeouts; `Forwarder::with_concurrency` bounds it, default 16.
- **Two conformance checks**: that every advertised endpoint URL is one a partner may actually
  call (a version-details document generated behind a reverse proxy publishes `http://10.0.0.5:…`
  to the whole network), and that a `Link: rel="next"` stays on the same host.
- **A mechanical no-silent-drop test**: every leaf of every 2.3.0 fixture that a downgrade does
  not carry to 2.2.1 must be named in the loss report. A new field modelled without a matching
  `lossy.record` fails on its first run.
- The pricing property tests generate all three readings of `tax_included`, so the breakdown's
  invariants are proven under each.

### Changed

- **Breaking.** The enums a peer *fills in* are now decoded leniently and reported rather than
  refused: `Status`, `SessionStatus`, `AuthMethod`, `ConnectionStatus`, `TariffType`,
  `WhitelistType`, `PowerType`, `ConnectorFormat`, `EvsePosition`, `ParkingDirection` and
  `EnergySourceCategory` gain a `Custom(String)` variant, keep the wire value verbatim, and are
  reported by `Validate` — the same treatment the enums OCPI 2.3.0 *opened* already had. One EVSE
  reporting a status or a power type this version does not define no longer costs a CPO its whole
  Locations page. The enums a value of which drives a decision — the pricing engine's inputs,
  `DayOfWeek`, `InterfaceRole`, `Role` and the command, charging-profile, booking and payment
  outcome types — stay closed, and stay `Copy`.
- **Breaking, and structural.** OCPI 2.3.0 is published as a **core release plus two branches**,
  and in July 2026 upstream moved the Payments module — and `Tariff.preauthorize_amount` with it —
  out of core onto the `payments` branch, while moving Invoice Reconciliation *into* core. In
  place, on released branches, with no version bump. This crate's features now mirror that: there
  is a `payments` feature (like `bookings`), the `invoice-reconciliation` feature is **gone**
  because the module is core, and `Tariff.preauthorize_amount` is behind `payments`. The Payments
  examples move to their own fixture corpus, and the field census records the field as
  branch-only — it was declared the other way round until the pin caught up.
- **Breaking.** `PricedSegment` gains `tax` and `tax_basis`; `AppliedComponent` gains
  `reservation`; `CostBreakdown` gains `tax_basis`. The audit artefact says what was charged
  rather than leaving it to be recomputed from a percentage that no longer determines it.
- **Breaking.** `Url::join` is documented as taking an already-encoded path — it is what a hub
  forwards with — and `Url::join_segment` is the one to use for anything that came out of a JSON
  document.
- `OcpiJson` decodes a request body once instead of twice. The two passes existed to tell a
  syntax error from a data error, which `serde_json::Error::classify` answers directly.

## [0.2.0] — 2026-08-30

### Added

**Reading a CDR.** `Cdr::period_spans()` closes the charging-period boundaries a CDR leaves
implicit — a period carries only its `start_date_time`, and the last one ends at the CDR's
`end_date_time`. Each `PeriodSpan` answers `volume(dimension)` and `duration_seconds()`.
`Cdr::delivery_latency_seconds()` measures how late a CDR arrived, returning `None` for the
`1970-01-01` placeholder timestamps the specification permits so they cannot poison an average.
`SignedData::value_for()`, `start_value()` and `end_value()` look a signed reading up by nature,
case-insensitively.

**`ContractIdParts`** parses and normalises eMI3/IDACS eMAIDs, whose separators are optional and
all-or-nothing: `DE-8AA-CA2B3C4D5-N` and `DE8AACA2B3C4D5N` are one contract, and comparing the
strings says otherwise.

**`DateTime::local_parts(offset_seconds)`** returns `LocalParts { date, time, iso_weekday }` in the
crate's own types, for the time-of-day rules OCPI writes in a Location's local time.

**`cargo xtask field-shapes`** checks 1,385 field cardinalities and string lengths against the
specification's property tables across all five modelled releases. The two existing censuses read a
property table's name and value columns; nothing read `Card.` or the length.

### Changed

- **Breaking.** `tariffs::TimeZone::to_local` returns `LocalParts` instead of
  `time::OffsetDateTime`.
- **Breaking.** `DateTime::as_offset_date_time`, `DateTime::from_utc` and `LocalDate::from_date` are
  crate-private, and `LocalDate::to_date` is removed. `time` no longer appears anywhere in the
  public API, so the backend can be replaced without that being a breaking change.

### Fixed

- **Breaking.** `SignedData.url` is `Option<OcpiString<512>>`, not `Option<Url>`. The specification
  types this field `string(512)` — it is the only URL-shaped field in OCPI that is not the `URL`
  type, which is `string(255)`. Modelled as a `Url`, a conformant 300-character link was reported
  `TooLong` by `validate()`, and since `ClientConfig::validate_outgoing` is on by default, a client
  could not send the CDR carrying it. Found by `xtask field-shapes`.
- A rustdoc `doc(cfg)` attribute on a macro invocation, which generated no documentation and failed
  the docs CI job under `--cfg docsrs`.

## [0.1.0] — 2026-08-30

The first functional release. `0.0.1` reserved the crate name and contained nothing.

### Added

**Wire models** — OCPI 2.3.0 (59 objects, all modules), 2.2.1 (53) and 2.1.1 (33), plus the 2.3.0
`bookings` and `payments` release branches, behind one feature each. 2.3.0 is the canonical model
and the others are defined as deltas from it. Money is `Number`, a `rust_decimal` under the hood; there is no
`f64` in any wire type. `CiString` compares and hashes case-insensitively, as OCPI defines it. Unknown fields are
preserved in `Extensions` rather than dropped, and every object round-trips byte-exactly.

**Parse permissively, validate explicitly** — decoding never fails on a length or a cross-field
rule; `Validate` reports them with an RFC 6901 pointer to the offending field.

**`transport`** — envelope and status codes, `Authorization: Token` credentials handling,
pagination, the `OCPI-to-`/`OCPI-from-` routing headers, endpoint URL builders, and RFC 7396 merge
patch.

**`client`** — async client over `reqwest`, the registration handshake, paginated crawls as a
`Stream`, and a typed client per module. Callers get canonical 2.3.0 objects whatever version the
peer runs.

**`server`** — `axum` router driven by one trait per module and interface, publishing 2.2.1 and
2.3.0 from a single set of handlers.

**`hub`** — routing table, broadcast push, open routing requests, GET-All merge, and version
bridging between parties.

**`convert`** — typed `Upgrade`/`Downgrade` with loss accounting, plus the JSON-level bridge keyed
by endpoint that the client, server and hub use to talk to a peer on another version.

**`tariffs`** — auditable pricing engine over CDRs and Sessions. `PricingPolicy` carries the
rounding and precision choices the specification deliberately leaves open; `CostBreakdown` records
what was applied, and `PricingNote` carries a machine-readable code rather than a sentence. The
specification's own ten worked tariff examples are tests.

**Conformance runner** — drives a live peer through the specification's rules, read-only, so it can
be run against production.

**`testkit`** — validated sample objects, in-memory stores with spec-accurate pagination, and
`MockPeer`: a complete conformant OCPI party to point a partner's client at.

**`schema`** — `JsonSchema` for every wire type. **`cli`** — the `ocpi` tool: `validate`, `versions`,
`pull`, `price`, `convert`, `conformance`, `serve-mock`, `schema`.

**Spec traceability** — `cargo xtask` censuses the specification and fails on drift:
`spec-coverage` for field names, `enum-coverage` for enumeration values, `no-floats` for the money
rule, `dead-config` for configuration that is assigned but never read, and `sync-fixtures` to
extract the specification's own examples into the round-trip suite.

## [0.0.1] — 2026-08-30

Name reservation. No functionality.

[0.3.0]: https://github.com/hupe1980/ocpi-kit/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/hupe1980/ocpi-kit/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/hupe1980/ocpi-kit/releases/tag/v0.1.0
