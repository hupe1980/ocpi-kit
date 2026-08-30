//! The *CDRs* module of OCPI 2.2.1, as a delta from [`v2_3_0::cdrs`](crate::v2_3_0::cdrs).
//!
//! The field list is unchanged between the two versions; what changed is the types the fields
//! carry — [`Price`], [`TokenType`] and
//! [`ConnectorType`] — so [`Cdr`], [`CdrToken`] and
//! [`CdrLocation`] are redefined and everything else is re-exported.
//!
//! Spec: 2.2.1 §mod_cdrs_cdrs_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    CiString, ContractId, CountryCode, Currency, DateTime, EvseId, Extensions, Number, OcpiString, PartyId,
    PartyRef, Validate, Validator, ViolationCode,
};

use super::locations::{ConnectorFormat, ConnectorType, GeoLocation, PowerType};
use super::tariffs::Tariff;
use super::tokens::TokenType;
use super::types::Price;

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::cdrs::{
    AuthMethod, CdrDimension, CdrDimensionType, ChargingPeriod, NON_CREDIT_ID_MAX_LEN, SignedData,
    SignedValue,
};

/// A Charge Detail Record: one charging session and its costs, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_cdrs_cdr_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Cdr {
    /// ISO-3166 alpha-2 country code of the CPO that 'owns' this CDR.
    pub country_code: CountryCode,
    /// ID of the CPO that 'owns' this CDR.
    pub party_id: PartyId,
    /// Uniquely identifies the CDR, unique per `country_code`/`party_id` combination.
    pub id: CiString<39>,
    /// Start of the charging session, or of the reservation when there was no session.
    pub start_date_time: DateTime,
    /// When the session was completed.
    pub end_date_time: DateTime,
    /// The Session this CDR belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<CiString<36>>,
    /// Token used to start this charging session.
    pub cdr_token: CdrToken,
    /// Method used for authentication. The last method used during the session.
    pub auth_method: AuthMethod,
    /// Reference to the authorization given by the eMSP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_reference: Option<CiString<36>>,
    /// Where the charging session took place.
    pub cdr_location: CdrLocation,
    /// Identification of the meter inside the Charge Point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter_id: Option<OcpiString<255>>,
    /// Currency of the CDR in ISO 4217 code.
    pub currency: Currency,
    /// Relevant Tariffs, as they were at the start of the session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tariffs: Vec<Tariff>,
    /// Charging Periods that make up this session. Cardinality `+`.
    pub charging_periods: Vec<ChargingPeriod>,
    /// Signed metering data belonging to this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_data: Option<SignedData>,
    /// Total sum of all the costs of this transaction.
    pub total_cost: Price,
    /// Total of the fixed costs, except fixed price components of parking and reservation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_fixed_cost: Option<Price>,
    /// Total energy charged, in kWh.
    pub total_energy: Number,
    /// Total cost of all the energy used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_energy_cost: Option<Price>,
    /// Total duration of the charging session, in hours.
    pub total_time: Number,
    /// Total cost related to the duration of charging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_time_cost: Option<Price>,
    /// Total duration during which the EV was not charging, in hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_parking_time: Option<Number>,
    /// Total cost related to parking, including fixed price components.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_parking_cost: Option<Price>,
    /// Total cost related to a reservation, including fixed price components.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_reservation_cost: Option<Price>,
    /// Human-readable remark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<OcpiString<255>>,
    /// Reference to an invoice that will later be sent for this CDR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_reference_id: Option<CiString<39>>,
    /// Whether this is a Credit CDR. Requires `credit_reference_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<bool>,
    /// The `id` of the CDR this Credit CDR corrects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_reference_id: Option<CiString<39>>,
    /// Whether the energy cost of this home-charging session is compensated to the EV driver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_charging_compensation: Option<bool>,
    /// Timestamp when this CDR was last updated (or created).
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Cdr {
    /// The CPO that owns this CDR.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }

    /// Whether this is a Credit CDR.
    #[must_use]
    pub fn is_credit(&self) -> bool {
        self.credit.unwrap_or(false)
    }

    /// The time the EV was actually charging, in hours.
    #[must_use]
    pub fn total_charging_time(&self) -> Number {
        self.total_time - self.total_parking_time.unwrap_or(Number::ZERO)
    }

    /// The total volume of one dimension across every charging period.
    #[must_use]
    pub fn dimension_total(&self, dimension: CdrDimensionType) -> Number {
        self.charging_periods
            .iter()
            .flat_map(|p| p.dimensions.iter())
            .filter(|d| d.dimension_type == dimension)
            .map(|d| d.volume)
            .sum()
    }
}

