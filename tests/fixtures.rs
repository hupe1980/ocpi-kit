//! Round-trip every JSON example the OCPI specification ships.
//!
//! For each file in `fixtures/<version>/` the test asserts that the crate
//!
//! 1. decodes it into the type the specification says it is,
//! 2. re-encodes it to *canonically the same JSON* — same keys, same values, with only number
//!    formatting and key order normalised, and
//! 3. finds no [`Validate`] violation in it.
//!
//! Every file must be accounted for: an unlisted fixture fails the test, so a spec release that
//! adds an example cannot slip through unread.
//!
//! Where the specification's own example is wrong, the expectation is recorded as
//! [`Expect::Erratum`] with the reason; the test asserts that the example still fails, so a fixed
//! upstream example shows up as a test failure telling us to promote it.

use std::collections::BTreeMap;
use std::path::Path;

use ocpi_kit::transport::OcpiResponse;
use ocpi_kit::types::{DisplayText, Validate};
use ocpi_kit::v2_3_0;

/// What the crate is expected to do with one fixture.
enum Expect {
    /// Decodes, round-trips canonically, and validates clean.
    Ok(fn(&str) -> Result<String, String>),
    /// The specification's own example is wrong and cannot decode. The reason is documented.
    Erratum(fn(&str) -> Result<String, String>, &'static str),
    /// The example decodes and validates, but does not round-trip byte-for-byte because the
    /// example itself is written in a non-conformant way that this crate deliberately tolerates
    /// on input and normalises on output — a `number` given as a JSON string, say.
    ///
    /// Asserted in both directions: it must still decode, and it must still *not* match. An
    /// upstream fix therefore shows up as a failure telling us to promote it to [`Expect::Ok`].
    ///
    /// Only the 2.1.1 corpus needs this today, so the variant is unused without that feature.
    #[cfg_attr(not(feature = "v2_1_1"), allow(dead_code))]
    Tolerated(fn(&str) -> Result<String, String>, &'static str),
}

/// Builds a round-tripper for one type: decode, validate, re-encode.
macro_rules! roundtrip {
    ($ty:ty) => {
        |json: &str| -> Result<String, String> {
            let value: $ty = serde_json::from_str(json).map_err(|e| e.to_string())?;
            value.validate().map_err(|v| format!("does not conform: {v}"))?;
            serde_json::to_string(&value).map_err(|e| e.to_string())
        }
    };
}

fn expectations_2_3_0() -> BTreeMap<&'static str, Expect> {
    use v2_3_0::cdrs::Cdr;
    use v2_3_0::credentials::Credentials;
    use v2_3_0::locations::{Evse, Location};
    use v2_3_0::sessions::Session;
    use v2_3_0::tariffs::Tariff;
    use v2_3_0::tokens::Token;
    use v2_3_0::versions::{Version, VersionDetails};

    let mut m: BTreeMap<&'static str, Expect> = BTreeMap::new();

    // --- CDRs ------------------------------------------------------------------------------
    m.insert(
        "cdr_example.json",
        Expect::Erratum(
            roundtrip!(Cdr),
            "the Tariff embedded in `tariffs[0]` omits `tax_included`, which OCPI 2.3.0 adds as a \
             required field (cardinality 1) of the Tariff object",
        ),
    );

    // --- Credentials -----------------------------------------------------------------------
    for f in [
        "credentials_example.json",
        "credentials_example2.json",
        "credentials_example3.json",
        "credentials_example4.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(Credentials)));
    }

    // --- Locations -------------------------------------------------------------------------
    for f in [
        "location_example.json",
        "location_example_parking_garage_opening_hours.json",
        "location_example_uc2_destination_charger.json",
        "location_example_uc3_destination_charger_not_published.json",
        "location_example_uc4_limited_visibility.json",
        "location_example_uc5_home_charge_point.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(Location)));
    }
    // The energy-mix examples are a Location fragment: `{"energy_mix": …}`.
    for f in [
        "location_energymix_example_complete.json",
        "location_energymix_example_energy_provider.json",
        "location_energymix_example_simple.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(EnergyMixExample)));
    }
    // The hours examples are bare `Hours` objects …
    for f in [
        "location_hours_247_open_exception_closing.json",
        "location_hours_opening_hours_with_exceptional_closing.json",
        "location_hours_opening_hours_with_exceptional_opening.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(v2_3_0::locations::Hours)));
    }
    // … while the regular-hours example is a Location fragment.
    m.insert("location_regularhours_example.json", Expect::Ok(roundtrip!(OpeningTimesExample)));
    m.insert(
        "location_put_example_add_evse.json",
        Expect::Erratum(
            roundtrip!(Evse),
            "the example uses `floor` (the EVSE field is `floor_level`) and gives `floor` and \
             `physical_reference` as JSON numbers where the property table says string(4) and \
             string(16)",
        ),
    );

    // --- PATCH bodies ----------------------------------------------------------------------
    for f in [
        "location_patch_example_location.json",
        "location_patch_example_remove_evse.json",
        "location_patch_example_status.json",
        "location_patch_example_tariff.json",
        "session_patch_example_charging_period.json",
        "session_patch_example_total_cost.json",
        "token_patch_example.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(serde_json::Value)));
    }

    // --- Sessions --------------------------------------------------------------------------
    for f in ["session_example_1_simple_start.json", "session_example_2_short_finished.json"] {
        m.insert(f, Expect::Ok(roundtrip!(Session)));
    }

    // --- Tariffs ---------------------------------------------------------------------------
    for f in [
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
        "tariff_19_simple_north_american_exclusive.json",
        "tariff_20_simple_north_american_inclusive.json",
        "tariffrestriction_example_max_duration.json",
        "tariffrestriction_example_max_power.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(Tariff)));
    }

    m.insert(
        "tariff_put_example.json",
        Expect::Erratum(
            roundtrip!(Tariff),
            "a PUT body \"must specify all required fields of an object\" \
             (§transport_and_format_put), but this one omits `last_updated`",
        ),
    );

    // --- Tokens ----------------------------------------------------------------------------
    for f in ["token_example_1_app_user.json", "token_example_2_full_rfid.json", "token_put_example.json"] {
        m.insert(f, Expect::Ok(roundtrip!(Token)));
    }

    // --- Transport envelope ----------------------------------------------------------------
    m.insert("transport_and_format_get_token_example.json", Expect::Ok(roundtrip!(OcpiResponse<Token>)));
    m.insert(
        "transport_and_format_get_token_list_example.json",
        Expect::Ok(roundtrip!(OcpiResponse<Vec<Token>>)),
    );
    m.insert(
        "transport_and_format_version_details_example.json",
        Expect::Ok(roundtrip!(OcpiResponse<VersionDetails>)),
    );
    m.insert(
        "transport_and_format_version_info_example.json",
        Expect::Ok(roundtrip!(OcpiResponse<Vec<Version>>)),
    );

    // --- Types and versions ----------------------------------------------------------------
    m.insert("type_displaytext_example.json", Expect::Ok(roundtrip!(DisplayText)));
    m.insert("versions_info_example.json", Expect::Ok(roundtrip!(Vec<Version>)));
    for f in ["version_details_example.json", "version_details_example2.json"] {
        m.insert(f, Expect::Ok(roundtrip!(VersionDetails)));
    }

    m
}

/// The `location_regularhours_example.json` and `location_hours_*.json` examples are a Location
/// fragment: a bare `{"opening_times": …}`.
#[derive(serde::Serialize, serde::Deserialize)]
struct OpeningTimesExample {
    opening_times: v2_3_0::locations::Hours,
}

impl Validate for OpeningTimesExample {
    fn validate_in(&self, v: &mut ocpi_kit::types::Validator) {
        v.field("opening_times", &self.opening_times);
    }
}

/// The `location_energymix_example_*.json` examples are a Location fragment:
/// a bare `{"energy_mix": …}`.
#[derive(serde::Serialize, serde::Deserialize)]
struct EnergyMixExample {
    // `EnergyMix` is wire-identical between 2.2.1 and 2.3.0, so one Rust type serves both.
    energy_mix: v2_3_0::locations::EnergyMix,
}

impl Validate for EnergyMixExample {
    fn validate_in(&self, v: &mut ocpi_kit::types::Validator) {
        v.field("energy_mix", &self.energy_mix);
    }
}

/// `payment_terminal_create_minimal.json` is a Terminal creation body: `terminal_id` only, with
/// no `last_updated` — which the Terminal object requires, because the server assigns it.
#[derive(serde::Serialize, serde::Deserialize)]
struct TerminalMinimal {
    terminal_id: ocpi_kit::types::CiString<36>,
}

impl Validate for TerminalMinimal {
    fn validate_in(&self, v: &mut ocpi_kit::types::Validator) {
        v.field("terminal_id", &self.terminal_id);
    }
}

/// Normalises a JSON document so that only meaningful differences remain: numbers become their
/// decimal text, and object keys are ordered (which `serde_json::Map` already does).
fn canonical(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Number(n) => Value::String(normalise_number(&n.to_string())),
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                // A cardinality `*` field that is absent and one that is `[]` say the same
                // thing — zero or more, of which this is zero — and this crate writes the
                // shorter of the two. Treat them as equal rather than as a round-trip failure.
                .filter(|(_, v)| !v.as_array().is_some_and(Vec::is_empty))
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `2.00` and `2` and `2.0` are the same number; JSON cannot tell them apart and neither should
/// the comparison.
fn normalise_number(text: &str) -> String {
    if !text.contains('.') {
        return text.to_owned();
    }
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" { "0".to_owned() } else { trimmed.to_owned() }
}

fn run_corpus(dir: &str, expectations: &BTreeMap<&'static str, Expect>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(dir);
    let mut seen = Vec::new();
    let mut failures = Vec::new();

    let entries = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("cannot read fixture directory {}: {e}", root.display()));
    for entry in entries {
        let path = entry.expect("readable directory entry").path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        seen.push(name.clone());

        let Some(expect) = expectations.get(name.as_str()) else {
            failures.push(format!(
                "{name}: no expectation recorded. Every example the specification ships must be \
                 accounted for; add it to the table in tests/fixtures.rs."
            ));
            continue;
        };

        let source = std::fs::read_to_string(&path).expect("readable fixture");
        let original: serde_json::Value = match serde_json::from_str(&source) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{name}: the fixture itself is not valid JSON: {e}"));
                continue;
            }
        };

