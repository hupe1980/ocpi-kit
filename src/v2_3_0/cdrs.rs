//! The *CDRs* module of OCPI 2.3.0: sealed records of what a session cost.
//!
//! *Module Identifier: `cdrs`* — Data owner: CPO.
//!
//! > *The CDR … can be thought of as sealed, preserving the information valid at the moment in
//! > time the underlying session was started. This is a requirement of the main use case for
//! > CDRs, namely invoicing.*
//!
//! Spec: 2.3.0 §mod_cdrs_cdrs_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    CiString, ContractId, CountryCode, Currency, DateTime, EvseId, Extensions, Number, OcpiString, PartyId,
    PartyRef, Validate, Validator, ViolationCode,
};
use crate::{ocpi_enum, ocpi_lenient_enum};

use super::locations::{ConnectorFormat, ConnectorType, GeoLocation, PowerType};
use super::tariffs::Tariff;
use super::tokens::TokenType;
use super::types::Price;

/// The maximum length of a normal, non-credit CDR id.
///
/// > *This field is longer than the usual 36 characters to allow for credit CDRs to have
/// > something appended to the original ID. Normal (non-credit) CDRs SHALL only have an ID with
/// > a maximum length of 36.*
pub const NON_CREDIT_ID_MAX_LEN: usize = 36;

