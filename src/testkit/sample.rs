//! Valid sample objects, for tests that need *an* object rather than a specific one.
//!
//! Every object these produce passes [`Validate`](crate::types::Validate); that is asserted by
//! this module's own tests, so a test that starts from one of these is starting from something
//! conformant.

use crate::types::{CiString, DateTime, Extensions, InvalidString, Number, Url};
use crate::v2_3_0::cdrs::{
    AuthMethod, Cdr, CdrDimension, CdrDimensionType, CdrLocation, CdrToken, ChargingPeriod,
};
use crate::v2_3_0::locations::{
    Connector, ConnectorFormat, ConnectorType, Evse, GeoLocation, Location, PowerType, Status,
};
use crate::v2_3_0::sessions::{Session, SessionStatus};
use crate::v2_3_0::tariffs::{PriceComponent, Tariff, TariffDimensionType, TariffElement, TaxIncluded};
use crate::v2_3_0::tokens::{Token, TokenType, WhitelistType};
use crate::v2_3_0::types::Price;

/// A fixed timestamp, so sample objects compare equal across runs.
#[must_use]
pub fn timestamp() -> DateTime {
    "2024-01-15T10:00:00Z".parse().expect("a valid RFC 3339 timestamp")
}

/// The coordinates the specification's Location example uses.
#[must_use]
pub fn coordinates() -> GeoLocation {
    GeoLocation::new("51.047599", "3.729944").expect("valid WGS 84 coordinates")
}

/// A Connector with one AC socket.
///
/// # Errors
///
/// Returns [`InvalidString`] if `id` is not a usable `CiString(36)`.
pub fn connector(id: &str) -> Result<Connector, InvalidString> {
    Ok(Connector {
        id: CiString::new(id)?,
        standard: ConnectorType::Iec62196T2,
        format: ConnectorFormat::Socket,
        power_type: PowerType::Ac3Phase,
        max_voltage: 400,
        max_amperage: 32,
        max_electric_power: Some(22_000),
        tariff_ids: Vec::new(),
        terms_and_conditions: None,
        capabilities: Vec::new(),
        last_updated: timestamp(),
        extensions: Extensions::new(),
    })
}

/// An EVSE with a single connector.
///
/// # Errors
///
/// Returns [`InvalidString`] if `uid` is not a usable `CiString(36)`.
pub fn evse(uid: &str) -> Result<Evse, InvalidString> {
    Ok(Evse {
        uid: CiString::new(uid)?,
        evse_id: Some(CiString::new("BE*BEC*E041503001")?),
        status: Status::Available,
        status_schedule: Vec::new(),
        capabilities: Vec::new(),
        connectors: vec![connector("1")?],
        floor_level: None,
        coordinates: None,
        physical_reference: None,
        directions: Vec::new(),
        parking_restrictions: Vec::new(),
        parking: Vec::new(),
        images: Vec::new(),
        accepted_service_providers: Vec::new(),
        last_updated: timestamp(),
        extensions: Extensions::new(),
    })
}

/// A published Location with one EVSE.
///
/// # Errors
///
/// Returns [`InvalidString`] if `id` is not a usable `CiString(36)`.
pub fn location(id: &str) -> Result<Location, InvalidString> {
    Ok(Location::builder()
        .country_code(CiString::new("NL")?)
        .party_id(CiString::new("TNM")?)
        .id(CiString::new(id)?)
        .publish(true)
        .name("Gent Zuid")
        .address("F.Rooseveltlaan 3A")
        .city("Gent")
        .postal_code("9000")
        .country("BEL")
        .coordinates(coordinates())
        .evses(vec![evse("3256")?])
        .time_zone("Europe/Brussels")
        .last_updated(timestamp())
        .build())
}

/// A Token an app user would present.
///
/// # Errors
///
/// Returns [`InvalidString`] if `uid` is not a usable `CiString(36)`.
pub fn token(uid: &str) -> Result<Token, InvalidString> {
    Ok(Token::builder()
        .country_code(CiString::new("DE")?)
        .party_id(CiString::new("TNM")?)
        .uid(CiString::new(uid)?)
        .token_type(TokenType::AppUser)
        .contract_id(CiString::new("DE8ACC12E46L89")?)
        .issuer("TheNewMotion")
        .valid(true)
        .whitelist(WhitelistType::Never)
        .last_updated(timestamp())
        .build())
}

/// The `CdrToken` that matches [`token`].
///
/// # Errors
///
/// Returns [`InvalidString`] if `uid` is not a usable `CiString(36)`.
pub fn cdr_token(uid: &str) -> Result<CdrToken, InvalidString> {
    Ok(CdrToken::builder()
        .country_code(CiString::new("DE")?)
        .party_id(CiString::new("TNM")?)
        .uid(CiString::new(uid)?)
        .token_type(TokenType::AppUser)
        .contract_id(CiString::new("DE8ACC12E46L89")?)
        .build())
}