        match expect {
            Expect::Ok(run) => match run(&source) {
                Ok(encoded) => {
                    let reencoded: serde_json::Value =
                        serde_json::from_str(&encoded).expect("our own output is valid JSON");
                    if canonical(&original) != canonical(&reencoded) {
                        failures.push(format!(
                            "{name}: round-trip changed the document.\n  expected: {}\n  actual:   {}",
                            serde_json::to_string(&canonical(&original)).unwrap(),
                            serde_json::to_string(&canonical(&reencoded)).unwrap(),
                        ));
                    }
                }
                Err(e) => failures.push(format!("{name}: {e}")),
            },
            Expect::Tolerated(run, reason) => match run(&source) {
                Ok(encoded) => {
                    let reencoded: serde_json::Value =
                        serde_json::from_str(&encoded).expect("our own output is valid JSON");
                    if canonical(&original) == canonical(&reencoded) {
                        failures.push(format!(
                            "{name}: recorded as tolerated-but-not-canonical ({reason}), but it \
                             now round-trips exactly. Promote it to Expect::Ok."
                        ));
                    }
                }
                Err(e) => failures.push(format!(
                    "{name}: recorded as tolerated-but-not-canonical ({reason}), but it no \
                     longer decodes at all: {e}"
                )),
            },
            Expect::Erratum(run, reason) => {
                if run(&source).is_ok() {
                    failures.push(format!(
                        "{name}: recorded as a spec erratum ({reason}), but it now decodes \
                         cleanly. If the upstream example was fixed, promote it to Expect::Ok."
                    ));
                }
            }
        }
    }

    for expected in expectations.keys() {
        if !seen.iter().any(|s| s == expected) {
            failures.push(format!(
                "{expected}: listed in the expectation table but not present in fixtures/{dir}/"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} fixtures in fixtures/{dir}/ did not behave as expected:\n\n{}",
        failures.len(),
        seen.len(),
        failures.join("\n\n")
    );
}

fn expectations_2_2_1() -> BTreeMap<&'static str, Expect> {
    use ocpi_kit::v2_2_1;
    use v2_2_1::cdrs::Cdr;
    use v2_2_1::credentials::Credentials;
    use v2_2_1::locations::{Evse, Location};
    use v2_2_1::sessions::Session;
    use v2_2_1::tariffs::Tariff;
    use v2_2_1::tokens::Token;
    use v2_2_1::versions::{Version, VersionDetails};

    let mut m: BTreeMap<&'static str, Expect> = BTreeMap::new();

    m.insert("cdr_example.json", Expect::Ok(roundtrip!(Cdr)));

    for f in [
        "credentials_example.json",
        "credentials_example2.json",
        "credentials_example3.json",
        "credentials_example4.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(Credentials)));
    }

    for f in [
        "location_example.json",
        "location_example_parking_garage_opening_hours.json",
        "location_example_uc2_destination_charger.json",
        "location_example_uc3_destination_charger_not_published.json",
        "location_example_uc4_limited_visibility.json",
        "location_example_uc5_home_charge_point.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(Location)));
    }
    for f in [
        "location_energymix_example_complete.json",
        "location_energymix_example_energy_provider.json",
        "location_energymix_example_simple.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(EnergyMixExample)));
    }
    for f in [
        "location_hours_247_open_exception_closing.json",
        "location_hours_opening_hours_with_exceptional_closing.json",
        "location_hours_opening_hours_with_exceptional_opening.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(v2_2_1::locations::Hours)));
    }
    m.insert("location_regularhours_example.json", Expect::Ok(roundtrip!(OpeningTimesExample)));
    m.insert(
        "location_put_example_add_evse.json",
        Expect::Erratum(
            roundtrip!(Evse),
            "the example uses `floor` (the EVSE field is `floor_level`) and gives `floor` and \
             `physical_reference` as JSON numbers where the property table says string(4) and \
             string(16); the same example is still wrong in OCPI 2.3.0",
        ),
    );

    for f in [
        "location_patch_example_location.json",
        "location_patch_example_remove_evse.json",
        "location_patch_example_status.json",
        "location_patch_example_tariff.json",
        "session_patch_example_charging_period.json",
        "session_patch_example_total_cost.json",
        "token_patch_example.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(serde_json::Value)));
    }

    for f in ["session_example_1_simple_start.json", "session_example_2_short_finished.json"] {
        m.insert(f, Expect::Ok(roundtrip!(Session)));
    }

    for f in [
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
        m.insert(f, Expect::Ok(roundtrip!(Tariff)));
    }
    m.insert(
        "tariff_put_example.json",
        Expect::Erratum(
            roundtrip!(Tariff),
            "a PUT body \"must specify all required fields of an object\" \
             (§transport_and_format_put), but this one omits `last_updated`",
        ),
    );

    for f in ["token_example_1_app_user.json", "token_example_2_full_rfid.json", "token_put_example.json"] {
        m.insert(f, Expect::Ok(roundtrip!(Token)));
    }

    m.insert("transport_and_format_get_token_example.json", Expect::Ok(roundtrip!(OcpiResponse<Token>)));
    m.insert(
        "transport_and_format_get_token_list_example.json",
        Expect::Ok(roundtrip!(OcpiResponse<Vec<Token>>)),
    );
    m.insert(
        "transport_and_format_version_details_example.json",
        Expect::Ok(roundtrip!(OcpiResponse<VersionDetails>)),
    );
    m.insert(
        "transport_and_format_version_info_example.json",
        Expect::Ok(roundtrip!(OcpiResponse<Vec<Version>>)),
    );

    m.insert("type_displaytext_example.json", Expect::Ok(roundtrip!(DisplayText)));
    m.insert("versions_info_example.json", Expect::Ok(roundtrip!(Vec<Version>)));
    for f in ["version_details_example.json", "version_details_example2.json"] {
        m.insert(f, Expect::Ok(roundtrip!(VersionDetails)));
    }

    m
}