/// A Charge Detail Record: one charging session and its costs.
///
/// Spec: 2.3.0 §mod_cdrs_cdr_object
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
    /// When the session was completed. Charging may have finished earlier.
    pub end_date_time: DateTime,
    /// The Session this CDR belongs to.
    ///
    /// > *Is only allowed to be omitted when the CPO has not implemented the Sessions module or
    /// > this CDR is the result of a reservation that never became a charging session.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<CiString<36>>,
    /// Token used to start this charging session.
    pub cdr_token: CdrToken,
    /// Method used for authentication. The **last** method used during the session.
    pub auth_method: AuthMethod,
    /// Reference to the authorization given by the eMSP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_reference: Option<CiString<36>>,
    /// The Booking this CDR also belongs to.
    ///
    /// > *Is only allowed to be omitted when the Session was reserved.*
    ///
    /// Added by the OCPI 2.3.0 `bookings` release branch, so it is behind the `bookings` feature.
    ///
    /// Spec: 2.3.0-bookings §mod_cdrs_cdr_object
    #[cfg(feature = "bookings")]
    #[cfg_attr(docsrs, doc(cfg(feature = "bookings")))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub booking_id: Option<CiString<36>>,
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
    /// Total duration of the charging session, charging and not charging, in hours.
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
    /// Human-readable remark, e.g. the reason a transaction was stopped.
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
    ///
    /// > *The actual charging duration … can be calculated:
    /// > `total_charging_time = total_time - total_parking_time`.*
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

    /// Every charging period with the interval it actually covers.
    ///
    /// A [`ChargingPeriod`] carries only its `start_date_time`: *"A period ends when the next one
    /// starts"*, and the last one ends at the CDR's `end_date_time`. Deriving that is three lines
    /// and an off-by-one, and every consumer that needs energy over time writes it.
    ///
    /// ```
    /// # use ocpi_kit::v2_3_0::cdrs::{Cdr, CdrDimensionType};
    /// # fn f(cdr: &Cdr) {
    /// for span in cdr.period_spans() {
    ///     if let Some(kwh) = span.volume(CdrDimensionType::Energy) {
    ///         println!("{} → {}: {kwh} kWh", span.start, span.end);
    ///     }
    /// }
    /// # }
    /// ```
    ///
    /// # What a period is, and is not
    ///
    /// A period is a **total, not a curve**. It says 4.3 kWh flowed between two instants and
    /// nothing about how. Re-cutting these intervals onto a finer grid — quarter hours, say —
    /// therefore needs an assumption the CDR does not carry, and the specification declines to
    /// make it: it puts the obligation on the CPO to start a new period *"every moment/event that
    /// has relevance for the total costs"* instead. Apportioning by elapsed time is the usual
    /// choice and is usually close, but it is the caller's assumption to make and to record, not
    /// this crate's to hide. [`tariffs`](crate::tariffs) takes the same position, and reports a
    /// `PeriodSpansPriceChange` note when a period outlasts the price that governs it.
    ///
    /// Periods are yielded in the order the CDR gives them. `validate()` reports a CDR whose
    /// periods are out of order, so check that first if the ordering matters — which for an
    /// interval it does.
    ///
    /// # Only on a CDR
    ///
    /// [`Session`](crate::v2_3_0::sessions::Session) carries the same periods and does not get
    /// this, on purpose. A running session has no `end_date_time`, so its final period has no
    /// honest end; and its whole list is provisional — *"any `charging_periods` from the existing
    /// object SHALL be replaced by the `charging_periods` from the newly received Session
    /// object"*. A CDR is the record that stops changing, which is what an interval needs.
    ///
    /// Spec: 2.3.0 §mod_cdrs_chargingperiod_class
    pub fn period_spans(&self) -> impl Iterator<Item = PeriodSpan<'_>> {
        self.charging_periods.iter().enumerate().map(move |(i, period)| PeriodSpan {
            start: period.start_date_time,
            end: self.charging_periods.get(i + 1).map_or(self.end_date_time, |next| next.start_date_time),
            period,
        })
    }

    /// How long after the session ended this CDR was written, in seconds.
    ///
    /// A CDR may arrive well after the session it records, and a consumer with a filing deadline
    /// needs to know by how much. `last_updated` is the moment to measure from because a CDR has
    /// no later one: *"Because a CDR is for billing purposes, it cannot be changed or replaced
    /// once sent to the eMSP. Changes are simply not allowed."* So on a CDR — unlike every other
    /// OCPI object — `last_updated` is when it was created.
    ///
    /// `None` when the CDR carries the `1970-1-1T00:00:00Z` placeholder timestamps the
    /// specification permits, which would otherwise report half a century of latency and poison
    /// an average. See [`has_placeholder_timestamps`](Self::has_placeholder_timestamps).
    ///
    /// Negative values are returned as they are. They mean the CPO's clock disagrees with the
    /// session it recorded, which is worth seeing rather than clamping away.
    ///
    /// Spec: 2.3.0 §mod_cdrs_cdr_object
    #[must_use]
    pub fn delivery_latency_seconds(&self) -> Option<i64> {
        if self.has_placeholder_timestamps() {
            return None;
        }
        Some(self.last_updated.unix_timestamp() - self.end_date_time.unix_timestamp())
    }

    /// Whether the timestamps are the `1970-1-1T00:00:00Z` placeholder the spec permits.
    ///
    /// > *If the MSP and CPO both agree that they accept CDRs that miss either or both the
    /// > `start_date_time` and `end_date_time` … the CPO could send a CDR where the
    /// > `start_date_time` and/or `end_date_time` are set to "1970-1-1T00:00:00Z".*
    #[must_use]
    pub fn has_placeholder_timestamps(&self) -> bool {
        self.start_date_time.unix_timestamp() == 0 || self.end_date_time.unix_timestamp() == 0
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
        #[cfg(feature = "bookings")]
        validate_fields!(self, v, booking_id);

        if self.charging_periods.is_empty() {
            v.report_at(
                "charging_periods",
                ViolationCode::EmptyRequiredList,
                "a CDR has cardinality `+` charging_periods: at least one is required",
            );
        }

        // "Normal (non-credit) CDRs SHALL only have an ID with a maximum length of 36."
        if !self.is_credit() && self.id.len() > NON_CREDIT_ID_MAX_LEN {
            v.report_at(
                "id",
                ViolationCode::TooLong,
                format!(
                    "a non-credit CDR id may be at most {NON_CREDIT_ID_MAX_LEN} characters; \
                     the extra length is reserved for credit CDRs"
                ),
            );
        }

        // "When set to true, this is a Credit CDR, and the field credit_reference_id needs to be
        //  set as well."
        if self.is_credit() && self.credit_reference_id.is_none() {
            v.report_at(
                "credit_reference_id",
                ViolationCode::MissingConditional,
                "is required to be set for a Credit CDR",
            );
        }
        if !self.is_credit() && self.credit_reference_id.is_some() {
            v.report_at(
                "credit",
                ViolationCode::Inconsistent,
                "credit_reference_id is set, so `credit` should be true",
            );
        }

        if !self.has_placeholder_timestamps() && self.end_date_time < self.start_date_time {
            v.report_at(
                "end_date_time",
                ViolationCode::Inconsistent,
                "a session cannot end before it starts",
            );
        }

        // The energy total has to agree with the metered ENERGY dimensions.
        let metered = self.dimension_total(CdrDimensionType::Energy);
        if !self.charging_periods.is_empty()
            && self
                .charging_periods
                .iter()
                .any(|p| p.dimensions.iter().any(|d| d.dimension_type == CdrDimensionType::Energy))
            && metered != self.total_energy
        {
            v.report_at(
                "total_energy",
                ViolationCode::Inconsistent,
                format!(
                    "is {}, but the ENERGY dimensions of the charging periods add up to {metered}",
                    self.total_energy
                ),
            );
        }

        validate_period_sequence(
            &self.charging_periods.iter().map(|p| p.start_date_time).collect::<Vec<_>>(),
            self.start_date_time,
            Some(self.end_date_time),
            v,
        );

        if self.total_parking_time.is_some_and(|p| p > self.total_time) {
            v.report_at(
                "total_parking_time",
                ViolationCode::Inconsistent,
                "cannot exceed total_time, of which it is a part",
            );
        }

        // "SHALL only be used in Sessions" — these dimensions have no meaning in a CDR.
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
                        format!(
                            "{} is marked \"Session Only\" and SHALL NOT appear in a CDR",
                            dim.dimension_type
                        ),
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

/// The token that started a session, as recorded in a CDR or Session.
///
/// Spec: 2.3.0 §mod_cdrs_cdr_token_object
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

/// The parts of a Location that a CDR needs, frozen at the start of the session.
///
/// Spec: 2.3.0 §mod_cdrs_cdr_location_class
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
    /// The EVSE's technical identifier. May be `#NA` for a reservation that never charged.
    pub evse_uid: CiString<36>,
    /// The EVSE's human-readable ID. May be `#NA` for a reservation that never charged.
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
    ///
    /// The spec marks `evse_uid`, `evse_id` and `connector_id` as *"allowed to be set to `#NA`
    /// when this CDR is created for a reservation that never resulted in a charging session"*,
    /// in which case the connector fields *"can be set to any value and should be ignored"*.
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

/// A period of a session during which the values that influence its cost were stable.
///
/// > *A CPO SHALL at least start (and add) a ChargingPeriod every moment/event that has relevance
/// > for the total costs of a CDR.*
///
/// Spec: 2.3.0 §mod_cdrs_chargingperiod_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct ChargingPeriod {
    /// Start of the charging period. A period ends when the next one starts.
    pub start_date_time: DateTime,
    /// Relevant values for this charging period. Cardinality `+`.
    pub dimensions: Vec<CdrDimension>,
    /// The Tariff relevant during this period. When absent, no Tariff is relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tariff_id: Option<CiString<36>>,
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

/// A [`ChargingPeriod`] together with the interval it covers.
///
/// Produced by [`Cdr::period_spans`], which is where the boundary rule is explained.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeriodSpan<'a> {
    /// The period's own `start_date_time`.
    pub start: DateTime,
    /// The next period's start, or the CDR's `end_date_time` for the last one.
    pub end: DateTime,
    /// The period itself.
    pub period: &'a ChargingPeriod,
}

