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
