//! Carries every OCPI 2.2.1 example through 2.3.0 and back, and asserts what survives.
//!
//! This is the property a hub depends on: an object that crosses a version boundary twice comes
//! back unchanged, and anything that could not make the trip was *reported* rather than dropped.
//!
//! The corpus is the specification's own examples, so this is not a test against invented data.

#![cfg(all(feature = "v2_2_1", feature = "v2_3_0", feature = "convert"))]

use std::path::Path;

use ocpi_kit::convert::{Downgrade, Lossy, Upgrade};
use ocpi_kit::types::Validate;
use ocpi_kit::{v2_2_1, v2_3_0};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/2.2.1").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Upgrades a 2.2.1 object to 2.3.0 and back, asserting both halves.
///
/// Returns the loss report of the downgrade, so the caller can assert what it expected to cost.
macro_rules! round_trip {
    ($name:expr, $old:ty, $new:ty) => {{
        let original: $old =
            serde_json::from_str(&fixture($name)).unwrap_or_else(|e| panic!("{}: {e}", $name));
        original.validate().unwrap_or_else(|v| panic!("{} is not conformant: {v}", $name));

        let up = Upgrade::<$new>::upgrade(original.clone());
        up.value.validate().unwrap_or_else(|v| panic!("{} upgraded is not conformant 2.3.0: {v}", $name));

        let down = Downgrade::<$old>::downgrade(up.value.clone());
        down.value
            .validate()
            .unwrap_or_else(|v| panic!("{} round-tripped is not conformant 2.2.1: {v}", $name));

        assert_eq!(down.value, original, "{} did not survive a 2.2.1 -> 2.3.0 -> 2.2.1 round trip", $name);
        (up.lossy, down.lossy)
    }};
}

#[test]
fn every_2_2_1_location_example_survives_a_round_trip_through_2_3_0() {
    for name in [
        "location_example.json",
        "location_example_parking_garage_opening_hours.json",
        "location_example_uc2_destination_charger.json",
        "location_example_uc3_destination_charger_not_published.json",
        "location_example_uc4_limited_visibility.json",
        "location_example_uc5_home_charge_point.json",
    ] {
        let (up, down) = round_trip!(name, v2_2_1::locations::Location, v2_3_0::locations::Location);
        assert!(up.is_empty(), "{name}: upgrading a Location loses nothing");
        assert!(down.is_empty(), "{name}: and neither does coming back");
    }
}

#[test]
fn every_2_2_1_tariff_example_survives_a_round_trip_through_2_3_0() {
    for name in [
        "tariff_1_simple_2hour.json",
        "tariff_2_alt_text.json",
        "tariff_3_alt_url.json",
        "tariff_4_complex.json",
        "tariff_5_free_of_charge.json",
        "tariff_6_025kwh_start_max_price.json",
        "tariff_8_simple_025kwh.json",
        "tariff_9_025kwh_start.json",
        "tariff_10_025kwh_parking_start.json",
        "tariff_11_not_possible_alt_text.json",
        "tariff_12_025kwh_min_price.json",
        "tariff_13_simple_3hour_5parking.json",
        "tariff_14_step_size.json",
        "tariff_15_reservation_5_euro_per_hour.json",
        "tariff_16_reservation_2_euro_fee_5_euro_per_hour.json",
        "tariff_17_reservation_with_expire_fee.json",
        "tariff_18_reservation_with_expire_time.json",
        "tariffrestriction_example_max_duration.json",
        "tariffrestriction_example_max_power.json",
    ] {
        let (up, down) = round_trip!(name, v2_2_1::tariffs::Tariff, v2_3_0::tariffs::Tariff);
        assert!(up.is_empty(), "{name}: upgrading a Tariff loses nothing");
        // And neither does coming back, for a Tariff that says its prices exclude tax — which
        // every 2.2.1 Tariff does by definition. `min_price`/`max_price` carry both bounds, so
        // there is nothing here to report; a loss on this path would be a false alarm.
        assert!(down.is_empty(), "{name}: coming back lost {down}");
    }
}

