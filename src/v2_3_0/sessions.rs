//! The *Sessions* module of OCPI 2.3.0: the live view of a charging session.
//!
//! *Module Identifier: `sessions`* — Data owner: CPO.
//!
//! > *The Session object is dynamic as it reflects the current state of the charging session.
//! > The information is meant to be viewed by the driver while the charging session is ongoing.*
//!
//! Spec: 2.3.0 §mod_sessions_sessions_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    CiString, CountryCode, Currency, DateTime, Extensions, Number, OcpiString, PartyId, PartyRef, Validate,
    Validator, ViolationCode,
};
use crate::{ocpi_enum, ocpi_lenient_enum};

use super::cdrs::{AuthMethod, CdrDimensionType, CdrToken, ChargingPeriod};
use super::types::Price;

/// One charging session, as it stands right now.
///
/// > *That doesn't mean it is required that energy has been transferred between EV and the
/// > Charge Point. … as the EV was connected to the Charge Point, some form of start tariff, park
/// > tariff or reservation cost might be relevant.*
///
/// Spec: 2.3.0 §mod_sessions_session_object
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
    ///
    /// > *When the session is still `PENDING`, this field SHALL be set to the time the Session
    /// > was created at the Charge Point. When a Session goes from `PENDING` to `ACTIVE`, this
    /// > field SHALL be updated to the moment the Session went to `ACTIVE`.*
    pub start_date_time: DateTime,
    /// When the session was completed. Charging may have finished earlier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date_time: Option<DateTime>,
    /// How many kWh were charged.
    pub kwh: Number,
    /// Token used to start this charging session.
    pub cdr_token: CdrToken,
    /// Method used for authentication. This might change during a session.
    pub auth_method: AuthMethod,
    /// Reference to the authorization given by the eMSP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_reference: Option<CiString<36>>,
    /// `Location.id` on which the charging session is or was happening.
    pub location_id: CiString<36>,
    /// `EVSE.uid` on which the charging session is or was happening.
    ///
    /// > *Allowed to be set to `#NA` when this session is created for a reservation, but no EVSE
    /// > yet assigned to the driver.*
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
    ///
    /// > *A total_cost of 0.00 means free of charge. When omitted … it does not imply the session
    /// > is/was free of charge.*
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
    ///
    /// Both fields may carry the `#NA` sentinel while a reservation has not been taken up.
    #[must_use]
    pub fn has_assigned_evse(&self) -> bool {
        !self.evse_uid.is_not_available() && !self.connector_id.is_not_available()
    }

    /// Whether the session has reached a state that will not change again.
    #[must_use]
    pub fn is_final(&self) -> bool {
        matches!(self.status, SessionStatus::Completed | SessionStatus::Invalid)
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
        crate::v2_3_0::cdrs::validate_period_sequence(
            &self.charging_periods.iter().map(|p| p.start_date_time).collect::<Vec<_>>(),
            self.start_date_time,
            self.end_date_time,
            v,
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
        // A COMPLETED session has finished: "No more modifications will be made to the Session
        // object using this state."
        if self.status == SessionStatus::Completed && self.end_date_time.is_none() {
            v.report_at(
                "end_date_time",
                ViolationCode::MissingConditional,
                "a COMPLETED session has finished and should carry the time it finished",
            );
        }
        if self.status == SessionStatus::Reservation && self.has_assigned_evse() {
            // Not a violation, just worth noting that the spec expects `#NA` here until the
            // driver arrives; a CPO that already knows the EVSE may legitimately name it.
        }
    }
}

/// The charging preferences an EV driver set for a session.
///
/// Spec: 2.3.0 §mod_sessions_charging_preferences_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct ChargingPreferences {
    /// Type of Smart Charging Profile selected by the driver.
    ///
    /// > *The ProfileType has to be supported at the Connector and for every supported
    /// > ProfileType, a Tariff MUST be provided.*
    pub profile_type: ProfileType,
    /// Expected departure, as an estimate given by the driver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub departure_time: Option<DateTime>,
    /// Requested amount of energy in kWh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_need: Option<Number>,
    /// Whether the driver allows their EV to be discharged. Default if omitted: `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discharge_allowed: Option<bool>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl ChargingPreferences {
    /// Whether discharging is allowed, applying the spec's default of `false`.
    #[must_use]
    pub fn discharge_allowed_or_default(&self) -> bool {
        self.discharge_allowed.unwrap_or(false)
    }
}

impl Validate for ChargingPreferences {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, profile_type, departure_time, energy_need);
        if self.energy_need.is_some_and(Number::is_negative) {
            v.report_at("energy_need", ViolationCode::OutOfRange, "cannot be negative");
        }
    }
}