/// A simple Tariff: a per-kWh price with 10% VAT.
///
/// # Errors
///
/// Returns [`InvalidString`] if `id` is not a usable `CiString(36)`.
pub fn tariff(id: &str, price_per_kwh: &str) -> Result<Tariff, InvalidString> {
    let price: Number = price_per_kwh.parse().unwrap_or(Number::ZERO);
    Ok(Tariff::builder()
        .country_code(CiString::new("NL")?)
        .party_id(CiString::new("TNM")?)
        .id(CiString::new(id)?)
        .currency("EUR")
        .elements(vec![
            TariffElement::builder()
                .price_components(vec![PriceComponent {
                    component_type: TariffDimensionType::Energy,
                    price,
                    vat: Some(Number::from(10u32)),
                    step_size: 1,
                    extensions: Extensions::new(),
                }])
                .build(),
        ])
        .tax_included(TaxIncluded::No)
        .last_updated(timestamp())
        .build())
}

/// An active Session that has charged some energy.
///
/// # Errors
///
/// Returns [`InvalidString`] if `id` is not a usable `CiString(36)`.
pub fn session(id: &str) -> Result<Session, InvalidString> {
    Ok(Session::builder()
        .country_code(CiString::new("NL")?)
        .party_id(CiString::new("TNM")?)
        .id(CiString::new(id)?)
        .start_date_time(timestamp())
        .kwh(Number::from(12u32))
        .cdr_token(cdr_token("012345678")?)
        .auth_method(AuthMethod::Whitelist)
        .location_id(CiString::new("LOC1")?)
        .evse_uid(CiString::new("3256")?)
        .connector_id(CiString::new("1")?)
        .currency("EUR")
        .charging_periods(vec![charging_period()])
        .status(SessionStatus::Active)
        .last_updated(timestamp())
        .build())
}

/// A charging period that consumed 12 kWh over one hour.
#[must_use]
pub fn charging_period() -> ChargingPeriod {
    ChargingPeriod::builder()
        .start_date_time(timestamp())
        .dimensions(vec![
            CdrDimension::new(CdrDimensionType::Energy, Number::from(12u32)),
            CdrDimension::new(CdrDimensionType::Time, Number::ONE),
        ])
        .build()
}

/// A completed CDR for a one-hour, 12 kWh session at €0.25/kWh.
///
/// # Errors
///
/// Returns [`InvalidString`] if `id` is not a usable `CiString(39)`.
pub fn cdr(id: &str) -> Result<Cdr, InvalidString> {
    let energy_cost: Number = "3.00".parse().unwrap_or(Number::ZERO);
    Ok(Cdr::builder()
        .country_code(CiString::new("NL")?)
        .party_id(CiString::new("TNM")?)
        .id(CiString::new(id)?)
        .start_date_time(timestamp())
        .end_date_time("2024-01-15T11:00:00Z".parse::<DateTime>().unwrap_or_else(|_| timestamp()))
        .session_id(CiString::new("101")?)
        .cdr_token(cdr_token("012345678")?)
        .auth_method(AuthMethod::Whitelist)
        .cdr_location(cdr_location()?)
        .currency("EUR")
        .charging_periods(vec![charging_period()])
        .total_cost(Price::new(energy_cost))
        .total_energy(Number::from(12u32))
        .total_time(Number::ONE)
        .last_updated(timestamp())
        .build())
}

/// The `CdrLocation` that matches [`location`].
///
/// # Errors
///
/// Returns [`InvalidString`] if any of the fixed values is not usable, which cannot happen.
pub fn cdr_location() -> Result<CdrLocation, InvalidString> {
    Ok(CdrLocation::builder()
        .id(CiString::new("LOC1")?)
        .address("F.Rooseveltlaan 3A")
        .city("Gent")
        .postal_code("9000")
        .country("BEL")
        .coordinates(coordinates())
        .evse_uid(CiString::new("3256")?)
        .evse_id(CiString::new("BE*BEC*E041503001")?)
        .connector_id(CiString::new("1")?)
        .connector_standard(ConnectorType::Iec62196T2)
        .connector_format(ConnectorFormat::Socket)
        .connector_power_type(PowerType::Ac3Phase)
        .build())
}

/// A credentials object for a CPO platform.
///
/// # Errors
///
/// Returns [`InvalidString`] if the values are not usable, which cannot happen.
pub fn credentials(
    token: &str,
    versions_url: &str,
) -> Result<crate::v2_3_0::credentials::Credentials, InvalidString> {
    use crate::v2_3_0::credentials::{Credentials, CredentialsRole};
    use crate::v2_3_0::locations::BusinessDetails;
    use crate::v2_3_0::types::Role;
    Ok(Credentials::builder()
        .token(crate::types::OcpiString::<64>::new(token)?)
        .url(Url::new_lenient(versions_url))
        .roles(vec![
            CredentialsRole::builder()
                .role(Role::Cpo)
                .business_details(BusinessDetails::builder().name("Example Operations").build())
                .party_id(CiString::new("TNM")?)
                .country_code(CiString::new("NL")?)
                .build(),
        ])
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Validate;

    #[test]
    fn every_sample_object_is_conformant() {
        location("LOC1").unwrap().validate().unwrap();
        evse("3256").unwrap().validate().unwrap();
        connector("1").unwrap().validate().unwrap();
        token("012345678").unwrap().validate().unwrap();
        tariff("T1", "0.25").unwrap().validate().unwrap();
        session("101").unwrap().validate().unwrap();
        cdr("CDR1").unwrap().validate().unwrap();
        credentials("test-token", "https://example.com/ocpi/versions").unwrap().validate().unwrap();
    }

    #[test]
    fn every_sample_object_round_trips_through_json() {
        let original = location("LOC1").unwrap();
        let json = serde_json::to_string(&original).unwrap();
        let decoded: Location = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }
}
