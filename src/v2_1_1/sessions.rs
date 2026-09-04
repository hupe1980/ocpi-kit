//! The *Sessions* module of OCPI 2.1.1.
//!
//! Two things to watch for: the timestamp fields are spelled `start_datetime` and `end_datetime`
//! — **without** the second underscore that every other OCPI version uses — and the session
//! carries a whole [`Location`] rather than the three ids that replaced it in OCPI 2.2.
//!
//! Spec: 2.1.1 §mod_sessions

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_lenient_enum;
use crate::types::validate_fields;
use crate::types::{Currency, DateTime, Extensions, Number, OcpiString, Validate, Validator, ViolationCode};

use super::cdrs::{AuthMethod, CdrDimensionType, ChargingPeriod};
use super::locations::Location;

/// One charging session, in OCPI 2.1.1.
///
/// Spec: 2.1.1 §mod_sessions_session_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Session {
    /// The unique id that identifies the session in the CPO platform.
    pub id: OcpiString<36>,
    /// When the session became active.
    ///
    /// **Note the spelling.** OCPI 2.1.1 writes `start_datetime`; every later version writes
    /// `start_date_time`. Getting this wrong is a silent data loss, which is why the Rust field
    /// is named after the later spelling and carries an explicit `#[serde(rename)]`.
    #[serde(rename = "start_datetime")]
    pub start_date_time: DateTime,
    /// When the session was completed.
    #[serde(rename = "end_datetime", default, skip_serializing_if = "Option::is_none")]
    pub end_date_time: Option<DateTime>,
    /// How many kWh were charged.
    pub kwh: Number,
    /// Reference to the `auth_id` of the Token that started the session.
    pub auth_id: OcpiString<36>,
    /// Method used for authentication.
    pub auth_method: AuthMethod,
    /// Where this session took place, *"including only the relevant EVSE and connector"*.
    pub location: Location,
    /// Optional identification of the kWh meter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter_id: Option<OcpiString<255>>,
    /// ISO 4217 code of the currency used for this session.
    pub currency: Currency,
    /// Charging Periods that can be used to calculate and verify the total cost.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub charging_periods: Vec<ChargingPeriod>,
    /// The total cost of the session, **excluding VAT**.
    ///
    /// > *A total_cost of 0.00 means free of charge. When omitted … this does not have to mean it
    /// > is free of charge.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<Number>,
    /// The status of the session.
    pub status: SessionStatus,
    /// Timestamp when this Session was last updated (or created).
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Session {
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

impl Validate for Session {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self, v,
            id,
            start_date_time as "start_datetime",
            end_date_time as "end_datetime",
            kwh, auth_id, auth_method, location, meter_id, currency, charging_periods, status,
            last_updated,
            total_cost,
        );
        if self.end_date_time.is_some_and(|end| end < self.start_date_time) {
            v.report_at("end_datetime", ViolationCode::Inconsistent, "a session cannot end before it starts");
        }
        if self.kwh.is_negative() {
            v.report_at("kwh", ViolationCode::OutOfRange, "a session cannot charge negative energy");
        }
        crate::v2_3_0::cdrs::validate_period_sequence(
            &self.charging_periods.iter().map(|p| p.start_date_time).collect::<Vec<_>>(),
            self.start_date_time,
            self.end_date_time,
            v,
        );
        if self.status == SessionStatus::Completed && self.end_date_time.is_none() {
            v.report_at(
                "end_datetime",
                ViolationCode::MissingConditional,
                "a COMPLETED session has finished and should carry the time it finished",
            );
        }
    }
}

ocpi_lenient_enum! {
    /// The state of a session, in OCPI 2.1.1.
    ///
    /// `RESERVATION` arrived in OCPI 2.2, with the reservation pricing that needed it.
    ///
    /// Spec: 2.1.1 §mod_sessions_sessionstatus_enum
    pub enum SessionStatus {
        /// The session is accepted and active.
        Active = "ACTIVE",
        /// The session has finished successfully.
        Completed = "COMPLETED",
        /// The session is declared invalid and will not be billed.
        Invalid = "INVALID",
        /// The session is pending; it has not yet started.
        Pending = "PENDING",
    }
}

impl SessionStatus {
    /// Whether the session can still change.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timestamp_fields_use_the_2_1_1_spelling_on_the_wire() {
        let json = r#"{"id":"101","start_datetime":"2015-06-29T22:39:09Z","kwh":0,"auth_id":"DE8ACC12E46L89","auth_method":"WHITELIST","location":{"id":"LOC1","type":"ON_STREET","address":"a","city":"b","postal_code":"c","country":"NLD","coordinates":{"latitude":"51.047599","longitude":"3.729944"},"last_updated":"2015-06-29T20:39:09Z"},"currency":"EUR","status":"PENDING","last_updated":"2015-06-29T22:39:09Z"}"#;
        let session: Session = serde_json::from_str(json).unwrap();
        assert!(session.validate().is_ok());
        let encoded = serde_json::to_string(&session).unwrap();
        assert!(encoded.contains("\"start_datetime\""), "not start_date_time: {encoded}");
        assert_eq!(encoded, json);
    }

    #[test]
    fn a_violation_points_at_the_wire_field_name() {
        let mut session: Session = serde_json::from_str(
            r#"{"id":"101","start_datetime":"2015-06-29T22:39:09Z","kwh":0,"auth_id":"X","auth_method":"WHITELIST","location":{"id":"LOC1","type":"ON_STREET","address":"a","city":"b","postal_code":"c","country":"NLD","coordinates":{"latitude":"51.047599","longitude":"3.729944"},"last_updated":"2015-06-29T20:39:09Z"},"currency":"EUR","status":"PENDING","last_updated":"2015-06-29T22:39:09Z"}"#,
        )
        .unwrap();
        session.status = SessionStatus::Completed;
        let err = session.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].pointer, "/end_datetime");
    }

    #[test]
    fn the_reservation_status_arrived_later() {
        assert_eq!(SessionStatus::ALL_KNOWN.len(), 4);
        assert!(!SessionStatus::from("RESERVATION").is_known());
    }
}