#[test]
fn a_downgraded_tariff_reports_the_tax_information_it_cannot_carry() {
    // A 2.2.1 Tariff upgrades to `tax_included: NO`, which downgrades back cleanly …
    let (_, down) =
        round_trip!("tariff_1_simple_2hour.json", v2_2_1::tariffs::Tariff, v2_3_0::tariffs::Tariff);
    assert!(down.is_empty(), "NO is exactly what a 2.2.1 Tariff means");

    // … but a genuinely tax-inclusive 2.3.0 Tariff cannot be expressed at all.
    let mut inclusive: v2_3_0::tariffs::Tariff = Upgrade::<v2_3_0::tariffs::Tariff>::upgrade(
        serde_json::from_str::<v2_2_1::tariffs::Tariff>(&fixture("tariff_1_simple_2hour.json"))
            .expect("decodes"),
    )
    .value;
    inclusive.tax_included = v2_3_0::tariffs::TaxIncluded::Yes;
    let converted = Downgrade::<v2_2_1::tariffs::Tariff>::downgrade(inclusive);
    assert_reports(&converted.lossy, "/tax_included");
}

#[test]
fn every_2_2_1_session_and_cdr_example_survives_a_round_trip() {
    for name in ["session_example_1_simple_start.json", "session_example_2_short_finished.json"] {
        let (up, down) = round_trip!(name, v2_2_1::sessions::Session, v2_3_0::sessions::Session);
        assert!(up.is_empty() && down.is_empty(), "{name}");
    }
    let (up, _) = round_trip!("cdr_example.json", v2_2_1::cdrs::Cdr, v2_3_0::cdrs::Cdr);
    assert!(up.is_empty(), "upgrading a CDR loses nothing");
}

#[test]
fn every_2_2_1_token_example_survives_a_round_trip() {
    for name in ["token_example_1_app_user.json", "token_example_2_full_rfid.json", "token_put_example.json"]
    {
        let (up, down) = round_trip!(name, v2_2_1::tokens::Token, v2_3_0::tokens::Token);
        assert!(up.is_empty() && down.is_empty(), "{name}");
    }
}