impl Validate for Cdr {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            country_code,
            party_id,
            id,
            start_date_time,
            end_date_time,
            session_id,
            cdr_token,
            auth_method,
            authorization_reference,
            cdr_location,
            meter_id,
            currency,
            tariffs,
            charging_periods,
            signed_data,
            total_cost,
            total_fixed_cost,
            total_energy,
            total_energy_cost,
            total_time,
            total_time_cost,
            total_parking_time,
            total_parking_cost,
            total_reservation_cost,
            remark,
            invoice_reference_id,
            credit_reference_id,
            last_updated,
        );
        if self.charging_periods.is_empty() {
            v.report_at(
                "charging_periods",
                ViolationCode::EmptyRequiredList,
                "a CDR has cardinality `+` charging_periods: at least one is required",
            );
        }
        if !self.is_credit() && self.id.len() > NON_CREDIT_ID_MAX_LEN {
            v.report_at(
                "id",
                ViolationCode::TooLong,
                format!("a non-credit CDR id may be at most {NON_CREDIT_ID_MAX_LEN} characters"),
            );
        }
        if self.is_credit() && self.credit_reference_id.is_none() {
            v.report_at(
                "credit_reference_id",
                ViolationCode::MissingConditional,
                "is required to be set for a Credit CDR",
            );
        }
        if self.end_date_time < self.start_date_time && self.start_date_time.unix_timestamp() != 0 {
            v.report_at(
                "end_date_time",
                ViolationCode::Inconsistent,
                "a session cannot end before it starts",
            );
        }
        for (i, period) in self.charging_periods.iter().enumerate() {
            for (j, dim) in period.dimensions.iter().enumerate() {
                if dim.dimension_type.is_session_only() {
                    v.enter("charging_periods");
                    v.enter(&i.to_string());
                    v.enter("dimensions");
                    v.enter(&j.to_string());
                    v.report_at(
                        "type",
                        ViolationCode::Inconsistent,
                        format!("{} SHALL only be used in Sessions", dim.dimension_type),
                    );
                    v.leave();
                    v.leave();
                    v.leave();
                    v.leave();
                }
            }
        }
    }
}

/// The token that started a session, as recorded in a CDR or Session, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_cdrs_cdr_token_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct CdrToken {
    /// ISO-3166 alpha-2 country code of the MSP that 'owns' this Token.
    pub country_code: CountryCode,
    /// ID of the eMSP that 'owns' this Token.
    pub party_id: PartyId,
    /// Unique ID by which this Token can be identified by the CPO's system.
    pub uid: CiString<36>,
    /// Type of the token.
    #[serde(rename = "type")]
    pub token_type: TokenType,
    /// Uniquely identifies the EV driver contract token within the eMSP's platform.
    pub contract_id: ContractId,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl CdrToken {
    /// The eMSP that owns this Token.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }
}

impl Validate for CdrToken {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, country_code, party_id, uid, token_type as "type", contract_id);
    }
}

/// The parts of a Location that a CDR needs, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_cdrs_cdr_location_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct CdrLocation {
    /// Uniquely identifies the location within the CPO's platform.
    pub id: CiString<36>,
    /// Display name of the location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<OcpiString<255>>,
    /// Street/block name and house number if available.
    pub address: OcpiString<45>,
    /// City or town.
    pub city: OcpiString<45>,
    /// Postal code of the location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<OcpiString<10>>,
    /// State, only to be used when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<OcpiString<20>>,
    /// ISO 3166-1 alpha-3 code for the country of this location.
    pub country: OcpiString<3>,
    /// Coordinates of the location.
    pub coordinates: GeoLocation,
    /// The EVSE's technical identifier. May be `#NA`.
    pub evse_uid: CiString<36>,
    /// The EVSE's human-readable ID. May be `#NA`.
    pub evse_id: EvseId,
    /// Identifier of the connector within the EVSE. May be `#NA`.
    pub connector_id: CiString<36>,
    /// The standard of the installed connector.
    pub connector_standard: ConnectorType,
    /// The format (socket/cable) of the installed connector.
    pub connector_format: ConnectorFormat,
    /// Whether the connector supplies AC or DC, and on how many phases.
    pub connector_power_type: PowerType,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl CdrLocation {
    /// Whether this CDR records a reservation that never became a charging session.
    #[must_use]
    pub fn is_reservation_only(&self) -> bool {
        self.evse_uid.is_not_available()
            || self.evse_id.is_not_available()
            || self.connector_id.is_not_available()
    }
}

impl Validate for CdrLocation {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            id,
            name,
            address,
            city,
            postal_code,
            state,
            country,
            coordinates,
            evse_uid,
            evse_id,
            connector_id,
            connector_standard,
            connector_format,
            connector_power_type,
        );
    }
}