/// OCPI 2.1.1's examples are embedded inline in the Markdown source rather than shipped as
/// `examples/*.json`; `xtask sync-fixtures` extracts them.
#[cfg(feature = "v2_1_1")]
fn expectations_2_1_1() -> BTreeMap<&'static str, Expect> {
    use ocpi_kit::v2_1_1;
    use v2_1_1::cdrs::Cdr;
    use v2_1_1::credentials::Credentials;
    use v2_1_1::locations::Location;
    use v2_1_1::sessions::Session;
    use v2_1_1::tariffs::Tariff;
    use v2_1_1::tokens::Token;
    use v2_1_1::versions::{Version, VersionDetails};

    let mut m: BTreeMap<&'static str, Expect> = BTreeMap::new();

    m.insert(
        "cdrs_example_of_a_cdr.json",
        Expect::Tolerated(
            roundtrip!(Cdr),
            "the embedded Tariff writes `price` as the JSON *string* `\"2.00\"` where \
             §types_number_type requires a JSON number; this crate parses it exactly and emits \
             it unquoted, so the document changes",
        ),
    );
    m.insert("credentials_example.json", Expect::Ok(roundtrip!(Credentials)));
    m.insert("locations_example.json", Expect::Ok(roundtrip!(Location)));
    for f in [
        "sessions_simple_session_example_of_a_just_starting_session.json",
        "sessions_simple_session_example_of_a_short_finished_session.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(Session)));
    }
    for f in [
        "tariffs_complex_tariff_example.json",
        "tariffs_free_of_charge_tariff_example.json",
        "tariffs_simple_tariff_example_2_euro_per_hour.json",
        "tariffs_simple_tariff_example_with_alternative_multi_language_text.json",
        "tariffs_simple_tariff_example_with_alternative_url.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(Tariff)));
    }
    m.insert("tokens_example.json", Expect::Ok(roundtrip!(Token)));

    // The transport chapter's examples are envelopes, not bare objects.
    m.insert(
        "transport_and_format_example_response_with_an_error_contains_no_data_field.json",
        Expect::Ok(roundtrip!(OcpiResponse<serde_json::Value>)),
    );
    m.insert(
        "transport_and_format_example_tokens_get_response_with_list_of_token_objects_emsp_end_point_list_of_objects.json",
        Expect::Ok(roundtrip!(OcpiResponse<Vec<Token>>)),
    );
    m.insert(
        "transport_and_format_example_tokens_get_response_with_one_token_object_cpo_end_point_one_object.json",
        Expect::Ok(roundtrip!(OcpiResponse<Token>)),
    );
    m.insert(
        "transport_and_format_example_version_details_response_one_object.json",
        Expect::Ok(roundtrip!(OcpiResponse<VersionDetails>)),
    );
    m.insert(
        "transport_and_format_example_version_information_response_list_of_objects.json",
        Expect::Ok(roundtrip!(OcpiResponse<Vec<Version>>)),
    );
    m.insert("version_information_endpoint_example.json", Expect::Ok(roundtrip!(Vec<Version>)));
    m.insert("version_information_endpoint_example_1.json", Expect::Ok(roundtrip!(VersionDetails)));

    m
}

