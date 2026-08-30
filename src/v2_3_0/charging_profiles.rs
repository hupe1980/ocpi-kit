//! The *Charging Profiles* module of OCPI 2.3.0: smart charging limits over time.
//!
//! *Module Identifier: `chargingprofiles`*
//!
//! Like [`commands`](super::commands), this module is asynchronous: the CPO answers with a
//! [`ChargingProfileResponse`] carrying a timeout and later POSTs the outcome to the
//! `response_url`.
//!
//! Spec: 2.3.0 §mod_charging_profiles_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_enum;
use crate::types::validate_fields;
use crate::types::{DateTime, Extensions, Number, Url, Validate, Validator, ViolationCode};

/// A request to set a charging profile on a session.
///
/// Spec: 2.3.0 §mod_charging_profiles_set_charging_profile_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct SetChargingProfile {
    /// Limits for the available power or current over time.
    pub charging_profile: ChargingProfile,
    /// URL that the [`ChargingProfileResult`] POST should be sent to.
    pub response_url: Url,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for SetChargingProfile {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, charging_profile, response_url);
    }
}

/// The CPO's immediate answer to a Charging Profile request.
///
/// Spec: 2.3.0 §mod_charging_profiles_response_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct ChargingProfileResponse {
    /// Response from the CPO on the ChargingProfile request.
    pub result: ChargingProfileResponseType,
    /// Timeout for this request in seconds.
    pub timeout: u32,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl ChargingProfileResponse {
    /// The timeout as a [`std::time::Duration`].
    #[must_use]
    pub const fn timeout_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout as u64)
    }

    /// Whether a result should be expected on the `response_url`.
    #[must_use]
    pub fn expects_result(&self) -> bool {
        self.result == ChargingProfileResponseType::Accepted
    }
}

impl Validate for ChargingProfileResponse {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, result);
        if self.result == ChargingProfileResponseType::Accepted && self.timeout == 0 {
            v.report_at(
                "timeout",
                ViolationCode::OutOfRange,
                "an accepted request needs a non-zero timeout for the eMSP to wait on",
            );
        }
    }
}

/// The asynchronous outcome of a GET for the active charging profile.
///
/// Spec: 2.3.0 §mod_charging_profiles_active_charging_profiles_result_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct ActiveChargingProfileResult {
    /// Whether the EVSE was able to process the request.
    pub result: ChargingProfileResultType,
    /// The requested profile, present when `result` is `ACCEPTED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ActiveChargingProfile>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for ActiveChargingProfileResult {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, result, profile);
        match (self.result, self.profile.is_some()) {
            (ChargingProfileResultType::Accepted, false) => v.report_at(
                "profile",
                ViolationCode::MissingConditional,
                "an ACCEPTED result carries the requested ActiveChargingProfile",
            ),
            (ChargingProfileResultType::Rejected | ChargingProfileResultType::Unknown, true) => v.report_at(
                "profile",
                ViolationCode::Inconsistent,
                "a profile is only returned when the result is ACCEPTED",
            ),
            _ => {}
        }
    }
}

/// The asynchronous outcome of a PUT of a charging profile.
///
/// Spec: 2.3.0 §mod_charging_profiles_charging_profiles_result_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ChargingProfileResult {
    /// Whether the EVSE was able to process the new or updated charging profile.
    pub result: ChargingProfileResultType,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for ChargingProfileResult {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, result);
    }
}

/// The asynchronous outcome of a DELETE of a charging profile.
///
/// Spec: 2.3.0 §mod_charging_profiles_clear_profiles_result_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ClearProfileResult {
    /// Whether the EVSE was able to process the removal of the charging profile.
    pub result: ChargingProfileResultType,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for ClearProfileResult {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, result);
    }
}

/// The charging profile the Charge Point has calculated, with the time it did so.
///
/// Spec: 2.3.0 §mod_charging_profiles_active_charging_profile_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct ActiveChargingProfile {
    /// When the Charge Point calculated this profile.
    ///
    /// > *All time measurements within the profile are relative to this timestamp.*
    pub start_date_time: DateTime,
    /// The profile itself.
    pub charging_profile: ChargingProfile,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for ActiveChargingProfile {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, start_date_time, charging_profile);
    }
}

/// A list of charging periods with a power or current limit each.
///
/// Spec: 2.3.0 §mod_charging_profiles_charging_profile_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct ChargingProfile {
    /// Starting point of an absolute profile.
    ///
    /// > *If absent the profile will be relative to start of charging.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date_time: Option<DateTime>,
    /// Duration of the charging profile in seconds.
    ///
    /// > *If the duration is left empty, the last period will continue indefinitely or until end
    /// > of the transaction in case `start_date_time` is absent.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,
    /// The unit of measure the limits are expressed in.
    pub charging_rate_unit: ChargingRateUnit,
    /// Minimum charging rate supported by the EV, in `charging_rate_unit`.
    ///
    /// > *Accepts at most one digit fraction (e.g. 8.1).*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_charging_rate: Option<Number>,
    /// Periods defining maximum power or current usage over time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub charging_profile_period: Vec<ChargingProfilePeriod>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for ChargingProfile {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, start_date_time, min_charging_rate, charging_profile_period,);
        if self.min_charging_rate.is_some_and(|r| r.scale() > 1) {
            v.report_at(
                "min_charging_rate",
                ViolationCode::OutOfRange,
                "accepts at most one digit fraction (e.g. 8.1)",
            );
        }
        // "The value of StartPeriod also defines the stop time of the previous period", which
        // only makes sense for a strictly increasing list.
        let mut previous: Option<u64> = None;
        for (i, period) in self.charging_profile_period.iter().enumerate() {
            if previous.is_some_and(|p| period.start_period <= p) {
                v.enter("charging_profile_period");
                v.enter(&i.to_string());
                v.report_at(
                    "start_period",
                    ViolationCode::Inconsistent,
                    "charging profile periods must be in strictly increasing order",
                );
                v.leave();
                v.leave();
            }
            previous = Some(period.start_period);
        }
        if let (Some(duration), Some(last)) = (self.duration, previous)
            && last >= duration
        {
            v.report_at(
                "duration",
                ViolationCode::Inconsistent,
                "the profile ends before its last period starts",
            );
        }
    }
}