impl PeriodSpan<'_> {
    /// The volume recorded for one dimension in this period.
    #[must_use]
    pub fn volume(&self, dimension: CdrDimensionType) -> Option<Number> {
        self.period.volume(dimension)
    }

    /// How long the interval is, in seconds. Negative if the CDR's periods are out of order.
    #[must_use]
    pub fn duration_seconds(&self) -> i64 {
        self.end.unix_timestamp() - self.start.unix_timestamp()
    }
}

impl Validate for ChargingPeriod {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, start_date_time, dimensions, tariff_id);
        if self.dimensions.is_empty() {
            v.report_at(
                "dimensions",
                ViolationCode::EmptyRequiredList,
                "a ChargingPeriod has cardinality `+` dimensions: at least one is required",
            );
        }
        let mut seen: Vec<&CdrDimensionType> = Vec::new();
        for d in &self.dimensions {
            if seen.contains(&&d.dimension_type) {
                v.report_at(
                    "dimensions",
                    ViolationCode::Inconsistent,
                    format!("the dimension {} appears more than once in one period", d.dimension_type),
                );
            }
            seen.push(&d.dimension_type);
        }
    }
}

/// Checks that a list of Charging Periods is a sequence a session could actually have had.
///
/// # Why this is worth checking
///
/// Nothing in the property tables says the periods are ordered, but everything built on them
/// assumes it. `step_size` is defined in terms of *"the last relevant PriceComponent"* and
/// *"the last time-based period"*; a period's duration is only knowable as the gap to the next
/// one; and a pricing engine reading them out of order will quietly bill the wrong rate.
///
/// It is also the failure that shows up in practice. Charging periods arrive from a CSMS through
/// a CPO's own aggregation, and a merge that loses the sort is invisible in every field-by-field
/// check — the objects are all individually valid.
///
/// So this reports three things, each as a [`ViolationCode::Inconsistent`] at the offending
/// index: a period that does not start after the one before it, one that starts before the
/// session did, and one that starts at or after the session ended.
///
/// Spec: 2.3.0 §mod_cdrs_cdr_object, §mod_cdrs_step_size
pub fn validate_period_sequence(
    starts: &[DateTime],
    session_start: DateTime,
    session_end: Option<DateTime>,
    v: &mut Validator,
) {
    let mut previous: Option<DateTime> = None;
    for (i, start) in starts.iter().copied().enumerate() {
        let at = |v: &mut Validator, message: String| {
            v.enter("charging_periods");
            v.enter(&i.to_string());
            v.report_at("start_date_time", ViolationCode::Inconsistent, message);
            v.leave();
            v.leave();
        };
        if let Some(previous) = previous
            && start <= previous
        {
            at(
                v,
                format!(
                    "is {start}, which is not after the previous period's {previous}; \
                     charging periods have to be in order for `step_size` and for a period's \
                     own duration to mean anything"
                ),
            );
        }
        if start < session_start {
            at(v, format!("is {start}, before the session started at {session_start}"));
        }
        if let Some(end) = session_end
            && start >= end
        {
            at(v, format!("is {start}, at or after the session ended at {end}"));
        }
        previous = Some(start);
    }
}