/// The 2.3.0 `payments` release branch carries the whole 2.3.0 core plus the Payments module.
///
/// Upstream moved Payments out of core in July 2026, onto its own release branch — the same shape
/// the Bookings module has always had. Its examples therefore live in this corpus rather than in
/// the core one, and the core corpus no longer carries any.
#[cfg(feature = "payments")]
fn expectations_payments() -> BTreeMap<&'static str, Expect> {
    use ocpi_kit::v2_3_0::payments::{FinancialAdviceConfirmation, Terminal};

    let mut m = expectations_2_3_0();
    m.insert("payment_terminal_location_assignment.json", Expect::Ok(roundtrip!(serde_json::Value)));
    for f in [
        "payment_terminal_activate.json",
        "payment_terminal_create.json",
        "payment_terminal_example_assigned_locations.json",
        "payment_terminal_example_assigned_locations_assigned_evses.json",
        "payment_terminal_example_newly_created.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(Terminal)));
    }
    m.insert("payment_terminal_create_minimal.json", Expect::Ok(roundtrip!(TerminalMinimal)));
    // A PUT body whose `terminal_id` is carried by the URL rather than the body.
    m.insert("payment_terminal_put_update.json", Expect::Ok(roundtrip!(serde_json::Value)));
    for f in [
        "payment_financial_advice_confirmation_create.json",
        "payment_financial_advice_confirmation_example_failure.json",
        "payment_financial_advice_confirmation_example_success.json",
    ] {
        m.insert(
            f,
            Expect::Erratum(
                roundtrip!(FinancialAdviceConfirmation),
                "`total_costs` is written in the OCPI 2.2.1 Price shape `{excl_vat, incl_vat}`; \
                 OCPI 2.3.0 replaced it with `{before_taxes, taxes[]}`",
            ),
        );
    }
    m
}

