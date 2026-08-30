//! Snapshot tests for the artefacts a human reads.
//!
//! Round-trip and equality tests prove that a value survives; they say nothing about whether what
//! comes out is *legible*. A `Link` header, an error envelope and a priced session are all read
//! by people — a partner's integration engineer, an on-call responder, a disputing driver — and
//! their shape is part of the contract even where the specification does not pin it down.
//!
//! So these are snapshots. When one changes, the diff is the review: either the change is an
//! improvement and the snapshot is accepted, or it is a regression nobody would have noticed in
//! an assertion on one field.
//!
//! Run `cargo insta review` after a deliberate change.

#![cfg(all(feature = "tariffs", feature = "transport", feature = "v2_3_0"))]

use ocpi_kit::tariffs::{PricedPeriod, PricedSession, PricingEngine, PricingPolicy, TimeZone};
use ocpi_kit::transport::{OcpiError, OcpiResponse, PageMeta, StatusCode};
use ocpi_kit::types::{DateTime, Number, Url};
use ocpi_kit::v2_3_0::tariffs::{
    PriceComponent, Tariff, TariffDimensionType, TariffElement, TariffRestrictions, TaxIncluded,
};

fn n(s: &str) -> Number {
    s.parse().expect("a valid number")
}

fn dt(s: &str) -> DateTime {
    s.parse().expect("a valid timestamp")
}

// ---------------------------------------------------------------------------------------------
// Pagination headers
// ---------------------------------------------------------------------------------------------

#[test]
fn pagination_headers_of_a_middle_page() {
    let meta = PageMeta {
        next: Some(Url::new("https://cpo.example.com/ocpi/cpo/2.3.0/cdrs?offset=150&limit=50").unwrap()),
        total_count: Some(1234),
        limit: Some(50),
    };
    let mut headers = http::HeaderMap::new();
    meta.write_to(&mut headers);

    let mut rendered: Vec<String> =
        headers.iter().map(|(name, value)| format!("{name}: {}", value.to_str().expect("ASCII"))).collect();
    rendered.sort();
    insta::assert_snapshot!("pagination_headers_middle_page", rendered.join("\n"));
}

