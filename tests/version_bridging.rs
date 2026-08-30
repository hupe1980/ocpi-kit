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
        let (up, _down) = round_trip!(name, v2_2_1::tariffs::Tariff, v2_3_0::tariffs::Tariff);
        assert!(up.is_empty(), "{name}: upgrading a Tariff loses nothing");
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

/// Asserts that a report mentions a given JSON Pointer.
#[track_caller]
fn assert_reports(lossy: &Lossy, pointer: &str) {
    assert!(
        lossy.as_slice().iter().any(|l| l.pointer == pointer),
        "expected a loss at {pointer}, got: {lossy}"
    );
}