/// One measured quantity within a [`ChargingPeriod`].
///
/// Spec: 2.3.0 §mod_cdrs_cdrdimension_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CdrDimension {
    /// Type of CDR dimension.
    #[serde(rename = "type")]
    pub dimension_type: CdrDimensionType,
    /// Volume of the dimension consumed, measured according to the dimension type.
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
        if self.dimension_type == CdrDimensionType::StateOfCharge {
            let pct = self.volume;
            if pct < Number::ZERO || pct > Number::from(100u32) {
                v.report_at(
                    "volume",
                    ViolationCode::OutOfRange,
                    "STATE_OF_CHARGE is a percentage: values allowed are 0 to 100",
                );
            }
        }
        if !self.dimension_type.may_be_negative() && self.volume.is_negative() {
            v.report_at(
                "volume",
                ViolationCode::OutOfRange,
                format!("{} cannot be negative", self.dimension_type),
            );
        }
    }
}

/// Signed metering data, for German *Eichrecht* and comparable regimes.
///
/// Spec: 2.3.0 §mod_cdrs_signed_data_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct SignedData {
    /// The name of the encoding used, as given by a company or group of companies.
    ///
    /// Known implementations include `OCMF`, `Alfen Eichrecht`, `EDL40 E-Mobility Extension` and
    /// `EDL40 Mennekes`.
    pub encoding_method: CiString<36>,
    /// Version of the encoding method, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_method_version: Option<i32>,
    /// Public key used to sign the data, base64 encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<OcpiString<512>>,
    /// One or more signed values. Cardinality `+`.
    pub signed_values: Vec<SignedValue>,
    /// URL where an EV driver can check the signed data of a charging session.
    ///
    /// A `string(512)`, not the `URL` type every other URL-shaped field in OCPI uses — which is
    /// `string(255)`. Modelling it as a [`Url`](crate::types::Url) would report a conformant
    /// 300-character link as `TooLong` and refuse to construct one, so it is the string the
    /// specification says it is. [`Url::new_lenient`](crate::types::Url::new_lenient) turns it
    /// into one when a caller wants that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<OcpiString<512>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl SignedData {
    /// The signed value recorded for one `nature`, compared case-insensitively.
    ///
    /// > *Possible values at moment of writing: Start, End, Intermediate. Others might be added
    /// > later.*
    ///
    /// Open by design, so this takes a `&str` rather than an enum: a peer is free to record a
    /// nature this crate has never heard of, and losing it would defeat the point of the object.
    #[must_use]
    pub fn value_for(&self, nature: &str) -> Option<&SignedValue> {
        self.signed_values.iter().find(|v| v.nature.eq_ignore_case(nature))
    }

    /// The `Start` reading, if the CPO recorded one.
    #[must_use]
    pub fn start_value(&self) -> Option<&SignedValue> {
        self.value_for("Start")
    }

    /// The `End` reading, if the CPO recorded one.
    #[must_use]
    pub fn end_value(&self) -> Option<&SignedValue> {
        self.value_for("End")
    }
}

impl Validate for SignedData {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, encoding_method, public_key, signed_values, url,);
        if self.signed_values.is_empty() {
            v.report_at(
                "signed_values",
                ViolationCode::EmptyRequiredList,
                "SignedData has cardinality `+` signed_values: at least one is required",
            );
        }
    }
}

/// One signed and plain value pair.
///
/// Spec: 2.3.0 §mod_cdrs_signed_value_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SignedValue {
    /// Nature of the value: the event it belongs to.
    ///
    /// > *Possible values at moment of writing: Start, End, Intermediate. Others might be added
    /// > later.*
    pub nature: CiString<32>,
    /// The un-encoded string of data. Its format depends on the encoding method.
    ///
    /// NOTE: earlier releases of the OCPI 2.3.0 documentation mistakenly gave a maximum of 512.
    pub plain_data: OcpiString<5000>,
    /// Blob of signed data, base64 encoded.
    ///
    /// **Carried verbatim, whatever its length.** A signed record is evidence: it is worth
    /// nothing if a byte moves, and an OCMF blob from a real meter routinely runs past the
    /// `string(5000)` the specification gives. This crate's governing rule applies — the value
    /// arrives intact and `validate()` reports the length as a
    /// [`crate::types::ViolationCode::TooLong`] — so a decode and re-encode
    /// round trip reproduces the original bytes exactly. `tests/fixtures.rs` asserts it.
    pub signed_data: OcpiString<5000>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for SignedValue {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, nature, plain_data, signed_data);
    }
}

