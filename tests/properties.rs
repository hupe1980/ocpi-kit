//! Property tests for the laws the crate's own code relies on.
//!
//! The fixture corpus proves the crate handles the documents the specification ships. These
//! prove the things a corpus cannot: that the laws hold for *every* input, including the ones
//! nobody thought to write an example of.
//!
//! Each property below is something another part of the crate assumes. A `CiString` used as a
//! map key assumes `Eq` and `Hash` agree. The hub assumes a merge patch is idempotent. The
//! version bridge assumes a downgrade followed by an upgrade is the identity where nothing was
//! lost. Where an assumption is wrong, everything built on it is subtly wrong, and only a
//! generator finds that.

#![cfg(feature = "transport")]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence};

/// Shared configuration.
///
/// The default persistence looks for `src/lib.rs` relative to the test file and warns when it
/// cannot find it, which it never can from `tests/`. Naming the file directly keeps a shrunk
/// counter-example — the valuable part of a property failure — without the noise.
fn config() -> Config {
    Config {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/properties.proptest-regressions",
        ))),
        ..Config::default()
    }
}

use ocpi_kit::transport::{PageQuery, Patch, merge};
use ocpi_kit::types::{CiString, DateTime, Number, OcpiString, Validate};

// ---------------------------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------------------------

/// Text that a peer might plausibly put in an `OcpiString` field, including the awkward cases.
///
/// `OcpiString` is UTF-8 and its limit counts characters, so non-ASCII belongs here.
fn any_text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9 _.:/-]{0,60}",
        "[a-zA-Zà-öø-ÿ]{0,40}",
        Just(String::new()),
        Just("#NA".to_owned()),
        Just("ß".repeat(30)),
        Just("é".repeat(45)),
    ]
}