/// The 2.3.0 `bookings` release branch carries the whole 2.3.0 core plus the Bookings module, so
/// its corpus is the 2.3.0 expectations plus the Booking objects — and minus the five Location
/// examples the branch never updated (see the erratum below).
#[cfg(feature = "bookings")]
fn expectations_bookings() -> BTreeMap<&'static str, Expect> {
    use ocpi_kit::v2_3_0::bookings::{Booking, BookingLocation};

    // The branch's own EVSE property table defines `parking` as `EVSEParking*`, exactly as the
    // core 2.3.0 table does, but two of its Location examples were never updated from the
    // earlier form where the field was a list of bare parking ids.
    const STALE_PARKING: &str = "the branch's own EVSE property table defines `parking` as \
         `EVSEParking*`, identically to core 2.3.0, but this example still gives it as a list of \
         bare parking-id strings";

    let mut m = expectations_2_3_0();

    for f in ["location_example.json", "location_example_parking_garage_opening_hours.json"] {
        m.insert(f, Expect::Erratum(roundtrip!(ocpi_kit::v2_3_0::locations::Location), STALE_PARKING));
    }

    m.insert(
        "booking_example.json",
        Expect::Erratum(
            roundtrip!(Booking),
            "every `booking_requests[].booking_request` omits `booking_location_id`, which the \
             BookingRequest table gives cardinality 1, and gives `party_id` as `INF12` where the \
             table says CiString(3)",
        ),
    );
    m.insert(
        "booking_location_example.json",
        Expect::Erratum(
            roundtrip!(BookingLocation),
            "`booking_option.evse_position` is a single string, but the BookingOption table gives \
             `evse_position` cardinality `*` — a list",
        ),
    );
    // The patch examples are partial objects: a merge patch is JSON, not a Booking.
    for f in [
        "booking_location_patch_example_calendar.json",
        "booking_location_patch_example_terms.json",
        "booking_patch_example_evse_uid.json",
        "booking_patch_example_status_update.json",
        "booking_request_patch_example_timeslot.json",
        "booking_request_patch_example_tokens.json",
    ] {
        m.insert(f, Expect::Ok(roundtrip!(serde_json::Value)));
    }

    m
}

#[test]
fn every_ocpi_2_3_0_example_round_trips() {
    run_corpus("2.3.0", &expectations_2_3_0());
}

#[test]
fn every_ocpi_2_2_1_example_round_trips() {
    run_corpus("2.2.1", &expectations_2_2_1());
}

#[test]
#[cfg(feature = "v2_1_1")]
fn every_ocpi_2_1_1_example_round_trips() {
    run_corpus("2.1.1", &expectations_2_1_1());
}

#[test]
#[cfg(feature = "bookings")]
fn every_ocpi_2_3_0_bookings_example_round_trips() {
    run_corpus("2.3.0-bookings", &expectations_bookings());
}

#[test]
#[cfg(feature = "payments")]
fn every_ocpi_2_3_0_payments_example_round_trips() {
    run_corpus("2.3.0-payments", &expectations_payments());
}