ocpi_lenient_enum! {
    /// How the driver was authenticated for a session.
    ///
    /// Spec: 2.3.0 §mod_cdrs_authmethod_enum
    pub enum AuthMethod {
        /// An authentication request was sent to the eMSP.
        AuthRequest = "AUTH_REQUEST",
        /// A command such as `StartSession` or `ReserveNow` started the session.
        Command = "COMMAND",
        /// A whitelist was used; no request to the eMSP was performed.
        Whitelist = "WHITELIST",
    }
}

ocpi_enum! {
    /// The quantities a [`ChargingPeriod`] can record.
    ///
    /// Some values are marked *Session Only* in the spec and must not appear in a CDR; see
    /// [`CdrDimensionType::is_session_only`].
    ///
    /// Spec: 2.3.0 §mod_cdrs_cdrdimensiontype_enum
    pub enum CdrDimensionType {
        /// Average charging current during this period, in A. Negative flows to the grid.
        Current = "CURRENT",
        /// Total energy (dis-)charged during this period, in kWh. Default `step_size` is 1.
        Energy = "ENERGY",
        /// Total energy fed back into the grid, in kWh.
        EnergyExport = "ENERGY_EXPORT",
        /// Total energy charged, in kWh.
        EnergyImport = "ENERGY_IMPORT",
        /// Sum of the maximum current over all phases reached during this period, in A.
        MaxCurrent = "MAX_CURRENT",
        /// Sum of the minimum current over all phases reached during this period, in A.
        MinCurrent = "MIN_CURRENT",
        /// Maximum power reached during this period, in kW.
        MaxPower = "MAX_POWER",
        /// Minimum power reached during this period, in kW.
        MinPower = "MIN_POWER",
        /// Time during which the vehicle is not requesting power, in hours.
        ///
        /// > *NOTE: Earlier versions of the OCPI 2.3.0 specification document mistakenly defined
        /// > PARKING_TIME as "Time during this ChargingPeriod not charging".*
        ParkingTime = "PARKING_TIME",
        /// Average power during this period, in kW. Negative flows to the grid.
        Power = "POWER",
        /// Time the EVSE has been reserved and not yet in use for this customer, in hours.
        ReservationTime = "RESERVATION_TIME",
        /// Time a reservation was held that then **expired**, in hours.
        ///
        /// From the 2.3.0 `bookings` branch, which core 2.3.0 does not have. Declared
        /// unconditionally rather than behind the feature because this enum is **closed**: a
        /// booking-aware CPO sending it would otherwise make the whole CDR undecodable.
        ///
        /// Spec: 2.3.0-bookings §mod_cdrs_cdrdimensiontype_enum
        ReservationExpires = "RESERVATION_EXPIRES",
        /// Time the session continued **after** the reserved slot ended, in hours.
        ///
        /// Also from the `bookings` branch, and unconditional for the same reason.
        ///
        /// Spec: 2.3.0-bookings §mod_cdrs_cdrdimensiontype_enum
        ReservationOvertime = "RESERVATION_OVERTIME",
        /// Current state of charge of the EV, in percent, 0 to 100.
        StateOfCharge = "STATE_OF_CHARGE",
        /// Time charging in this period, in hours.
        Time = "TIME",
    }
}

impl CdrDimensionType {
    /// Whether the spec marks this dimension *Session Only*.
    ///
    /// > *Some of these values are not useful for CDRs, and SHALL therefore only be used in
    /// > Sessions.*
    ///
    /// Spec: 2.3.0 §mod_cdrs_cdrdimensiontype_enum
    #[must_use]
    pub const fn is_session_only(self) -> bool {
        matches!(
            self,
            Self::Current | Self::EnergyExport | Self::EnergyImport | Self::Power | Self::StateOfCharge
        )
    }

    /// Whether a negative volume is meaningful for this dimension.
    ///
    /// The spec says so explicitly for the bidirectional quantities: *"When negative, the current
    /// is flowing from the EV to the grid."*
    #[must_use]
    pub const fn may_be_negative(self) -> bool {
        matches!(self, Self::Current | Self::Energy | Self::MinCurrent | Self::MinPower | Self::Power)
    }

    /// The unit the volume is measured in.
    #[must_use]
    pub const fn unit(self) -> &'static str {
        match self {
            Self::Current | Self::MaxCurrent | Self::MinCurrent => "A",
            Self::Energy | Self::EnergyExport | Self::EnergyImport => "kWh",
            Self::MaxPower | Self::MinPower | Self::Power => "kW",
            Self::ParkingTime
            | Self::ReservationTime
            | Self::ReservationExpires
            | Self::ReservationOvertime
            | Self::Time => "h",
            Self::StateOfCharge => "%",
        }
    }
}