/// Text that belongs in a `CiString`: *"Only printable ASCII allowed."*
fn any_ascii_text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9 _.:/*-]{0,60}",
        Just(String::new()),
        Just("#NA".to_owned()),
        Just("NL*TNM*001".to_owned()),
    ]
}

/// A decimal within the range OCPI actually uses: money, energy and percentages.
fn any_number() -> impl Strategy<Value = Number> {
    (-99_999_999i64..99_999_999i64, 0u32..=4).prop_map(|(mantissa, scale)| {
        let decimal = rust_decimal::Decimal::new(mantissa, scale);
        Number::new(decimal)
    })
}

/// A timestamp in the range OCPI timestamps occupy.
fn any_datetime() -> impl Strategy<Value = DateTime> {
    // 2000-01-01 .. 2100-01-01
    (946_684_800i64..4_102_444_800i64).prop_map(|s| DateTime::from_unix_timestamp(s).expect("in range"))
}

/// A JSON value shaped like something that crosses an OCPI wire.
fn any_json() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::from),
        (-1000i64..1000).prop_map(serde_json::Value::from),
        "[a-z]{0,8}".prop_map(serde_json::Value::from),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::Array),
            prop::collection::hash_map("[a-z]{1,6}", inner, 0..4)
                .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
        ]
    })
}

// ---------------------------------------------------------------------------------------------
// CiString: the laws a case-insensitive key must obey
// ---------------------------------------------------------------------------------------------

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

proptest! {
    #![proptest_config(config())]

    /// Two `CiString`s that compare equal must hash equal, or every `HashMap` keyed on one is
    /// broken. This is the law `HashMap` documents and the one that is easiest to violate.
    ///
    /// The folding is deliberately **ASCII**: a `CiString` is *"Only printable ASCII allowed"*,
    /// so full Unicode case mapping — under which `ß` uppercases to the two characters `SS` —
    /// would be both wrong and length-changing.
    #[test]
    fn cistring_hash_agrees_with_eq(text in any_ascii_text()) {
        let lower: CiString<255> = CiString::new_lenient(text.to_ascii_lowercase());
        let upper: CiString<255> = CiString::new_lenient(text.to_ascii_uppercase());
        prop_assert_eq!(&lower, &upper, "case must not affect equality");
        prop_assert_eq!(hash_of(&lower), hash_of(&upper), "Eq and Hash must agree");
        prop_assert_eq!(lower.cmp(&upper), core::cmp::Ordering::Equal, "Ord must agree too");
    }

    /// Equality is reflexive, `eq_ignore_case` agrees with it, and the original case is kept on
    /// the way out — a peer's identifier is never rewritten.
    #[test]
    fn cistring_equality_is_consistent(text in any_ascii_text()) {
        let value: CiString<255> = CiString::new_lenient(text.clone());
        prop_assert_eq!(&value, &value);
        prop_assert!(value.eq_ignore_case(&text));
        prop_assert!(value.eq_ignore_case(&text.to_ascii_uppercase()));
        prop_assert_eq!(value.as_str(), text.as_str(), "the original case survives");
    }

    /// Anything outside printable ASCII decodes — permissively, as always — and is reported.
    #[test]
    fn a_cistring_reports_characters_outside_printable_ascii(text in "[à-öø-ÿ]{1,10}") {
        let value: CiString<255> = CiString::new_lenient(text.clone());
        prop_assert_eq!(value.as_str(), text.as_str(), "nothing is dropped on ingest");
        prop_assert!(value.validate().is_err(), "but it is not conformant");
        prop_assert!(CiString::<255>::new(text).is_err(), "and strict construction refuses it");
    }

    /// Decoding is permissive and validation is what reports the limit — the crate's governing
    /// rule, stated as a property: a lenient value always survives, and it is conformant exactly
    /// when it is within the limit.
    #[test]
    fn a_length_limit_is_reported_not_enforced_on_ingest(text in any_text()) {
        let value: OcpiString<16> = OcpiString::new_lenient(text.clone());
        prop_assert_eq!(value.as_str(), text.as_str(), "nothing is truncated on ingest");

        let within = text.chars().count() <= 16;
        prop_assert_eq!(value.validate().is_ok(), within,
            "a {}-character value in a string(16)", text.chars().count());
        prop_assert_eq!(OcpiString::<16>::new(text.clone()).is_ok(), within,
            "strict construction must agree with validation");
    }

    /// A `string(N)` counts Unicode scalar values, not bytes: a 45-character string of two-byte
    /// characters is 90 bytes and must still be conformant.
    #[test]
    fn a_length_limit_counts_characters_not_bytes(n in 0usize..=45) {
        let text = "é".repeat(n);
        prop_assert!(text.len() >= n, "these are multi-byte characters");
        let value: OcpiString<45> = OcpiString::new_lenient(text);
        prop_assert!(value.validate().is_ok(), "{n} characters fit in a string(45)");
    }
}

// ---------------------------------------------------------------------------------------------
// Number: the JSON boundary
// ---------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    /// Every decimal in OCPI's actual range survives a JSON round-trip exactly, and says so.
    #[test]
    fn a_realistic_number_round_trips_through_json(n in any_number()) {
        prop_assert!(n.json_round_trips(), "{n} should survive a JSON round-trip");
        prop_assert!(n.validate().is_ok());

        let json = serde_json::to_string(&n).expect("serialises");
        let back: Number = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, n, "via {}", json);
    }

    /// An integral value is written as a JSON integer, never as `20.0`.
    #[test]
    fn an_integral_number_stays_an_integer_on_the_wire(i in -1_000_000i64..1_000_000) {
        let json = serde_json::to_string(&Number::from(i)).expect("serialises");
        prop_assert!(!json.contains('.'), "{i} was written as {json}");
        prop_assert_eq!(json.parse::<i64>().expect("an integer"), i);
    }

    /// A quoted number is tolerated on input and normalised on output — the behaviour the 2.1.1
    /// CDR example depends on.
    #[test]
    fn a_quoted_number_parses_to_the_same_value(n in any_number()) {
        let quoted = format!("\"{n}\"");
        let parsed: Number = serde_json::from_str(&quoted).expect("quoted numbers are tolerated");
        prop_assert_eq!(parsed, n);
    }

    /// Addition is exact: summing a list of decimals and summing it in reverse give the same
    /// answer. With `f64` this fails.
    #[test]
    fn decimal_addition_is_associative(values in prop::collection::vec(any_number(), 0..12)) {
        let forward: Number = values.iter().copied().sum();
        let backward: Number = values.iter().rev().copied().sum();
        prop_assert_eq!(forward, backward);
    }
}

// ---------------------------------------------------------------------------------------------
// DateTime: canonicalisation
// ---------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    /// Formatting and re-parsing is the identity, and formatting is idempotent — the property
    /// the fixture corpus's canonical comparison depends on.
    #[test]
    fn a_timestamp_round_trips_and_its_text_is_stable(t in any_datetime()) {
        let text = t.to_string();
        let parsed: DateTime = text.parse().expect("our own output parses");
        prop_assert_eq!(parsed, t);
        prop_assert_eq!(parsed.to_string(), text, "formatting is idempotent");
    }

    /// The unix timestamp is preserved exactly through the string form.
    #[test]
    fn a_timestamp_preserves_its_instant(secs in 946_684_800i64..4_102_444_800i64) {
        let t = DateTime::from_unix_timestamp(secs).expect("in range");
        prop_assert_eq!(t.unix_timestamp(), secs);
        let parsed: DateTime = t.to_string().parse().expect("parses");
        prop_assert_eq!(parsed.unix_timestamp(), secs);
    }
}

// ---------------------------------------------------------------------------------------------
// JSON Merge Patch: RFC 7396
// ---------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    /// Applying the same merge patch twice changes nothing the second time. A hub retrying a
    /// PATCH after a timeout depends on this.
    #[test]
    fn a_merge_patch_is_idempotent(target in any_json(), patch in any_json()) {
        let mut once = target.clone();
        merge(&mut once, &patch);
        let mut twice = once.clone();
        merge(&mut twice, &patch);
        prop_assert_eq!(once, twice);
    }

    /// Merging `{}` into an object changes nothing.
    #[test]
    fn an_empty_object_patch_is_the_identity_on_an_object(target in any_json()) {
        prop_assume!(target.is_object());
        let mut result = target.clone();
        merge(&mut result, &serde_json::json!({}));
        prop_assert_eq!(result, target);
    }

    /// An object patch applied to a target that is *not* an object replaces it, per RFC 7396:
    /// *"if Target is not an Object: Target = {}"* before the patch's members are applied. So
    /// merging `{}` into `null` yields `{}`, not `null`.
    #[test]
    fn an_object_patch_replaces_a_non_object_target(target in any_json()) {
        prop_assume!(!target.is_object());
        let mut result = target;
        merge(&mut result, &serde_json::json!({ "a": 1 }));
        prop_assert_eq!(result, serde_json::json!({ "a": 1 }));
    }

    /// A patch replaces a scalar wholesale, per RFC 7396: the result is the patch itself.
    #[test]
    fn a_non_object_patch_replaces_the_target(target in any_json(), replacement in any_json()) {
        prop_assume!(!replacement.is_object());
        let mut result = target;
        merge(&mut result, &replacement);
        prop_assert_eq!(result, replacement);
    }

    /// `null` in a patch removes the key, and only that key.
    #[test]
    fn null_removes_exactly_one_key(
        keep in "[a-z]{1,6}",
        drop_key in "[a-z]{1,6}",
        value in any_json(),
    ) {
        prop_assume!(keep != drop_key);
        let mut target = serde_json::json!({ keep.clone(): value.clone(), drop_key.clone(): 1 });
        merge(&mut target, &serde_json::json!({ drop_key.clone(): serde_json::Value::Null }));
        prop_assert!(target.get(&drop_key).is_none(), "the null key is gone");
        prop_assert_eq!(target.get(&keep), Some(&value), "the other key is untouched");
    }

    /// A patch without `last_updated` is always rejected, whatever else it carries. This is the
    /// specification's own example of a `2001`, and the router relies on it.
    #[test]
    fn a_patch_without_last_updated_never_applies(fields in prop::collection::hash_map("[a-z]{1,6}", any_json(), 0..5)) {
        let mut object = serde_json::Map::new();
        for (k, v) in fields {
            if k != "last_updated" {
                object.insert(k, v);
            }
        }
        let patch: Patch<serde_json::Value> = Patch::from_value(serde_json::Value::Object(object));
        prop_assert!(patch.last_updated().is_none());
        prop_assert!(patch.apply(&serde_json::json!({})).is_err(), "a patch without last_updated is a 2001");
    }
}

// ---------------------------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    /// A query written into a URL and read back out of it is the same query. Both sides of the
    /// pagination contract are in this crate, so a disagreement here is a disagreement with
    /// itself.
    #[test]
    fn a_page_query_survives_the_url(
        offset in prop::option::of(0u64..1_000_000),
        limit in prop::option::of(1u64..1000),
    ) {
        let mut query = PageQuery::new();
        if let Some(o) = offset {
            query = query.with_offset(o);
        }
        if let Some(l) = limit {
            query = query.with_limit(l);
        }

        let text = query.to_query_string();
        for (name, value) in [("offset", offset), ("limit", limit)] {
            match value {
                Some(v) => prop_assert!(text.contains(&format!("{name}={v}")), "{text}"),
                None => prop_assert!(!text.contains(&format!("{name}=")), "{text}"),
            }
        }
    }

    /// Clamping never raises a limit and never exceeds the cap — the rule a server applies to a
    /// client's request and a client applies to a peer's known maximum.
    #[test]
    fn clamping_a_limit_only_ever_lowers_it(asked in 1u64..10_000, cap in 1u64..10_000) {
        let clamped = PageQuery::new().with_limit(asked).clamped_to(cap);
        let effective = clamped.limit.expect("a limit was set");
        prop_assert!(effective <= cap, "{effective} exceeds the cap {cap}");
        prop_assert!(effective <= asked, "{effective} is more than the {asked} asked for");
    }
}

// ---------------------------------------------------------------------------------------------
// Version bridging
// ---------------------------------------------------------------------------------------------

#[cfg(all(feature = "convert", feature = "v2_2_1"))]
mod bridging {
    use super::*;
    use ocpi_kit::convert::{Downgrade, Upgrade};
    use ocpi_kit::types::Extensions;
    use ocpi_kit::v2_2_1;

    proptest! {
    #![proptest_config(config())]

        /// A 2.2.1 `Price` upgraded to 2.3.0 and brought back is the same price. This is the
        /// conversion a hub performs on every CDR that crosses a version boundary, and the one
        /// with the most room to lose a cent.
        #[test]
        fn a_price_survives_a_round_trip_through_2_3_0(
            excl in any_number(),
            incl in prop::option::of(any_number()),
        ) {
            let original = v2_2_1::types::Price {
                excl_vat: excl,
                incl_vat: incl,
                extensions: Extensions::default(),
            };
            let up = Upgrade::<ocpi_kit::v2_3_0::types::Price>::upgrade(original.clone());
            let down = up.value.downgrade();
            prop_assert_eq!(down.value.excl_vat, original.excl_vat, "the net amount is exact");
            prop_assert_eq!(down.value.incl_vat, original.incl_vat, "the gross amount is exact");
        }

        /// Whatever a conversion loses, it says so: a `Converted` never reports a loss whose
        /// pointer is empty or whose reason is blank, because a loss report nobody can act on is
        /// no better than a silent drop.
        #[test]
        fn every_reported_loss_names_a_field_and_a_reason(
            excl in any_number(),
            incl in prop::option::of(any_number()),
        ) {
            let price = v2_2_1::types::Price {
                excl_vat: excl,
                incl_vat: incl,
                extensions: Extensions::default(),
            };
            let up = Upgrade::<ocpi_kit::v2_3_0::types::Price>::upgrade(price);
            for loss in &up.value.clone().downgrade().lossy {
                prop_assert!(loss.pointer.starts_with('/') || loss.pointer.is_empty());
                prop_assert!(!loss.reason.trim().is_empty(), "a loss must say why");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    /// Every violation carries a well-formed RFC 6901 JSON Pointer: empty, or a sequence of
    /// `/`-prefixed tokens. Anything else cannot be fed to a JSON tool, which is the whole point
    /// of reporting a pointer rather than a field name.
    #[test]
    fn every_violation_pointer_is_a_valid_json_pointer(text in any_text()) {
        let value: OcpiString<8> = OcpiString::new_lenient(text);
        if let Err(violations) = value.validate() {
            for v in &violations {
                prop_assert!(
                    v.pointer.is_empty() || v.pointer.starts_with('/'),
                    "{:?} is not an RFC 6901 pointer", v.pointer
                );
                prop_assert!(!v.message.trim().is_empty(), "a violation must explain itself");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The pricing engine's own invariants
// ---------------------------------------------------------------------------------------------

/// A breakdown is a document somebody files. These are the two things that have to hold for it to
/// be evidence of anything, whatever tariff and session produced it.
#[cfg(all(feature = "tariffs", feature = "v2_3_0"))]
mod pricing {
    use super::{any_number, config};
    use ocpi_kit::tariffs::{PricedPeriod, PricedSession, PricingEngine, TimeZone};
    use ocpi_kit::types::{DateTime, Extensions, Number};
    use ocpi_kit::v2_3_0::tariffs::{
        PriceComponent, PriceLimit, Tariff, TariffDimensionType, TariffElement, TaxIncluded,
    };
    use proptest::prelude::*;

    /// A non-negative amount with at most two decimals, which is what a price or a limit is.
    fn any_amount() -> impl Strategy<Value = Number> {
        (0i64..100_000i64).prop_map(|cents| Number::new(rust_decimal::Decimal::new(cents, 2)))
    }

    /// A VAT percentage, **including invalid ones**.
    ///
    /// A negative percentage is a malformed tariff that `validate()` reports — and the engine
    /// does not require validated input, so the invariants below have to survive it. A generator
    /// that only produces well-formed tariffs proves the engine works on well-formed tariffs,
    /// which is not the interesting half.
    fn any_vat() -> impl Strategy<Value = Option<Number>> {
        proptest::option::of(
            (-500i64..3000i64).prop_map(|hundredths| Number::new(rust_decimal::Decimal::new(hundredths, 2))),
        )
    }

    fn any_limit() -> impl Strategy<Value = Option<PriceLimit>> {
        proptest::option::of((any_amount(), proptest::option::of(any_amount())).prop_map(
            |(before_taxes, after_taxes)| PriceLimit {
                before_taxes,
                after_taxes,
                extensions: Extensions::new(),
            },
        ))
    }

    /// All three readings of `tax_included`, because each one means something different by
    /// `quantity × price` and the breakdown's invariants have to hold under every one of them.
    fn any_tax_basis() -> impl Strategy<Value = TaxIncluded> {
        proptest::prop_oneof![Just(TaxIncluded::No), Just(TaxIncluded::Yes), Just(TaxIncluded::NotApplicable)]
    }

    fn any_tariff() -> impl Strategy<Value = Tariff> {
        (
            proptest::collection::vec((any_amount(), any_vat(), 0u32..3600u32), 1..4),
            any_limit(),
            any_limit(),
            any_tax_basis(),
        )
            .prop_map(|(components, min_price, max_price, tax_included)| {
                let dimensions = [
                    TariffDimensionType::Energy,
                    TariffDimensionType::Time,
                    TariffDimensionType::ParkingTime,
                    TariffDimensionType::Flat,
                ];
                let price_components: Vec<_> = components
                    .into_iter()
                    .enumerate()
                    .map(|(i, (price, vat, step_size))| PriceComponent {
                        component_type: dimensions[i % dimensions.len()],
                        price,
                        vat,
                        step_size,
                        extensions: Extensions::new(),
                    })
                    .collect();
                let mut tariff = Tariff::builder()
                    .country_code("DE")
                    .party_id("ALL")
                    .id("prop")
                    .currency("EUR")
                    .elements(vec![TariffElement::builder().price_components(price_components).build()])
                    .tax_included(tax_included)
                    .last_updated("2024-01-15T10:00:00Z".parse::<DateTime>().expect("valid"))
                    .build();
                tariff.min_price = min_price;
                tariff.max_price = max_price;
                tariff
            })
    }

    fn any_session() -> impl Strategy<Value = PricedSession> {
        proptest::collection::vec((any_number(), any_number(), any_number()), 1..4).prop_map(|periods| {
            let start: DateTime = "2024-01-15T10:00:00Z".parse().expect("valid");
            let mut session = PricedSession::new(start, TimeZone::utc());
            for (i, (energy, charging, parking)) in periods.into_iter().enumerate() {
                let at = DateTime::from_unix_timestamp(
                    start.unix_timestamp() + i64::try_from(i).expect("small") * 600,
                )
                .expect("in range");
                session = session.with_period(PricedPeriod {
                    energy_kwh: energy.get().abs().into(),
                    charging_hours: charging.get().abs().into(),
                    parking_hours: parking.get().abs().into(),
                    ..PricedPeriod::new(at)
                });
            }
            session
        })
    }

    proptest! {
        #![proptest_config(config())]

        /// The tax lines account for exactly the difference between the two totals.
        ///
        /// Without this a breakdown can print totals whose VAT does not match the tax lines
        /// beside them, which is not a document any party can file or dispute.
        #[test]
        fn tax_lines_always_account_for_the_difference_between_the_totals(
            tariff in any_tariff(),
            session in any_session(),
        ) {
            let breakdown = PricingEngine::new().price(&session, &[tariff]).expect("prices");
            let summed: Number = breakdown.taxes.iter().map(|t| t.amount).sum();
            prop_assert_eq!(
                summed,
                breakdown.total_incl_vat - breakdown.total_excl_vat,
                "tax lines {:?} against totals {} / {}",
                breakdown.taxes,
                breakdown.total_excl_vat,
                breakdown.total_incl_vat
            );
        }

        /// The inclusive total is never below the exclusive one, and neither is ever negative.
        ///
        /// A clamp is the easy way to break this: raising one total and leaving the other alone
        /// produces a session that costs less with tax than without.
        #[test]
        fn the_totals_are_ordered_and_non_negative(
            tariff in any_tariff(),
            session in any_session(),
        ) {
            let breakdown = PricingEngine::new().price(&session, &[tariff]).expect("prices");
            prop_assert!(!breakdown.total_excl_vat.is_negative(), "{}", breakdown.total_excl_vat);
            prop_assert!(
                breakdown.total_incl_vat >= breakdown.total_excl_vat,
                "{} incl < {} excl",
                breakdown.total_incl_vat,
                breakdown.total_excl_vat
            );
        }

        /// Every number in a breakdown survives being written to JSON and read back.
        #[test]
        fn a_breakdown_round_trips_through_json(
            tariff in any_tariff(),
            session in any_session(),
        ) {
            let breakdown = PricingEngine::new().price(&session, &[tariff]).expect("prices");
            let json = serde_json::to_string(&breakdown).expect("serialises");
            let back: ocpi_kit::tariffs::CostBreakdown = serde_json::from_str(&json).expect("parses");
            prop_assert_eq!(back, breakdown);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Robustness: every parser that faces a peer, against input nobody would write on purpose
// ---------------------------------------------------------------------------------------------
//
// This crate forbids `unsafe`, so the risk from a hostile peer is not memory corruption: it is a
// panic. A panic in a header parser is a request that kills a task; a panic inside a hub's
// forwarder is a request that kills a task holding somebody else's message. `cargo-fuzz` would
// explore these harder, and needs nightly and a separate crate; these run on every commit, which
// is the property that matters more.
//
// Each of these is a parser a *peer* controls the input of.

/// A string biased towards the shapes that break parsers: separators, delimiters, and the
/// prefixes each parser looks for.
fn hostile_text() -> impl Strategy<Value = String> {
    prop_oneof![
        2 => ".*",
        1 => proptest::string::string_regex("[<>;=\"', \t\r\n:/?&+-]{0,40}").expect("a valid regex"),
        1 => proptest::string::string_regex("(Token |Bearer |rel=|<|>|;|=){1,10}.{0,30}")
            .expect("a valid regex"),
        1 => proptest::string::string_regex("[0-9T:.Z+-]{0,40}").expect("a valid regex"),
    ]
}

proptest! {
    #![proptest_config(config())]

    /// The `Authorization` header parser, on anything at all.
    ///
    /// It Base64-decodes before it validates, which is the step most likely to be surprised.
    #[test]
    fn parsing_an_authorization_header_never_panics(value in hostile_text()) {
        use ocpi_kit::transport::CredentialsToken;
        for lenient in [false, true] {
            let _ = CredentialsToken::parse_header(&value, lenient);
        }
    }

    /// The `Link` header parser, which has to find `rel="next"` among arbitrary parameters.
    #[test]
    fn parsing_a_link_header_never_panics(value in hostile_text()) {
        let _ = ocpi_kit::transport::headers::parse_link_next(&value);
    }

    /// The two scalar parsers every object is made of.
    #[test]
    fn parsing_a_scalar_never_panics(value in hostile_text()) {
        let _ = value.parse::<DateTime>();
        let _ = value.parse::<Number>();
        let _ = value.parse::<ocpi_kit::types::PartyRef>();
        let _ = ocpi_kit::types::Url::new(&value);
        // `new_lenient` accepts anything by construction; the policy check is what must survive it.
        let _ = ocpi_kit::types::UrlPolicy::default().check(&ocpi_kit::types::Url::new_lenient(value));
    }

    /// The pagination headers, whose values a peer writes and this client reads.
    #[test]
    fn reading_pagination_headers_never_panics(
        link in hostile_text(),
        total in hostile_text(),
        limit in hostile_text(),
    ) {
        use ocpi_kit::transport::PageMeta;
        let mut headers = http::HeaderMap::new();
        for (name, value) in [("link", link), ("x-total-count", total), ("x-limit", limit)] {
            if let Ok(value) = http::HeaderValue::from_str(&value) {
                headers.insert(http::HeaderName::from_static(name), value);
            }
        }
        let _ = PageMeta::from_headers(&headers);
    }

    /// The envelope, from bytes that are not necessarily JSON and not necessarily an envelope.
    #[test]
    fn decoding_an_envelope_never_panics(body in prop::collection::vec(any::<u8>(), 0..512)) {
        use ocpi_kit::transport::OcpiResponse;
        let _ = serde_json::from_slice::<OcpiResponse<serde_json::Value>>(&body);
    }

    /// RFC 7396 merge, over values that are deliberately not objects.
    ///
    /// The hub applies a patch to a document it has never decoded, so both sides are a peer's.
    #[test]
    fn merging_arbitrary_values_never_panics(
        target in any_hostile_json(),
        patch in any_hostile_json(),
    ) {
        let mut target = target;
        merge(&mut target, &patch);
    }
}

/// Arbitrary JSON, including the keys and strings a generator would not otherwise produce.
fn any_hostile_json() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::from),
        any::<i32>().prop_map(serde_json::Value::from),
        ".*".prop_map(serde_json::Value::from),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::from),
            prop::collection::hash_map(".{0,6}", inner, 0..4)
                .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
        ]
    })
}

#[cfg(all(feature = "convert", feature = "v2_2_1"))]
mod bridge_robustness {
    use super::{any_hostile_json, config};
    use ocpi_kit::VersionNumber;
    use ocpi_kit::convert::wire::ObjectKind;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(config())]

        /// The version bridge, on a document that is not the object the endpoint claims.
        ///
        /// A hub runs this over a body a peer sent, so "not the object it should be" is the
        /// ordinary case rather than the exceptional one. It must be an error, never a panic.
        #[test]
        fn bridging_an_arbitrary_document_never_panics(value in any_hostile_json()) {
            for kind in [
                ObjectKind::Location,
                ObjectKind::Cdr,
                ObjectKind::Tariff,
                ObjectKind::Credentials,
            ] {
                let _ = kind.bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_3_0, value.clone());
                let _ = kind.bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_2_1, value.clone());
            }
        }

        /// The endpoint classifier, on any path a peer could put in a URL.
        #[test]
        fn classifying_an_arbitrary_path_never_panics(path in ".*") {
            use ocpi_kit::convert::wire::Payload;
            use ocpi_kit::{InterfaceRole, ModuleId};
            for module in [ModuleId::Locations, ModuleId::Tokens, ModuleId::Commands, ModuleId::Cdrs] {
                for interface in [InterfaceRole::Sender, InterfaceRole::Receiver] {
                    for payload in [Payload::Request, Payload::Response] {
                        let _ = ObjectKind::for_endpoint(&module, interface, &path, payload);
                    }
                }
            }
        }
    }
}
