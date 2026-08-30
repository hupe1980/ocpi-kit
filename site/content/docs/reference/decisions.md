+++
title = "Design decisions"
weight = 35
description = "Why ocpi-kit is shaped the way it is: the choices the specification leaves open, and the ones it does not make at all."
+++

OCPI leaves a good deal to the implementer. Every choice this crate made in that space is recorded
here with its reasoning. Where the specification is at fault rather than merely quiet, the entry is
in [Spec errata](@/docs/reference/errata.md) instead.

## Types

**`OcpiString<N>` counts Unicode scalar values, not bytes.** The specification is silent. Scalar
values are what a human means by "45 characters", and a peer that sends an accented name should not
be told it is too long. Ingest is lenient; the limit is reported by `validate()`. See
[Parse, validate, construct](@/docs/concepts/parse-validate-construct.md).

**Wire structs are not `#[non_exhaustive]`.** They mirror a published, versioned document: a new
field is a new OCPI version, which is a new module here. Marking them would cost every user
struct-literal construction and exhaustive matching for a flexibility the domain does not have. The
behavioural enums this crate may itself extend — `Bridge`, `RoutingScenario`, `ViolationCode`,
`Unbridgeable` — *are* `#[non_exhaustive]`.

**The open-enum catch-all is `Custom`, not `Other`.** `ImageCategory` and `TokenType` each have a
real spec value called `OTHER`. See [Open enums](@/docs/concepts/enums.md).

**`From<&str>` is lenient; `new()` and `FromStr` are strict.** The builders need an infallible
conversion to be usable, so the strictness guarantee lives at the wire instead
(`ClientConfig::validate_outgoing`, on by default), where it also catches the cross-field rules no
constructor could see.

**`f64` → `Decimal` goes through the shortest round-tripping string.** `Decimal::try_from(f64)` is
not exact; `f64::to_string` prints the decimal the peer actually wrote. See
[Numbers](@/docs/concepts/numbers.md).

**`time` is fully wrapped.** `types::DateTime` exposes no `time` type in its public API — the
conversions to and from `OffsetDateTime` are crate-private — so the backend could be swapped for
`jiff` without that being a breaking change. Callers get whole seconds from `unix_timestamp()` and
full precision through the RFC 3339 text.

**A charging period is a total, not a curve, and the crate will not pretend otherwise.**
`Cdr::period_spans()` closes the interval boundaries, which are unambiguous. Re-cutting those
intervals onto a finer grid is an assumption the CDR does not carry and the specification declines
to make, so it stays with the caller who has to record it. `Session` gets no equivalent: its final
period has no end while the session runs, and the whole list is replaced on every PATCH. See
[Reading a CDR](@/docs/concepts/reading-a-cdr.md).

**`SignedData.url` is modelled as the `string(512)` the specification says, not as a `Url`.** It is
the only URL-shaped field in OCPI that is not the `URL` type, which is `string(255)` — so modelling
it as one rejects a conformant link. See [Spec errata](@/docs/reference/errata.md).

## Versions

**Everything above `transport` speaks the canonical 2.3.0 model.** The alternative — one
version-polymorphic model with serde tricks — hides real semantic differences (`Price`,
`tax_included`), makes lossy conversions implicit and produces worse error messages. Explicit
per-version modules plus conversions are more code and strictly more correct. See
[Versions and conversion](@/docs/concepts/versions.md).

**`client`, `server` and `hub` enable `convert`.** They speak the canonical model, which is only
defensible if they can reach a peer that does not — and most peers do not. Bridging is part of what
makes those layers work, not an optional extra.

**`bridgeable` is the single predicate for "can this build carry a document between these two
versions".** Whether both versions are *modelled* is a different question, and answering that one
instead promises translations that do not exist.

**A merge patch crosses a version boundary by inspection, not conversion.** A patch is not an
object, so it cannot be decoded and re-encoded; but one writing only fields the two versions agree
about means the same thing in both. `ObjectKind::divergent_fields` is the list they disagree about.

**`v2_2_1` implements the 2.2.1-d2 errata semantics.** The bugfix branch is what people deploy.

**OCPI 2.1.1 is modelled and deliberately not bridged.** It has no owner fields on objects, no
routing headers and no `Price`, so carrying an object across that boundary is a decision about a
particular deployment rather than a translation a library can make on its own.

## Server