ocpi_enum! {
    /// Possible responses to a `PUT` of [`ChargingPreferences`].
    ///
    /// > *If a PUT with ChargingPreferences is received for an EVSE that does not have the
    /// > capability `CHARGING_PREFERENCES_CAPABLE`, the receiver should respond with an HTTP
    /// > status of 404 and an OCPI status code of 2001.*
    ///
    /// Spec: 2.3.0 §mod_sessions_charging_preferences_response_enum
    pub enum ChargingPreferencesResponse {
        /// Accepted; the EVSE will try to accomplish them, without guarantee.
        Accepted = "ACCEPTED",
        /// The CPO requires `departure_time` for preference-based smart charging.
        DepartureRequired = "DEPARTURE_REQUIRED",
        /// The CPO requires `energy_need` for preference-based smart charging.
        EnergyNeedRequired = "ENERGY_NEED_REQUIRED",
        /// The preferences contain a demand the EVSE knows it cannot fulfil.
        NotPossible = "NOT_POSSIBLE",
        /// `profile_type` contains a value the EVSE does not support.
        ProfileTypeNotSupported = "PROFILE_TYPE_NOT_SUPPORTED",
    }
}

ocpi_enum! {
    /// The smart charging profile a driver can choose between.
    ///
    /// Each profile type a Connector supports needs its own Tariff, so the driver can see what
    /// each option costs. See [`TariffType`](crate::v2_3_0::tariffs::TariffType).
    ///
    /// Spec: 2.3.0 §mod_sessions_profile_type_enum
    pub enum ProfileType {
        /// The driver wants the cheapest charging profile possible.
        Cheap = "CHEAP",
        /// The driver wants their EV charged as quickly as possible.
        Fast = "FAST",
        /// The driver wants as much regenerative (green) energy as possible.
        Green = "GREEN",
        /// The driver has no special preferences.
        Regular = "REGULAR",
    }
}

ocpi_lenient_enum! {
    /// The state of a session.
    ///
    /// Spec: 2.3.0 §mod_sessions_sessionstatus_enum
    pub enum SessionStatus {
        /// Accepted and active; all pre-conditions were met.
        Active = "ACTIVE",
        /// Finished successfully. No more modifications will be made.
        Completed = "COMPLETED",
        /// Declared invalid; will not be billed.
        Invalid = "INVALID",
        /// Not yet started; the initial state. It might never become active.
        Pending = "PENDING",
        /// Started due to a reservation; charging has not yet started.
        Reservation = "RESERVATION",
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

    fn session(status: SessionStatus, end: Option<&str>) -> Session {
        Session::builder()
            .country_code("NL")
            .party_id("STK")
            .id("101")
            .start_date_time("2020-03-09T10:17:09Z".parse::<DateTime>().unwrap())
            .maybe_end_date_time(end.map(|e| e.parse::<DateTime>().unwrap()))
            .kwh(Number::ZERO)
            .cdr_token(
                super::super::cdrs::CdrToken::builder()
                    .country_code("NL")
                    .party_id("TST")
                    .uid("123abc")
                    .token_type(super::super::tokens::TokenType::Rfid)
                    .contract_id("NL-TST-C12345678-S")
                    .build(),
            )
            .auth_method(AuthMethod::Whitelist)
            .location_id("LOC1")
            .evse_uid("3256")
            .connector_id("1")
            .currency("EUR")
            .status(status)
            .last_updated("2020-03-09T10:17:09Z".parse::<DateTime>().unwrap())
            .build()
    }

    #[test]
    fn a_completed_session_must_say_when_it_ended() {
        assert!(session(SessionStatus::Active, None).validate().is_ok());
        let err = session(SessionStatus::Completed, None).validate().unwrap_err();
        assert_eq!(err.as_slice()[0].pointer, "/end_date_time");
        assert!(session(SessionStatus::Completed, Some("2020-03-09T11:21:00Z")).validate().is_ok());
    }

    #[test]
    fn a_session_cannot_end_before_it_starts() {
        let s = session(SessionStatus::Completed, Some("2020-03-09T09:00:00Z"));
        assert!(s.validate().unwrap_err().as_slice().iter().any(|x| x.code == ViolationCode::Inconsistent));
    }

    #[test]
    fn reservation_sessions_may_carry_the_na_sentinel() {
        let mut s = session(SessionStatus::Reservation, None);
        s.evse_uid = CiString::new("#NA").unwrap();
        s.connector_id = CiString::new("#NA").unwrap();
        assert!(!s.has_assigned_evse());
        assert!(s.validate().is_ok());
    }

    #[test]
    fn terminal_states_match_the_spec_table() {
        assert!(SessionStatus::Completed.is_terminal());
        assert!(SessionStatus::Invalid.is_terminal());
        assert!(!SessionStatus::Pending.is_terminal());
    }
}