#[cfg(test)]
mod cdr_helper_tests {
    use super::*;

    fn dt(s: &str) -> DateTime {
        s.parse().expect("a valid timestamp")
    }

    fn period(start: &str, kwh: &str) -> ChargingPeriod {
        ChargingPeriod::builder()
            .start_date_time(dt(start))
            .dimensions(vec![CdrDimension {
                dimension_type: CdrDimensionType::Energy,
                volume: kwh.parse().expect("a number"),
                extensions: Extensions::new(),
            }])
            .build()
    }

    /// Built here rather than from `testkit`, which is a feature these tests must not require.
    fn cdr_with(periods: Vec<ChargingPeriod>, end: &str, last_updated: &str) -> Cdr {
        use crate::types::CiString;
        let energy: Number = periods.iter().filter_map(|p| p.volume(CdrDimensionType::Energy)).sum();
        Cdr::builder()
            .country_code(CiString::new("NL").expect("valid"))
            .party_id(CiString::new("TNM").expect("valid"))
            .id(CiString::new("CDR1").expect("valid"))
            .start_date_time(dt("2024-01-15T10:00:00Z"))
            .end_date_time(dt(end))
            .session_id(CiString::new("SESS1").expect("valid"))
            .cdr_token(CdrToken {
                country_code: CiString::new("DE").expect("valid"),
                party_id: CiString::new("ABC").expect("valid"),
                uid: CiString::new("012345678").expect("valid"),
                token_type: TokenType::Rfid,
                contract_id: CiString::new("DE8AACA2B3C4D5N").expect("valid"),
                extensions: Extensions::new(),
            })
            .auth_method(AuthMethod::Whitelist)
            .cdr_location(cdr_location())
            .currency("EUR")
            .charging_periods(periods)
            .total_cost(crate::v2_3_0::types::Price::new("1.00".parse().expect("a number")))
            .total_energy(energy)
            .total_time("1".parse::<Number>().expect("a number"))
            .last_updated(dt(last_updated))
            .build()
    }

    fn cdr_location() -> CdrLocation {
        use crate::types::CiString;
        CdrLocation::builder()
            .id(CiString::new("LOC1").expect("valid"))
            .address("F.Rooseveltlaan 3A")
            .city("Gent")
            .country("BEL")
            .coordinates(
                crate::v2_3_0::locations::GeoLocation::new("3.729944", "51.047599")
                    .expect("valid coordinates"),
            )
            .evse_uid(CiString::new("3256").expect("valid"))
            .evse_id(CiString::new("BE*BEC*E041503001").expect("valid"))
            .connector_id(CiString::new("1").expect("valid"))
            .connector_standard(crate::v2_3_0::locations::ConnectorType::Iec62196T2)
            .connector_format(crate::v2_3_0::locations::ConnectorFormat::Socket)
            .connector_power_type(crate::v2_3_0::locations::PowerType::Ac3Phase)
            .build()
    }

