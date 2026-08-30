//! The *Sessions* module of OCPI 2.2.1, as a delta from
//! [`v2_3_0::sessions`](crate::v2_3_0::sessions).
//!
//! Only [`Session`] is redefined, because its `total_cost` is the 2.2.1
//! [`Price`] and its `cdr_token` the 2.2.1
//! [`CdrToken`].
//!
//! Spec: 2.2.1 §mod_sessions_sessions_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    CiString, CountryCode, Currency, DateTime, Extensions, Number, OcpiString, PartyId, PartyRef, Validate,
    Validator, ViolationCode,
};

use super::cdrs::{AuthMethod, CdrDimensionType, CdrToken, ChargingPeriod};
use super::types::Price;

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::sessions::{
    ChargingPreferences, ChargingPreferencesResponse, ProfileType, SessionStatus,
};

/// One charging session, as it stands right now, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_sessions_session_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Session {
    /// ISO-3166 alpha-2 country code of the CPO that 'owns' this Session.
    pub country_code: CountryCode,
    /// ID of the CPO that 'owns' this Session.
    pub party_id: PartyId,
    /// The unique id that identifies the charging session in the CPO platform.
    pub id: CiString<36>,
    /// When the session became `ACTIVE` in the Charge Point.
    pub start_date_time: DateTime,
    /// When the session was completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date_time: Option<DateTime>,
    /// How many kWh were charged.
    pub kwh: Number,
    /// Token used to start this charging session.
    pub cdr_token: CdrToken,
    /// Method used for authentication.
    pub auth_method: AuthMethod,
    /// Reference to the authorization given by the eMSP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_reference: Option<CiString<36>>,
    /// `Location.id` on which the charging session is or was happening.
    pub location_id: CiString<36>,
    /// `EVSE.uid` on which the charging session is or was happening. May be `#NA`.
    pub evse_uid: CiString<36>,
    /// `Connector.id` where the charging session is or was happening. May be `#NA`.
    pub connector_id: CiString<36>,
    /// Optional identification of the kWh meter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter_id: Option<OcpiString<255>>,
    /// ISO 4217 code of the currency used for this session.
    pub currency: Currency,
    /// Charging Periods that can be used to calculate and verify the total cost.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub charging_periods: Vec<ChargingPeriod>,
    /// The total cost of the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<Price>,
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
    /// The CPO that owns this Session.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }

    /// Whether an EVSE and connector have been assigned yet.
    #[must_use]
    pub fn has_assigned_evse(&self) -> bool {
        !self.evse_uid.is_not_available() && !self.connector_id.is_not_available()
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

impl Validate for Session {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            country_code,
            party_id,
            id,
            start_date_time,
            end_date_time,
            kwh,
            cdr_token,
            auth_method,
            authorization_reference,
            location_id,
            evse_uid,
            connector_id,
            meter_id,
            currency,
            charging_periods,
            total_cost,
            status,
            last_updated,
        );
        if self.end_date_time.is_some_and(|end| end < self.start_date_time) {
            v.report_at(
                "end_date_time",
                ViolationCode::Inconsistent,
                "a session cannot end before it starts",
            );
        }
        if self.kwh.is_negative() {
            v.report_at("kwh", ViolationCode::OutOfRange, "a session cannot charge negative energy");
        }
        if self.status == SessionStatus::Completed && self.end_date_time.is_none() {
            v.report_at(
                "end_date_time",
                ViolationCode::MissingConditional,
                "a COMPLETED session has finished and should carry the time it finished",
            );
        }
    }
}
