//! The *CDRs* module of OCPI 2.1.1.
//!
//! The 2.1.1 CDR is markedly smaller than its successors: the cost is a bare `number` excluding
//! VAT rather than a `Price`, the location is a **whole `Location` object** rather than a
//! purpose-built `CdrLocation`, the driver is identified by `auth_id` rather than by a
//! `CdrToken`, and there is no `session_id`, no signed metering data and no credit CDR.
//!
//! Spec: 2.1.1 §mod_cdrs

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_lenient_enum;
use crate::types::validate_fields;
use crate::types::{
    CiString, Currency, DateTime, Extensions, Number, OcpiString, Validate, Validator, ViolationCode,
};

use super::locations::Location;
use super::tariffs::Tariff;

/// A Charge Detail Record, in OCPI 2.1.1.
///
/// Spec: 2.1.1 §mod_cdrs_cdr_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Cdr {
    /// Uniquely identifies the CDR within the CPO's platform.
    pub id: CiString<36>,
    /// Start timestamp of the charging session.
    pub start_date_time: DateTime,
    /// Stop timestamp of the charging session.
    ///
    /// Renamed to `end_date_time` in OCPI 2.2.
    pub stop_date_time: DateTime,
    /// Reference to the `auth_id` of the Token that started the session.
    ///
    /// Replaced by the richer `cdr_token` object in OCPI 2.2.
    pub auth_id: OcpiString<36>,
    /// Method used for authentication.
    pub auth_method: AuthMethod,
    /// Where the charging session took place.
    ///
    /// A whole `Location` object, *"including only the relevant EVSE and Connector"*. OCPI 2.2
    /// replaced this with the purpose-built `CdrLocation`, which is both smaller and unambiguous
    /// about which EVSE the session used.
    pub location: Location,
    /// Identification of the meter inside the Charge Point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter_id: Option<OcpiString<255>>,
    /// Currency of the CDR in ISO 4217 code.
    pub currency: Currency,
    /// Relevant Tariffs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tariffs: Vec<Tariff>,
    /// Charging Periods that make up this session. Cardinality `+`.
    pub charging_periods: Vec<ChargingPeriod>,
    /// Total cost of this transaction, **excluding VAT**.
    pub total_cost: Number,
    /// Total energy charged, in kWh.
    pub total_energy: Number,
    /// Total duration of the session, in hours.
    pub total_time: Number,
    /// Total duration during which the EV was not charging, in hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_parking_time: Option<Number>,
    /// Human-readable remark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<OcpiString<255>>,
    /// Timestamp when this CDR was last updated (or created).
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Cdr {
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
            id,
            start_date_time,
            stop_date_time,
            auth_id,
            auth_method,
            location,
            meter_id,
            currency,
            tariffs,
            charging_periods,
            total_cost,
            total_energy,
            total_time,
            total_parking_time,
            remark,
            last_updated,
        );
        if self.charging_periods.is_empty() {
            v.report_at(
                "charging_periods",
                ViolationCode::EmptyRequiredList,
                "a CDR has cardinality `+` charging_periods: at least one is required",
            );
        }
        if self.stop_date_time < self.start_date_time {
            v.report_at(
                "stop_date_time",
                ViolationCode::Inconsistent,
                "a session cannot stop before it starts",
            );
        }
        if self.total_parking_time.is_some_and(|p| p > self.total_time) {
            v.report_at(
                "total_parking_time",
                ViolationCode::Inconsistent,
                "cannot exceed total_time, of which it is a part",
            );
        }
    }
}

/// A period of a session during which the values that influence its cost were stable.
///
/// Identical in shape to the later versions except that its dimensions are the six of
/// [`CdrDimensionType`], and it has no `tariff_id` — that arrived in OCPI 2.2.
///
/// Spec: 2.1.1 §mod_cdrs_chargingperiod_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct ChargingPeriod {
    /// Start of the charging period.
    pub start_date_time: DateTime,
    /// Relevant values for this charging period. Cardinality `+`.
    pub dimensions: Vec<CdrDimension>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl ChargingPeriod {
    /// The volume recorded for one dimension in this period.
    #[must_use]
    pub fn volume(&self, dimension: CdrDimensionType) -> Option<Number> {
        self.dimensions.iter().find(|d| d.dimension_type == dimension).map(|d| d.volume)
    }
}

impl Validate for ChargingPeriod {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, start_date_time, dimensions);
        if self.dimensions.is_empty() {
            v.report_at(
                "dimensions",
                ViolationCode::EmptyRequiredList,
                "a ChargingPeriod has cardinality `+` dimensions: at least one is required",
            );
        }
    }
}

/// One measured quantity within a [`ChargingPeriod`].
///
/// Spec: 2.1.1 §mod_cdrs_cdrdimension_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CdrDimension {
    /// Type of CDR dimension.
    #[serde(rename = "type")]
    pub dimension_type: CdrDimensionType,
    /// Volume of the dimension consumed.
    pub volume: Number,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl CdrDimension {
    /// Creates a dimension measurement.
    #[must_use]
    pub fn new(dimension_type: CdrDimensionType, volume: Number) -> Self {
        Self { dimension_type, volume, extensions: Extensions::new() }
    }
}

impl Validate for CdrDimension {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, dimension_type as "type", volume);
    }
}

ocpi_lenient_enum! {
    /// How the driver was authenticated, in OCPI 2.1.1.
    ///
    /// `COMMAND` — the session was started by a remote command — arrived in OCPI 2.2.
    ///
    /// Spec: 2.1.1 §mod_cdrs_authmethod_enum
    pub enum AuthMethod {
        /// An authentication request was sent to the eMSP.
        AuthRequest = "AUTH_REQUEST",
        /// A whitelist was used; no request to the eMSP was performed.
        Whitelist = "WHITELIST",
    }
}

ocpi_lenient_enum! {
    /// The quantities a [`ChargingPeriod`] can record, in OCPI 2.1.1.
    ///
    /// Six values. OCPI 2.2 grew this to thirteen and split the session-only ones out.
    ///
    /// Spec: 2.1.1 §mod_cdrs_cdrdimensiontype_enum
    pub enum CdrDimensionType {
        /// Total amount of energy charged during this period, in kWh.
        Energy = "ENERGY",
        /// A flat fee, without a unit.
        Flat = "FLAT",
        /// Sum of the maximum current over all phases, in A.
        MaxCurrent = "MAX_CURRENT",
        /// Sum of the minimum current over all phases, in A.
        MinCurrent = "MIN_CURRENT",
        /// Time not charging, in hours.
        ParkingTime = "PARKING_TIME",
        /// Time charging, in hours.
        Time = "TIME",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_2_1_1_dimension_set_is_the_small_one() {
        assert_eq!(CdrDimensionType::ALL_KNOWN.len(), 6);
        // ENERGY_EXPORT and STATE_OF_CHARGE arrived in OCPI 2.2.
        assert!(!CdrDimensionType::from("STATE_OF_CHARGE").is_known());
        assert!(!AuthMethod::from("COMMAND").is_known(), "COMMAND arrived with the Commands module");
    }

    #[test]
    fn the_cost_is_a_bare_number_excluding_vat() {
        let json = r#"{"volume":12.5,"type":"ENERGY"}"#;
        let dimension: CdrDimension = serde_json::from_str(json).unwrap();
        assert_eq!(dimension.dimension_type, CdrDimensionType::Energy);
        assert_eq!(serde_json::to_string(&dimension).unwrap(), r#"{"type":"ENERGY","volume":12.5}"#);
    }
}
