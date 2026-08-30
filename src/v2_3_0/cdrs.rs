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

use crate::ocpi_enum;
use crate::types::validate_fields;
use crate::types::{
    CiString, ContractId, CountryCode, Currency, DateTime, EvseId, Extensions, Number, OcpiString, PartyId,
    PartyRef, Url, Validate, Validator, ViolationCode,
};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<Url>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
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

ocpi_enum! {
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
            Self::ParkingTime | Self::ReservationTime | Self::Time => "h",
            Self::StateOfCharge => "%",
        }
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