    /// A period ends where the next one starts, and the last one at the CDR's own end.
    #[test]
    fn a_period_span_runs_to_the_next_period_and_the_last_to_the_cdrs_end() {
        let cdr = cdr_with(
            vec![period("2024-01-15T10:00:00Z", "4.3"), period("2024-01-15T10:30:00Z", "1.1")],
            "2024-01-15T11:00:00Z",
            "2024-01-15T11:05:00Z",
        );
        let spans: Vec<_> = cdr.period_spans().collect();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].end, dt("2024-01-15T10:30:00Z"), "the next period's start");
        assert_eq!(spans[1].end, dt("2024-01-15T11:00:00Z"), "the CDR's end");
        assert_eq!(spans[0].duration_seconds(), 1800);
        assert_eq!(spans[1].duration_seconds(), 1800);
        assert_eq!(spans[0].volume(CdrDimensionType::Energy).map(|v| v.to_string()), Some("4.3".into()));
        assert!(spans[0].volume(CdrDimensionType::ParkingTime).is_none());

        // The spans partition the session: they meet end-to-start and cover it exactly.
        assert_eq!(spans[0].start, cdr.start_date_time);
        assert_eq!(spans[0].end, spans[1].start);
        assert_eq!(spans.last().expect("a span").end, cdr.end_date_time);
    }

    #[test]
    fn a_single_period_spans_the_whole_session() {
        let cdr = cdr_with(
            vec![period("2024-01-15T10:00:00Z", "5.4")],
            "2024-01-15T11:00:00Z",
            "2024-01-15T11:00:00Z",
        );
        let spans: Vec<_> = cdr.period_spans().collect();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].duration_seconds(), 3600);
    }

    /// The latency a consumer with a filing deadline measures — and the one case that would
    /// otherwise report half a century.
    #[test]
    fn delivery_latency_is_measured_from_last_updated_and_skips_placeholder_timestamps() {
        let cdr = cdr_with(
            vec![period("2024-01-15T10:00:00Z", "1")],
            "2024-01-15T11:00:00Z",
            "2024-01-17T09:00:00Z",
        );
        assert_eq!(cdr.delivery_latency_seconds(), Some(2 * 86_400 - 2 * 3600));

        // "the CPO could send a CDR where the start_date_time and/or end_date_time are set to
        //  1970-1-1T00:00:00Z" — a latency of 54 years is not a measurement.
        let mut placeholder = cdr.clone();
        placeholder.start_date_time = dt("1970-01-01T00:00:00Z");
        placeholder.end_date_time = dt("1970-01-01T00:00:00Z");
        assert!(placeholder.has_placeholder_timestamps());
        assert_eq!(placeholder.delivery_latency_seconds(), None);

        // A CPO whose clock disagrees with its own session is shown, not clamped.
        let mut skewed = cdr;
        skewed.last_updated = dt("2024-01-15T10:59:00Z");
        assert_eq!(skewed.delivery_latency_seconds(), Some(-60));
    }

    /// The signed record is evidence: it survives a round trip byte for byte, over-length or not.
    #[test]
    fn an_over_length_signed_blob_survives_a_round_trip_exactly() {
        // Real OCMF payloads run past the `string(5000)` the specification gives.
        let blob = "O".repeat(6000);
        let json = format!(r#"{{"nature":"End","plain_data":"{blob}","signed_data":"{blob}"}}"#);
        let value: SignedValue = serde_json::from_str(&json).expect("decodes");
        assert_eq!(value.signed_data.as_str(), blob, "not a byte moved");
        assert_eq!(serde_json::to_string(&value).expect("encodes"), json, "and it goes back out the same");
        assert_eq!(
            value.validate().expect_err("the length is still reported").as_slice()[0].code,
            crate::types::ViolationCode::TooLong,
        );
    }

    /// `SignedData.url` is a `string(512)`, not the `string(255)` `URL` type.
    ///
    /// Modelled as a `Url` it reported a conformant link as `TooLong`, and — because
    /// `ClientConfig::validate_outgoing` is on by default — a client could not send the CDR
    /// carrying it.
    #[test]
    fn a_signed_data_url_may_run_past_the_length_of_an_ocpi_url() {
        use crate::types::Validate;
        let long = format!("https://e.com/{}", "a".repeat(300));
        assert!(long.len() > 255 && long.len() <= 512);
        let json = format!(
            r#"{{"encoding_method":"OCMF","signed_values":[{{"nature":"End","plain_data":"p","signed_data":"s"}}],"url":"{long}"}}"#
        );
        let data: SignedData = serde_json::from_str(&json).expect("decodes");
        assert_eq!(data.url.as_ref().expect("present").as_str(), long);
        data.validate().expect("a 314-character signed-data URL is conformant");
    }

    #[test]
    fn signed_values_are_reachable_by_nature() {
        let value = |nature: &str| SignedValue {
            nature: crate::types::CiString::new(nature).expect("valid"),
            plain_data: crate::types::OcpiString::new_lenient("plain"),
            signed_data: crate::types::OcpiString::new_lenient("signed"),
            extensions: Extensions::new(),
        };
        let data = SignedData::builder()
            .encoding_method(crate::types::CiString::<36>::new("OCMF").expect("valid"))
            .signed_values(vec![value("Start"), value("End")])
            .build();
        assert!(data.start_value().is_some());
        assert!(data.end_value().is_some());
        // "Others might be added later", and the nature is a CiString.
        assert!(data.value_for("end").is_some(), "natures compare case-insensitively");
        assert!(data.value_for("Intermediate").is_none());
    }
}

#[cfg(test)]
mod dimension_tests {
    use super::*;

    /// The `bookings` branch's two reservation dimensions, on a **closed** enum: a missing value
    /// here makes the whole CDR undecodable rather than degrading. No fixture uses them, so only
    /// `xtask enum-coverage` sees the gap.
    #[test]
    fn the_bookings_branch_reservation_dimensions_decode() {
        for (wire, expected, unit) in [
            ("RESERVATION_TIME", CdrDimensionType::ReservationTime, "h"),
            ("RESERVATION_EXPIRES", CdrDimensionType::ReservationExpires, "h"),
            ("RESERVATION_OVERTIME", CdrDimensionType::ReservationOvertime, "h"),
        ] {
            let decoded: CdrDimensionType =
                serde_json::from_str(&format!("\"{wire}\"")).unwrap_or_else(|e| panic!("{wire}: {e}"));
            assert_eq!(decoded, expected);
            assert_eq!(serde_json::to_string(&decoded).expect("serialises"), format!("\"{wire}\""));
            assert_eq!(decoded.unit(), unit);
            assert!(!decoded.is_session_only(), "{wire} has no Session-Only mark in the branch table");
        }
    }
}

