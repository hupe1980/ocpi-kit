# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] — unreleased

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

[0.2.0]: https://github.com/hupe1980/ocpi-kit/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hupe1980/ocpi-kit/releases/tag/v0.1.0