/// One time period within a [`ChargingProfile`].
///
/// Spec: 2.3.0 §mod_charging_profiles_charging_profile_period_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ChargingProfilePeriod {
    /// Start of the period, in seconds from the start of the profile.
    pub start_period: u64,
    /// Charging rate limit during this period, in the profile's `charging_rate_unit`.
    ///
    /// > *Accepts at most one digit fraction (e.g. 8.1).*
    pub limit: Number,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for ChargingProfilePeriod {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, limit);
        if self.limit.is_negative() {
            v.report_at("limit", ViolationCode::OutOfRange, "a charging rate limit cannot be negative");
        }
        if self.limit.scale() > 1 {
            v.report_at("limit", ViolationCode::OutOfRange, "accepts at most one digit fraction (e.g. 8.1)");
        }
    }
}

ocpi_enum! {
    /// The unit a charging profile is defined in.
    ///
    /// Spec: 2.3.0 §mod_charging_profiles_chargingrateunit
    pub enum ChargingRateUnit {
        /// Watts: the total allowed charging power, usually convenient for DC.
        Watts = "W",
        /// Amperes per phase — not the sum of all phases — usually convenient for AC.
        Amperes = "A",
    }
}

ocpi_enum! {
    /// The CPO's immediate answer to a Charging Profile request.
    ///
    /// Spec: 2.3.0 §mod_charging_profiles_responsetype_enum
    pub enum ChargingProfileResponseType {
        /// Accepted by the CPO; the request will be forwarded to the EVSE.
        Accepted = "ACCEPTED",
        /// Charging Profiles are not supported by this CPO, Charge Point or EVSE.
        NotSupported = "NOT_SUPPORTED",
        /// Rejected by the CPO.
        Rejected = "REJECTED",
        /// Rejected by the CPO: requests are sent more often than allowed.
        TooOften = "TOO_OFTEN",
        /// The Session in the requested command is not known by this CPO.
        UnknownSession = "UNKNOWN_SESSION",
    }
}

ocpi_enum! {
    /// The EVSE's eventual answer, delivered to the `response_url`.
    ///
    /// Deliberately distinct from [`ChargingProfileResponseType`].
    ///
    /// Spec: 2.3.0 §mod_charging_profiles_resulttype_enum
    pub enum ChargingProfileResultType {
        /// Accepted by the EVSE.
        Accepted = "ACCEPTED",
        /// Rejected by the EVSE.
        Rejected = "REJECTED",
        /// No Charging Profiles were found by the EVSE matching the request.
        Unknown = "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn period(start: u64, limit: &str) -> ChargingProfilePeriod {
        ChargingProfilePeriod {
            start_period: start,
            limit: limit.parse().unwrap(),
            extensions: Extensions::new(),
        }
    }

    #[test]
    fn profile_periods_must_increase() {
        let good = ChargingProfile::builder()
            .charging_rate_unit(ChargingRateUnit::Amperes)
            .charging_profile_period(vec![period(0, "16"), period(1800, "8")])
            .build();
        assert!(good.validate().is_ok());

        let backwards = ChargingProfile::builder()
            .charging_rate_unit(ChargingRateUnit::Amperes)
            .charging_profile_period(vec![period(1800, "16"), period(0, "8")])
            .build();
        let err = backwards.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].pointer, "/charging_profile_period/1/start_period");
    }

    #[test]
    fn limits_take_at_most_one_fractional_digit() {
        assert!(period(0, "8.1").validate().is_ok());
        assert!(period(0, "8.15").validate().is_err());
        assert!(period(0, "-1").validate().is_err());
    }

    #[test]
    fn an_accepted_active_profile_result_must_carry_the_profile() {
        let empty =
            ActiveChargingProfileResult::builder().result(ChargingProfileResultType::Accepted).build();
        assert_eq!(empty.validate().unwrap_err().as_slice()[0].pointer, "/profile");

        let rejected =
            ActiveChargingProfileResult::builder().result(ChargingProfileResultType::Rejected).build();
        assert!(rejected.validate().is_ok());
    }

    #[test]
    fn duration_must_cover_the_last_period() {
        let p = ChargingProfile::builder()
            .charging_rate_unit(ChargingRateUnit::Watts)
            .duration(1800u64)
            .charging_profile_period(vec![period(0, "11000"), period(3600, "7400")])
            .build();
        assert!(p.validate().unwrap_err().as_slice().iter().any(|x| x.pointer == "/duration"));
    }
}