#[test]
fn pagination_headers_of_a_last_page_have_no_link() {
    let meta = PageMeta { next: None, total_count: Some(3), limit: Some(50) };
    let mut headers = http::HeaderMap::new();
    meta.write_to(&mut headers);

    let mut rendered: Vec<String> =
        headers.iter().map(|(name, value)| format!("{name}: {}", value.to_str().expect("ASCII"))).collect();
    rendered.sort();
    insta::assert_snapshot!("pagination_headers_last_page", rendered.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// Error envelopes
// ---------------------------------------------------------------------------------------------

/// Every error a peer can be shown, as the JSON it is shown as.
///
/// This is the crate's whole public error vocabulary in one place: if a variant's message becomes
/// unhelpful, or a status code moves, it shows up here rather than in a partner's inbox.
#[test]
fn the_error_envelopes_a_peer_can_be_shown() {
    let errors: Vec<OcpiError> = vec![
        OcpiError::MalformedJson("expected value at line 1 column 1".to_owned()),
        OcpiError::Decode {
            path: "/evses/0/connectors/0/standard".to_owned(),
            message: "unknown variant `IEC_62196_T2_X`".to_owned(),
        },
        OcpiError::Unauthorized("no Authorization header".to_owned()),
        OcpiError::TokenAOutOfScope,
        OcpiError::NotFound("/ocpi/cpo/2.3.0/locations/LOC99".to_owned()),
        OcpiError::MethodNotAllowed("this client is already registered".to_owned()),
        OcpiError::NotRoutable("a GET addressed to the hub is not a Broadcast Push".to_owned()),
        OcpiError::Transport("connection closed before message completed".to_owned()),
        OcpiError::UrlRefused {
            url: "http://169.254.169.254/latest/meta-data".to_owned(),
            reason: "169.254.169.254 is on a private or loopback network".to_owned(),
        },
        OcpiError::Remote {
            status_code: StatusCode::UNKNOWN_TOKEN,
            status_message: Some("no such Token".to_owned()),
        },
        OcpiError::MissingData { status_code: StatusCode::SUCCESS },
    ];

    let rendered: Vec<String> = errors
        .iter()
        .map(|error| {
            // The timestamp is the one field that cannot be a snapshot; everything else is.
            let envelope: OcpiResponse<()> = error.to_response();
            format!(
                "HTTP {} | {} | {}",
                error.http_status(),
                envelope.status_code,
                envelope.status_message.unwrap_or_default(),
            )
        })
        .collect();
    insta::assert_snapshot!("error_envelopes", rendered.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// Priced sessions
// ---------------------------------------------------------------------------------------------

fn component(kind: TariffDimensionType, price: &str, vat: Option<&str>, step: u32) -> PriceComponent {
    PriceComponent {
        component_type: kind,
        price: n(price),
        vat: vat.map(n),
        step_size: step,
        extensions: ocpi_kit::types::Extensions::default(),
    }
}

fn element(components: Vec<PriceComponent>, restrictions: Option<TariffRestrictions>) -> TariffElement {
    TariffElement::builder().price_components(components).maybe_restrictions(restrictions).build()
}

fn tariff(elements: Vec<TariffElement>) -> Tariff {
    Tariff::builder()
        .country_code("DE")
        .party_id("ALL")
        .id("14")
        .currency("EUR")
        .elements(elements)
        .tax_included(TaxIncluded::No)
        .last_updated(dt("2015-06-29T20:39:09Z"))
        .build()
}

/// The specification's own `step_size` worked example, rendered in full.
///
/// > *Charging fee of € 1.20 per hour before 17:00 with a `step_size` of 30 minutes; € 2.40 per
/// > hour after 17:00 with a `step_size` of 15 minutes; parking fee of € 1.00 per hour before
/// > 20:00 with a `step_size` of 15 minutes.*
///
/// The spec's answer is € 0.73: 12 minutes of charging at € 2.40/h, and 8 minutes of parking
/// billed as 15. What the snapshot adds over asserting that one number is *why* — which element
/// priced each segment, what the measured and billed quantities were, and where the rounding
/// landed. That is the artefact a driver disputing an invoice is shown.
#[test]
fn the_specs_step_size_example_broken_down() {
    let after_17 = TariffRestrictions::builder()
        .start_time("17:00".parse::<ocpi_kit::types::LocalTime>().unwrap())
        .build();
    let before_17 = TariffRestrictions::builder()
        .end_time("17:00".parse::<ocpi_kit::types::LocalTime>().unwrap())
        .build();
    let before_20 = TariffRestrictions::builder()
        .end_time("20:00".parse::<ocpi_kit::types::LocalTime>().unwrap())
        .build();

    let t = tariff(vec![
        element(vec![component(TariffDimensionType::Time, "2.40", Some("20"), 900)], Some(after_17)),
        element(vec![component(TariffDimensionType::Time, "1.20", Some("20"), 1800)], Some(before_17)),
        element(vec![component(TariffDimensionType::ParkingTime, "1.00", Some("20"), 900)], Some(before_20)),
    ]);

    // 12 minutes charging from 19:40, then 8 minutes parking from 19:52.
    let session = PricedSession::new(dt("2024-01-15T19:40:00Z"), TimeZone::utc())
        .with_period(PricedPeriod {
            charging_hours: n("12") / n("60"),
            ..PricedPeriod::new(dt("2024-01-15T19:40:00Z"))
        })
        .with_period(PricedPeriod {
            parking_hours: n("8") / n("60"),
            ..PricedPeriod::new(dt("2024-01-15T19:52:00Z"))
        })
        .ending(dt("2024-01-15T20:00:00Z"));

    let breakdown = PricingEngine::new().price(&session, &[t]).expect("the session prices");
    assert_eq!(breakdown.total_excl_vat.to_string(), "0.73", "the specification's own answer");
    insta::assert_json_snapshot!("step_size_example_breakdown", breakdown);
}

/// The same session under the OCPI 3.0 policy, which has no `step_size`.
///
/// Side by side with the snapshot above, this is the clearest statement of what `step_size` costs
/// a driver: the same 8 minutes of parking, billed exactly.
#[test]
fn the_same_session_without_step_size_as_ocpi_3_0_will_bill_it() {
    let before_20 = TariffRestrictions::builder()
        .end_time("20:00".parse::<ocpi_kit::types::LocalTime>().unwrap())
        .build();
    let t = tariff(vec![
        element(vec![component(TariffDimensionType::Time, "2.40", Some("20"), 900)], None),
        element(vec![component(TariffDimensionType::ParkingTime, "1.00", Some("20"), 900)], Some(before_20)),
    ]);

    let session = PricedSession::new(dt("2024-01-15T19:40:00Z"), TimeZone::utc())
        .with_period(PricedPeriod {
            charging_hours: n("12") / n("60"),
            ..PricedPeriod::new(dt("2024-01-15T19:40:00Z"))
        })
        .with_period(PricedPeriod {
            parking_hours: n("8") / n("60"),
            ..PricedPeriod::new(dt("2024-01-15T19:52:00Z"))
        })
        .ending(dt("2024-01-15T20:00:00Z"));

    let engine = PricingEngine::with_policy(PricingPolicy::default().without_step_size());
    let breakdown = engine.price(&session, &[t]).expect("the session prices");
    insta::assert_json_snapshot!("no_step_size_breakdown", breakdown);
}