#[cfg(test)]
mod period_sequence_tests {
    use super::*;
    use crate::types::Violation;

    fn dt(s: &str) -> DateTime {
        s.parse().expect("a valid timestamp")
    }

    fn check(starts: &[&str], start: &str, end: Option<&str>) -> Vec<Violation> {
        let mut v = Validator::new();
        validate_period_sequence(
            &starts.iter().map(|s| dt(s)).collect::<Vec<_>>(),
            dt(start),
            end.map(dt),
            &mut v,
        );
        v.finish().into_vec()
    }

    #[test]
    fn a_well_formed_sequence_is_accepted() {
        assert!(
            check(
                &["2024-01-15T10:00:00Z", "2024-01-15T10:30:00Z", "2024-01-15T11:00:00Z"],
                "2024-01-15T10:00:00Z",
                Some("2024-01-15T11:30:00Z"),
            )
            .is_empty()
        );
    }

    #[test]
    fn periods_out_of_order_are_reported_at_the_offending_index() {
        // The failure a merge of two period streams produces: everything is individually valid.
        let found = check(
            &["2024-01-15T10:00:00Z", "2024-01-15T11:00:00Z", "2024-01-15T10:30:00Z"],
            "2024-01-15T10:00:00Z",
            Some("2024-01-15T12:00:00Z"),
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].pointer, "/charging_periods/2/start_date_time");
        assert_eq!(found[0].code, ViolationCode::Inconsistent);
    }

    #[test]
    fn two_periods_at_the_same_instant_are_reported() {
        // Not merely unordered: a zero-length period has no duration to price.
        let found = check(&["2024-01-15T10:00:00Z", "2024-01-15T10:00:00Z"], "2024-01-15T10:00:00Z", None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].pointer, "/charging_periods/1/start_date_time");
    }

    #[test]
    fn a_period_outside_the_session_is_reported() {
        let before = check(&["2024-01-15T09:00:00Z"], "2024-01-15T10:00:00Z", None);
        assert_eq!(before.len(), 1);
        assert!(before[0].message.contains("before the session started"), "{:?}", before[0]);

        let after = check(&["2024-01-15T13:00:00Z"], "2024-01-15T10:00:00Z", Some("2024-01-15T12:00:00Z"));
        assert_eq!(after.len(), 1);
        assert!(after[0].message.contains("after the session ended"), "{:?}", after[0]);
    }

    #[test]
    fn an_empty_or_single_period_list_has_nothing_to_disagree_with() {
        assert!(check(&[], "2024-01-15T10:00:00Z", None).is_empty());
        assert!(check(&["2024-01-15T10:00:00Z"], "2024-01-15T10:00:00Z", None).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim(t: CdrDimensionType, v: &str) -> CdrDimension {
        CdrDimension::new(t, v.parse().unwrap())
    }

    #[test]
    fn session_only_dimensions_are_rejected_in_a_cdr() {
        let p = ChargingPeriod::builder()
            .start_date_time("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .dimensions(vec![dim(CdrDimensionType::StateOfCharge, "50")])
            .build();
        assert!(p.validate().is_ok(), "a Session may carry STATE_OF_CHARGE");
        assert!(CdrDimensionType::StateOfCharge.is_session_only());
        assert!(!CdrDimensionType::Energy.is_session_only());
    }

    #[test]
    fn dimension_units_and_signs_follow_the_table() {
        assert_eq!(CdrDimensionType::Energy.unit(), "kWh");
        assert_eq!(CdrDimensionType::ParkingTime.unit(), "h");
        assert!(CdrDimensionType::Power.may_be_negative(), "V2G power flows both ways");
        assert!(!CdrDimensionType::ParkingTime.may_be_negative());
        assert!(dim(CdrDimensionType::ParkingTime, "-1").validate().is_err());
        assert!(dim(CdrDimensionType::Power, "-7.5").validate().is_ok());
        assert!(dim(CdrDimensionType::StateOfCharge, "101").validate().is_err());
    }

    #[test]
    fn a_period_cannot_measure_the_same_dimension_twice() {
        let p = ChargingPeriod::builder()
            .start_date_time("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .dimensions(vec![dim(CdrDimensionType::Energy, "1"), dim(CdrDimensionType::Energy, "2")])
            .build();
        assert_eq!(p.validate().unwrap_err().as_slice()[0].code, ViolationCode::Inconsistent);
    }

    #[test]
    fn empty_dimensions_are_a_cardinality_violation() {
        let p = ChargingPeriod::builder()
            .start_date_time("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .dimensions(vec![])
            .build();
        assert_eq!(p.validate().unwrap_err().as_slice()[0].code, ViolationCode::EmptyRequiredList);
    }
}