**A router refuses at start-up to publish a version it cannot write.** The handler traits speak
2.3.0; a router advertising 2.1.1 would answer partners with plausible objects in a shape that
version does not define. See [Server](@/docs/layers/server.md).

**`ServerConfig::receiver_path_prefix` defaults to `Some("receiver")`.** Locations Sender
`{loc}/{evse}/{conn}` and Receiver `{cc}/{party}/{loc}` are both three segments, so no route
ordering can tell them apart. Either publish the Receiver interfaces one segment deeper, or run one
router per role; mounting both with no prefix panics at start-up, in either order.

**A cap is enforced where it is imposed.** `ServerConfig::max_page_limit` clamps in the `Page`
extractor, so a handler is handed a `limit` it can honour literally and `X-Limit` describes what
actually happened rather than annotating a page that was already built.

**A server validates what it is about to send, warns, and sends it.** Refusing a partner's `GET`
because one Location in a page of a hundred has a 46-character name turns your data quality into an
outage on their side; serving it silently is how it becomes their support ticket. It is not a
setting, because a switch that only silences a log line is not one.

**The Sender-side callback paths are fixed by this crate.** OCPI hands the `response_url` shape to
the implementation, and for Charging Profiles that freedom is the only thing that can distinguish
two identically-shaped result bodies. See [Spec errata](@/docs/reference/errata.md).

## Hub

**A hub refuses a request whose headers and method match no routing scenario**, rather than picking
the nearest one. `OcpiError::NotRoutable` carries the specification's own advice: omit the
`OCPI-to-` headers and make it an Open Routing Request. See [Hub](@/docs/layers/hub.md).

**A crossing this build cannot translate is refused rather than relayed**, by default. Handing a
2.1.1 object to a 2.3.0 party produces a document the receiver misreads rather than rejects.
`Unbridgeable::RelayVerbatim` is there for a hub that is deliberately a pipe.

## Tariffs

**Default rounding is half away from zero, applied at the component and then summed.** The
specification says nothing about rounding on purpose. `PricingPolicy` carries the choice and
`CostBreakdown` records what was applied, and the specification's own worked tariff examples pass
with the default.

**`PricingPolicy` separates reported precision from monetary precision.** A duration in hours is a
repeating decimal, so a verbatim measurement is a number JSON cannot hold. `quantity_decimals` is
what the breakdown *says* was measured; money is computed from the exact quantity regardless.

**A period that outlasts its price is priced, not refused.** Erroring out would leave the party with
no number at all for a CDR a partner has already invoiced. The rate that applied when the period
began is the only defensible answer from the data, and the note is what makes it reviewable.

**A `PricingNote` carries a code, not just a sentence.** A reconciliation pipeline has to be able to
ask "how many CDRs this month span a price change?", and grepping English is not a way to do that.
See [Tariffs](@/docs/layers/tariffs.md).

## Verification and packaging

**The conformance runner is read-only and does not exercise registration.** A check that mutates the
peer it is checking cannot be run against production. See
[Conformance](@/docs/layers/conformance.md).

**`MockPeer` belongs in the crate.** The dozen handler traits over a store are the same dozen for
everybody, and each is a chance to get pagination or the ownership rule slightly wrong and then test
against that mistake. Shipping it makes conformance *this* crate's job, which is testable. See
[Testkit](@/docs/layers/testkit.md).

**The no-float rule is enforced by `xtask`, not clippy**, and **a configuration field must be read,
not merely assigned**. Both are explained under
[Spec traceability](@/docs/reference/traceability.md).

**The specification is not vendored.** OCPI is CC BY-ND 4.0, so `specs/` is a gitignored local
checkout. The fixtures extracted from it *are* committed, so the round-trip suite runs everywhere.

**MSRV is a fixed pin per minor release.** A floating "latest stable minus two" lets a patch release
break a consumer's build.

**One crate, not a workspace.** Cargo features give the same dependency isolation and keep
versioning and documentation in one place. The module boundaries are drawn so a split stays
mechanical if compile times ever demand it.

**Types are not generated from JSON schemas.** There are no official OCPI schemas, and the community
ones encode `f64` money and drop `CiString` semantics. This crate generates *tests* from the
specification text instead of *code* — see [Spec traceability](@/docs/reference/traceability.md).

**Pushes are never auto-retried or queued.** The specification says not to; `Resync` is the answer.
See [Client](@/docs/layers/client.md).