#[test]
fn credentials_carry_the_hub_role_across_in_both_directions() {
    for name in [
        "credentials_example.json",
        "credentials_example2.json",
        "credentials_example3.json",
        "credentials_example4.json",
    ] {
        let original: v2_2_1::credentials::Credentials =
            serde_json::from_str(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let up = original.clone().upgrade();
        assert!(up.lossy.is_empty(), "{name}: no example uses the HUB role");
        let down = Downgrade::<v2_2_1::credentials::Credentials>::downgrade(up.value);
        assert_eq!(down.value, original, "{name}");
        assert!(down.lossy.is_empty(), "{name}");
    }
}

#[test]
fn a_2_3_0_location_downgrades_and_says_what_it_left_behind() {
    // The 2.3.0 example uses every field 2.3.0 added.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/2.3.0/location_example.json");
    let source = std::fs::read_to_string(path).expect("readable");
    let modern: v2_3_0::locations::Location = serde_json::from_str(&source).expect("decodes");
    modern.validate().expect("the spec example is conformant");

    let converted = Downgrade::<v2_2_1::locations::Location>::downgrade(modern.clone());
    converted.value.validate().expect("the downgraded object is conformant 2.2.1");
    assert_reports(&converted.lossy, "/parking_places");
    assert_reports(&converted.lossy, "/evses/0/parking");

    // Everything that *could* cross did: upgrading again restores the fields 2.2.1 still has.
    let back = Upgrade::<v2_3_0::locations::Location>::upgrade(converted.value);
    assert_eq!(back.value.id, modern.id);
    assert_eq!(back.value.evses.len(), modern.evses.len());
    assert_eq!(back.value.help_phone, None, "help_phone genuinely did not survive");
}

#[test]
fn connector_standards_added_after_2_2_1_keep_their_wire_value_through_a_downgrade() {
    let mut modern: v2_3_0::locations::Connector = serde_json::from_str(
        r#"{"id":"1","standard":"IEC_62196_T2","format":"SOCKET","power_type":"DC","max_voltage":920,"max_amperage":400,"last_updated":"2024-01-01T00:00:00Z"}"#,
    )
    .expect("decodes");
    modern.standard = v2_3_0::locations::ConnectorType::Mcs;

    let converted = Downgrade::<v2_2_1::locations::Connector>::downgrade(modern);
    assert!(converted.lossy.is_empty(), "the value crossed intact");
    let json = serde_json::to_string(&converted.value).expect("encodes");
    assert!(json.contains(r#""standard":"MCS""#), "{json}");
    // A 2.2.1 conformance check still says the peer should not have seen this value.
    assert!(converted.value.validate().is_err());
}

/// The pricing engine, fed a 2.2.1 CDR directly and the same CDR carried up to 2.3.0.
///
/// `PricedSession::from_cdr_v2_2_1` is the entry point that makes "price a CDR from either
/// version" true, and it had no test at all — the kind of public API that compiles forever and is
/// never run. The two routes must agree: `Price` is an *output* of pricing, and the charging
/// periods a 2.2.1 CDR carries are wire-identical to a 2.3.0 one's, so a version boundary cannot
/// change what a session cost.
#[test]
#[cfg(feature = "tariffs")]
fn a_2_2_1_cdr_prices_the_same_directly_as_it_does_after_an_upgrade() {
    use ocpi_kit::tariffs::{PricedSession, PricingEngine, TimeZone};

    let old: v2_2_1::cdrs::Cdr = serde_json::from_str(&fixture("cdr_example.json")).expect("decodes");
    let tariff: v2_3_0::tariffs::Tariff = Upgrade::<v2_3_0::tariffs::Tariff>::upgrade(
        old.tariffs.first().expect("the example embeds its tariff").clone(),
    )
    .value;

    let zone = TimeZone::named("Europe/Amsterdam").expect("a real zone");
    let direct = PricedSession::from_cdr_v2_2_1(&old, zone.clone());
    let upgraded = Upgrade::<v2_3_0::cdrs::Cdr>::upgrade(old).value;
    let bridged = PricedSession::from_cdr(&upgraded, zone);

    assert_eq!(direct.periods, bridged.periods, "the periods are the same objects in both versions");

    let engine = PricingEngine::new();
    let a = engine.price(&direct, std::slice::from_ref(&tariff)).expect("prices");
    let b = engine.price(&bridged, &[tariff]).expect("prices");
    assert_eq!(a.total_excl_vat, b.total_excl_vat, "a version boundary cannot change a cost");
    assert_eq!(a.total_incl_vat, b.total_incl_vat);
    assert!(a.total_excl_vat > "0".parse().expect("a number"), "the example is not free");
}

/// The one claim `ObjectKind::divergent_fields` makes, checked against the whole 2.2.1 corpus.
///
/// That list is what decides whether a merge patch can cross a version boundary, and a patch is
/// the one document this crate cannot verify by decoding it. So the list is verified here
/// instead, against the specification's own examples: carry each one to 2.3.0 and back, and no
/// top-level field outside its object's list is allowed to have moved.
///
/// If OCPI 2.3.0 turns out to change a field this list does not name, a PATCH writing that field
/// would be let through and silently mean something else at the far end. This is the test that
/// stops it.
#[test]
fn no_field_outside_the_declared_divergences_moves_across_a_version_boundary() {
    use ocpi_kit::VersionNumber;
    use ocpi_kit::convert::wire::ObjectKind;

    let corpus: &[(&str, ObjectKind)] = &[
        ("location_example.json", ObjectKind::Location),
        ("location_example_parking_garage_opening_hours.json", ObjectKind::Location),
        ("location_example_uc2_destination_charger.json", ObjectKind::Location),
        ("location_example_uc5_home_charge_point.json", ObjectKind::Location),
        ("tariff_4_complex.json", ObjectKind::Tariff),
        ("tariff_6_025kwh_start_max_price.json", ObjectKind::Tariff),
        ("tariff_12_025kwh_min_price.json", ObjectKind::Tariff),
        ("cdr_example.json", ObjectKind::Cdr),
        ("session_example_2_short_finished.json", ObjectKind::Session),
        ("token_example_2_full_rfid.json", ObjectKind::Token),
        ("credentials_example.json", ObjectKind::Credentials),
    ];

    for (name, kind) in corpus {
        let original: serde_json::Value =
            serde_json::from_str(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let up = kind
            .bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_3_0, original)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = kind
            .bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_2_1, up.value.clone())
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        // Both sides of the comparison have been through this crate's own encoder, so a
        // difference is a difference in *meaning* rather than in how a decimal was written.
        let declared = kind.divergent_fields();
        let before = back.value.as_object().expect("an object");
        let after = up.value.as_object().expect("an object");
        let keys: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
        for key in keys {
            if declared.contains(&key.as_str()) {
                continue;
            }
            assert_eq!(
                before.get(key),
                after.get(key),
                "{name}: `{key}` changed crossing between the versions, but \
                 {kind}::divergent_fields does not say so — a merge patch writing it would be let \
                 through and mean something else at the far end",
            );
        }
    }
}

/// Asserts that a report mentions a given JSON Pointer.
#[track_caller]
fn assert_reports(lossy: &Lossy, pointer: &str) {
    assert!(
        lossy.as_slice().iter().any(|l| l.pointer == pointer),
        "expected a loss at {pointer}, got: {lossy}"
    );
}

/// Every field a downgrade drops must appear in its loss report — mechanically, over the whole
/// 2.3.0 corpus.
///
/// The round-trip tests above prove that a 2.2.1 object survives a visit to 2.3.0. They cannot
/// prove the *other* direction is honest, because going back always loses something and a test
/// that only checks the value would pass a conversion that dropped a field silently — which is
/// exactly the failure this crate claims not to have.
///
/// So this walks the JSON: every leaf pointer present in the 2.3.0 document and absent from the
/// 2.2.1 one has to be **named** by the [`Lossy`] report, either exactly or by a prefix. A new
/// 2.3.0 field added to a wire model without a matching `lossy.record` fails here on its first
/// run, in whichever example happens to use it.
mod nothing_is_dropped_in_silence {
    use super::{Lossy, Path};
    use ocpi_kit::VersionNumber;
    use ocpi_kit::convert::wire::ObjectKind;
    use serde_json::Value;

    fn fixture_2_3_0(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/2.3.0").join(name);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// Every leaf pointer in `value`, in RFC 6901 form.
    fn leaves(value: &Value, at: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) if !map.is_empty() => {
                for (key, child) in map {
                    leaves(child, &format!("{at}/{}", key.replace('~', "~0").replace('/', "~1")), out);
                }
            }
            Value::Array(items) if !items.is_empty() => {
                for (index, child) in items.iter().enumerate() {
                    leaves(child, &format!("{at}/{index}"), out);
                }
            }
            _ => out.push(at.to_owned()),
        }
    }

    /// The fields 2.3.0 **renamed** rather than added, whose value is still there under another
    /// name.
    ///
    /// Keeping this list explicit is the point: every entry is a claim that the information
    /// survived, and anything not on it has to be either present or reported. It is short, and it
    /// is the whole semantic difference between the two versions' money.
    fn is_renamed(pointer: &str) -> bool {
        // `Price`/`PriceLimit`: `{before_taxes, taxes[]}` in 2.3.0, `{excl_vat, incl_vat}` in
        // 2.2.1. The amounts cross; the tax *names* do not, and those are reported as losses.
        pointer.ends_with("/before_taxes")
            || pointer.ends_with("/after_taxes")
            || pointer.contains("/taxes/")
            // 2.2.1 has no `tax_included` because it has no other reading: a 2.2.1
            // `PriceComponent.price` is "Price per unit (excl. VAT)" by definition. Downgrading a
            // Tariff that says anything *other* than NO is recorded as a loss.
            || pointer == "/tax_included"
    }

    /// Whether `pointer` is named by a loss, exactly or as one of its descendants.
    fn is_reported(pointer: &str, lossy: &Lossy) -> bool {
        lossy.iter().any(|loss| {
            let named = loss.pointer.trim_end_matches('/');
            !named.is_empty() && (pointer == named || pointer.starts_with(&format!("{named}/")))
        })
    }

    fn check(kind: ObjectKind, name: &str) {
        let source = fixture_2_3_0(name);
        let converted = kind
            .bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_2_1, source.clone())
            .unwrap_or_else(|e| panic!("{name} does not bridge: {e}"));

        let mut before = Vec::new();
        leaves(&source, "", &mut before);
        let mut after = Vec::new();
        leaves(&converted.value, "", &mut after);

        for pointer in before {
            if after.contains(&pointer) || is_renamed(&pointer) || is_reported(&pointer, &converted.lossy) {
                continue;
            }
            // A value that merely *moved* is fine as long as the move is reported; one that is
            // gone with nothing said about it is the failure this test exists for.
            panic!(
                "{name}: {pointer} is in the 2.3.0 document, not in the 2.2.1 one, and not in the \
                 loss report ({})",
                if converted.lossy.is_empty() {
                    "which is empty".to_owned()
                } else {
                    converted.lossy.to_string()
                }
            );
        }
    }

    #[test]
    fn locations() {
        for name in [
            "location_example.json",
            "location_example_parking_garage_opening_hours.json",
            "location_example_uc2_destination_charger.json",
            "location_example_uc3_destination_charger_not_published.json",
            "location_example_uc4_limited_visibility.json",
            "location_example_uc5_home_charge_point.json",
        ] {
            check(ObjectKind::Location, name);
        }
    }

    #[test]
    fn tariffs() {
        for name in [
            "tariff_1_simple_2hour.json",
            "tariff_4_complex.json",
            "tariff_6_025kwh_start_max_price.json",
            "tariff_12_025kwh_min_price.json",
            "tariff_13_simple_3hour_5parking.json",
            "tariff_19_simple_north_american_exclusive.json",
            "tariff_20_simple_north_american_inclusive.json",
        ] {
            check(ObjectKind::Tariff, name);
        }
    }

    #[test]
    fn sessions_and_cdrs() {
        for name in ["session_example_1_simple_start.json", "session_example_2_short_finished.json"] {
            check(ObjectKind::Session, name);
        }
        // The specification's own 2.3.0 CDR example cannot be bridged, because it cannot be
        // decoded: the Tariff it embeds omits `tax_included`, which 2.3.0 makes required. That is
        // a recorded erratum, and asserting it *here* as well means an upstream fix turns this
        // test red and says to price the example instead of excusing it.
        let broken = fixture_2_3_0("cdr_example.json");
        let error = ObjectKind::Cdr
            .bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_2_1, broken)
            .expect_err("the spec's own CDR example is still missing tax_included");
        assert!(error.to_string().contains("tax_included"), "{error}");
    }

    #[test]
    fn credentials_and_tokens() {
        for name in [
            "credentials_example.json",
            "credentials_example2.json",
            "credentials_example3.json",
            "credentials_example4.json",
        ] {
            check(ObjectKind::Credentials, name);
        }
        for name in ["token_example_1_full_example.json", "token_example_2_minimal_example.json"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/2.3.0").join(name);
            if path.exists() {
                check(ObjectKind::Token, name);
            }
        }
    }
}
