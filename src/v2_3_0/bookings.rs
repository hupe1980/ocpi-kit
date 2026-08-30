//! The *Bookings* module, from the OCPI 2.3.0 `bookings` release branch.
//!
//! *Module Identifier: `Booking`* — Data owner: CPO.
//!
//! **Spec quirks.** Two, both worth knowing before an integration:
//!
//! * The identifier really is `Booking` — singular, and the only module ID in OCPI that is not
//!   lower case. It is also missing from that branch's `ModuleID` table.
//!   [`ModuleId::matches`](crate::ModuleId::matches) accepts the lower-case plural too, because
//!   implementations that guessed it exist and failing to discover the module is worse.
//! * `Cancellation.who_canceled` is typed as `Role` in the property table but links to the
//!   *`InterfaceRole`* anchor. The description — *"Who canceled the booking"*, with the enum's
//!   own values naming the CPO and the MSP — only makes sense as a party role, so
//!   [`Cancellation::who_canceled`] is a [`Role`].
//!
//! # Lifecycle
//!
//! > *A Booking starts in a `PENDING` state when initially requested by the eMSP. … From
//! > `RESERVED`, the Booking can transition to `FULFILLED`, `CANCELED` or `NO_SHOW`.*
//!
//! [`ReservationStatus::can_transition_to`] encodes the whole state machine.
//!
//! Spec: 2.3.0-bookings §mod_bookings_bookings_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_enum;
use crate::types::validate_fields;
use crate::types::{
    CiString, ContractId, CountryCode, DateTime, Extensions, Number, OcpiText, PartyId, PartyRef, Url,
    Validate, Validator, ViolationCode,
};

use super::locations::{ConnectorFormat, ConnectorType, EvsePosition, PowerType, VehicleType};
use super::tokens::TokenType;
use super::types::Role;

/// A booking of a charging slot at a Location.
///
/// Spec: 2.3.0-bookings §mod_bookings_booking_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Booking {
    /// ID for the CPO side.
    pub id: CiString<36>,
    /// ISO-3166 alpha-2 country code of the CPO that 'owns' this Booking.
    pub country_code: CountryCode,
    /// ID of the CPO that 'owns' this Booking.
    pub party_id: PartyId,
    /// Request ID determined by the requesting party.
    ///
    /// > *The same request ID SHALL be used for all edits on booking.*
    pub request_id: CiString<36>,
    /// The specification selected for charging at this Location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub booking_option: Option<BookingOption>,
    /// `Location.id` on which the reservation was made.
    pub location_id: CiString<36>,
    /// Tokens that can be used to take up the booking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub booking_tokens: Vec<BookingToken>,
    /// Tariffs relevant for this booking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tariff_ids: Vec<CiString<36>>,
    /// The timeslot booked.
    pub period: Timeslot,
    /// The current state of the reservation.
    pub reservation_status: ReservationStatus,
    /// Why the booking was canceled, and by whom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canceled: Option<Cancellation>,
    /// How to get to the Location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub access_information: Vec<AccessInformation>,
    /// Authorization reference for the resulting Session and CDR.
    pub authorization_reference: CiString<36>,
    /// The booking terms that were accepted.
    pub booking_terms: BookingTerms,
    /// Every request made for this booking. Cardinality `+`.
    pub booking_requests: Vec<BookingRequestStatus>,
    /// When this Booking was last changed.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Booking {
    /// The CPO that owns this Booking.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }

    /// Whether the booking has reached a state it will not leave.
    #[must_use]
    pub fn is_final(&self) -> bool {
        self.reservation_status.is_terminal()
    }
}

impl Validate for Booking {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            id,
            country_code,
            party_id,
            request_id,
            booking_option,
            location_id,
            booking_tokens,
            tariff_ids,
            period,
            reservation_status,
            canceled,
            access_information,
            authorization_reference,
            booking_terms,
            booking_requests,
            last_updated,
        );
        if self.booking_requests.is_empty() {
            v.report_at(
                "booking_requests",
                ViolationCode::EmptyRequiredList,
                "a Booking has cardinality `+` booking_requests: the request that created it is \
                 always one of them",
            );
        }
        // "canceled: Is the booking canceled, why and by whom."
        match (self.reservation_status, self.canceled.is_some()) {
            (ReservationStatus::Canceled, false) => v.report_at(
                "canceled",
                ViolationCode::MissingConditional,
                "a CANCELED booking should say why and by whom",
            ),
            (status, true) if status != ReservationStatus::Canceled => v.report_at(
                "reservation_status",
                ViolationCode::Inconsistent,
                format!("a cancellation is recorded, but the status is {status}"),
            ),
            _ => {}
        }
    }
}

/// A Location that can be booked, with its calendars and terms.
///
/// > *Each bookingLocation should include either the `booking_option` or the `evse_uid`. One of
/// > them is mandatory.*
///
/// Spec: 2.3.0-bookings §mod_bookings_bookinglocation_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct BookingLocation {
    /// ISO-3166 alpha-2 country code of the CPO that 'owns' this BookingLocation.
    pub country_code: CountryCode,
    /// ID of the CPO that 'owns' this BookingLocation.
    pub party_id: PartyId,
    /// The unique id that identifies this BookingLocation in the CPO platform.
    pub id: CiString<36>,
    /// `Location.id` on which the reservation can be made.
    pub location_id: CiString<36>,
    /// What drivers can book at this Location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub booking_option: Option<BookingOption>,
    /// How many charging stations are bookable here, and whether booking is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
    /// Tariffs relevant here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tariff_ids: Vec<CiString<36>>,
    /// The terms that apply to a booking here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub booking_terms: Option<BookingTerms>,
    /// The calendars showing availability.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub calendars: Vec<Calendar>,
    /// When this BookingLocation was last changed.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl BookingLocation {
    /// The CPO that owns this BookingLocation.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }
}

impl Validate for BookingLocation {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            country_code,
            party_id,
            id,
            location_id,
            booking_option,
            policy,
            tariff_ids,
            booking_terms,
            calendars,
            last_updated,
        );
        // "Each bookingLocation should include either the booking_option or the evse_uid.
        //  One of them is mandatory."
        let names_an_evse = self.booking_option.as_ref().is_some_and(|o| o.evse_uid.is_some());
        if self.booking_option.is_none() && !names_an_evse {
            v.report_at(
                "booking_option",
                ViolationCode::MissingConditional,
                "either `booking_option` or an EVSE must be given; one of them is mandatory",
            );
        }
    }
}

/// The availability of a BookingLocation over a period.
///
/// Spec: 2.3.0-bookings §mod_bookings_calendar_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Calendar {
    /// ID of this calendar.
    pub id: CiString<36>,
    /// Start of the calendar.
    pub begin_from: DateTime,
    /// End of the calendar.
    pub end_before: DateTime,
    /// The smallest booking increment within an available timeslot, in minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeslot_increment: Option<u32>,
    /// The available timeslots. Cardinality `+`.
    pub available_timeslots: Vec<Timeslot>,
    /// When this calendar was last changed.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Calendar {
    /// Whether `slot` fits inside one of the available timeslots.
    #[must_use]
    pub fn can_accommodate(&self, slot: &Timeslot) -> bool {
        self.available_timeslots.iter().any(|available| {
            slot.start_date_time >= available.start_date_time && slot.end_date_time <= available.end_date_time
        })
    }
}

impl Validate for Calendar {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, id, begin_from, end_before, available_timeslots, last_updated);
        if self.end_before <= self.begin_from {
            v.report_at(
                "end_before",
                ViolationCode::Inconsistent,
                "a calendar must cover a non-empty period",
            );
        }
        if self.available_timeslots.is_empty() {
            v.report_at(
                "available_timeslots",
                ViolationCode::EmptyRequiredList,
                "a Calendar has cardinality `+` available_timeslots",
            );
        }
    }
}

/// A window of time, with the power available in it.
///
/// Spec: 2.3.0-bookings §mod_bookings_timeslot_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Timeslot {
    /// Start of this timeslot.
    pub start_date_time: DateTime,
    /// End of this timeslot.
    pub end_date_time: DateTime,
    /// Minimum power guaranteed during this timeslot, in watts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_power: Option<Number>,
    /// Maximum power available during this timeslot, in watts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_power: Option<Number>,
    /// Whether green energy is available during this timeslot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub green_energy_support: Option<bool>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Timeslot {
    /// The length of this timeslot in minutes, or `None` if it is not a forward interval.
    #[must_use]
    pub fn duration_minutes(&self) -> Option<i64> {
        let seconds = self.end_date_time.unix_timestamp() - self.start_date_time.unix_timestamp();
        (seconds > 0).then_some(seconds / 60)
    }
}

impl Validate for Timeslot {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, start_date_time, end_date_time, min_power, max_power);
        if self.end_date_time <= self.start_date_time {
            v.report_at(
                "end_date_time",
                ViolationCode::Inconsistent,
                "a timeslot must cover a non-empty period",
            );
        }
        if let (Some(min), Some(max)) = (self.min_power, self.max_power)
            && max < min
        {
            v.report_at(
                "max_power",
                ViolationCode::Inconsistent,
                "the maximum power cannot be below the guaranteed minimum",
            );
        }
    }
}

/// What a driver can book at a Location.
///
/// Spec: 2.3.0-bookings §mod_bookings_booking_option_class
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct BookingOption {
    /// A bookable `EVSE.uid`. May be `#NA` when no EVSE is assigned yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_uid: Option<CiString<36>>,
    /// `Connector.id` where the booking will happen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<CiString<36>>,
    /// Reference to a `Parking.id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parking_id: Option<CiString<36>>,
    /// The position of the EVSE relative to the parking space.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub evse_position: Vec<EvsePosition>,
    /// The vehicle types the parking accommodates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub vehicle_types: Vec<VehicleType>,
    /// The connector formats available.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub connector_format: Vec<ConnectorFormat>,
    /// The connector types available.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub connector_types: Vec<ConnectorType>,
    /// The power types available.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub power_types: Vec<PowerType>,
    /// Maximum vehicle weight, in kilograms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vehicle_weight: Option<Number>,
    /// Maximum vehicle height, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vehicle_height: Option<Number>,
    /// Maximum vehicle length, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vehicle_length: Option<Number>,
    /// Maximum vehicle width, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vehicle_width: Option<Number>,
    /// Minimum length of the parking space, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_parking_space_length: Option<Number>,
    /// Minimum width of the parking space, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_parking_space_width: Option<Number>,
    /// Whether vehicles loaded with dangerous substances may park.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dangerous_goods_allowed: Option<bool>,
    /// Whether a vehicle can charge without reversing into or out of the space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_through: Option<bool>,
    /// Whether a refrigeration outlet is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refrigeration_outlet: Option<bool>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for BookingOption {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            evse_uid,
            connector_id,
            parking_id,
            evse_position,
            vehicle_types,
            connector_format,
            connector_types,
            power_types,
            max_vehicle_weight,
            max_vehicle_height,
            max_vehicle_length,
            max_vehicle_width,
            min_parking_space_length,
            min_parking_space_width,
        );
    }
}

/// One request made against a booking, and what became of it.
///
/// Spec: 2.3.0-bookings §mod_bookings_request_status_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct BookingRequestStatus {
    /// The current state of the request.
    pub request_status: ReservationRequestStatus,
    /// The request that was received.
    pub booking_request: BookingRequest,
    /// When it was received.
    pub request_received: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for BookingRequestStatus {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, request_status, booking_request, request_received);
    }
}

/// A request from an eMSP to make or change a booking.
///
/// Spec: 2.3.0-bookings §mod_bookings_request_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct BookingRequest {
    /// ISO-3166 alpha-2 country code of the MSP requesting the booking.
    pub country_code: CountryCode,
    /// ID of the MSP requesting the booking.
    pub party_id: PartyId,
    /// Request ID determined by the requesting party.
    pub request_id: CiString<36>,
    /// The specification selected for charging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub booking_option: Option<BookingOption>,
    /// `Location.id` on which the reservation is made.
    pub location_id: CiString<36>,
    /// The `BookingLocation.id` being booked.
    pub booking_location_id: CiString<36>,
    /// Tokens that can be used to take up the booking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tokens: Vec<BookingToken>,
    /// How to get to the Location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub access_information: Vec<AccessInformation>,
    /// The period requested.
    pub period: Period,
    /// Authorization reference for the resulting Session and CDR.
    pub authorization_reference: CiString<36>,
    /// The power requested, in kW.
    ///
    /// > *If it isn't the maximum available the CPO can relocate the extra to another session.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_required: Option<u32>,
    /// Set when the request is to cancel the booking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canceled: Option<Cancellation>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl BookingRequest {
    /// The eMSP that made this request.
    #[must_use]
    pub fn requester(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }
}

impl Validate for BookingRequest {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            country_code,
            party_id,
            request_id,
            booking_option,
            location_id,
            booking_location_id,
            tokens,
            access_information,
            period,
            authorization_reference,
            canceled,
        );
    }
}

/// A window of time. Unlike [`Timeslot`], it carries no power information.
///
/// Spec: 2.3.0-bookings §mod_bookings_period_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Period {
    /// Start of this period.
    pub start_date_time: DateTime,
    /// End of this period.
    pub end_date_time: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for Period {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, start_date_time, end_date_time);
        if self.end_date_time <= self.start_date_time {
            v.report_at(
                "end_date_time",
                ViolationCode::Inconsistent,
                "a period must cover a non-empty span of time",
            );
        }
    }
}

/// A Token that can take up a booking.
///
/// Spec: 2.3.0-bookings §mod_bookings_booking_token_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct BookingToken {
    /// ISO-3166 alpha-2 country code of the MSP that 'owns' this Token.
    pub country_code: CountryCode,
    /// ID of the eMSP that 'owns' this Token.
    pub party_id: PartyId,
    /// Unique ID by which this Token can be identified.
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

impl Validate for BookingToken {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, country_code, party_id, uid, token_type as "type", contract_id);
    }
}

/// The terms a booking is made under.
///
/// Spec: 2.3.0-bookings §mod_bookings_booking_terms_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct BookingTerms {
    /// Whether charging for a reserved booking requires an RFID card at the charger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rfid_auth_required: Option<bool>,
    /// Whether any token in the same token group may be used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_groups_supported: Option<bool>,
    /// Whether charging can be started remotely, through the Commands module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_auth_supported: Option<bool>,
    /// What is needed to access the Location. Cardinality `+`.
    pub supported_access_methods: Vec<AccessMethod>,
    /// Minutes before the booking until which it can be changed.
    pub change_until_minutes: Number,
    /// Minutes before the booking until which it can be canceled.
    pub cancel_until_minutes: Number,
    /// Whether changing the booking is disallowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_not_allowed: Option<bool>,
    /// Whether starting the session early is possible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub early_start_allowed: Option<bool>,
    /// How many minutes early a session may start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub early_start_time: Option<Number>,
    /// Minutes after the booking start after which it counts as a no-show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noshow_timeout: Option<Number>,
    /// Whether the CPO charges a no-show fee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noshow_fee: Option<bool>,
    /// Whether a driver may charge for longer than booked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub late_stop_allowed: Option<bool>,
    /// How many minutes a session may run past the end of the booking.
    ///
    /// The specification's description reads *"Number of minutes late start is allowed"*, which
    /// is a copy-paste of `early_start_time`; the field name and its position after
    /// `late_stop_allowed` make the intent clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub late_stop_time: Option<Number>,
    /// Whether the same RFID token may be attached to several overlapping bookings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlapping_bookings_allowed: Option<bool>,
    /// Minimum booking duration in minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_booking_duration: Option<Number>,
    /// Maximum booking duration in minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_booking_duration: Option<Number>,
    /// The CPO's URL to the booking terms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub booking_terms: Option<Url>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl BookingTerms {
    /// Whether a booking starting at `start` may still be canceled at `now`.
    #[must_use]
    pub fn may_cancel_at(&self, now: DateTime, start: DateTime) -> bool {
        minutes_before(now, start) >= self.cancel_until_minutes
    }

    /// Whether a booking starting at `start` may still be changed at `now`.
    #[must_use]
    pub fn may_change_at(&self, now: DateTime, start: DateTime) -> bool {
        if self.change_not_allowed.unwrap_or(false) {
            return false;
        }
        minutes_before(now, start) >= self.change_until_minutes
    }
}

fn minutes_before(now: DateTime, start: DateTime) -> Number {
    let seconds = start.unix_timestamp() - now.unix_timestamp();
    Number::from(seconds) / Number::from(60u32)
}

impl Validate for BookingTerms {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            supported_access_methods,
            change_until_minutes,
            cancel_until_minutes,
            early_start_time,
            noshow_timeout,
            late_stop_time,
            min_booking_duration,
            max_booking_duration,
            booking_terms,
        );
        if self.supported_access_methods.is_empty() {
            v.report_at(
                "supported_access_methods",
                ViolationCode::EmptyRequiredList,
                "BookingTerms has cardinality `+` supported_access_methods: a driver needs to \
                 know how to get in",
            );
        }
        if let (Some(min), Some(max)) = (self.min_booking_duration, self.max_booking_duration)
            && max < min
        {
            v.report_at(
                "max_booking_duration",
                ViolationCode::Inconsistent,
                "the maximum booking duration cannot be below the minimum",
            );
        }
    }
}

/// How to get to a booked charger.
///
/// Spec: 2.3.0-bookings §mod_bookings_access_information_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AccessInformation {
    /// How the Location is accessed.
    pub method: AccessMethod,
    /// The value for the method: a licence plate, an access code, and so on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<OcpiText>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for AccessInformation {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, method, value);
        // OPEN and INTERCOM need nothing; the rest are useless without their value.
        if self.value.is_none()
            && matches!(
                self.method,
                AccessMethod::Token | AccessMethod::LicensePlate | AccessMethod::AccessCode
            )
        {
            v.report_at(
                "value",
                ViolationCode::MissingConditional,
                format!("{} needs the value the driver is to present", self.method),
            );
        }
    }
}

/// Why a booking was canceled, and by whom.
///
/// Spec: 2.3.0-bookings §mod_bookings_cancellation_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Cancellation {
    /// The reason.
    pub cancellation_reason: CanceledReason,
    /// Who canceled.
    ///
    /// **Spec erratum.** The property table types this as `Role` but links to the
    /// `InterfaceRole` anchor. `SENDER`/`RECEIVER` would not answer "who canceled the booking",
    /// and the reasons themselves are split into CPO-set and MSP-set, so this is a
    /// [`Role`].
    pub who_canceled: Role,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for Cancellation {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, cancellation_reason, who_canceled);
    }
}

/// How many bookable stations a Location has, and whether booking is required.
///
/// Spec: 2.3.0-bookings §mod_bookings_policy_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Policy {
    /// Whether a reservation is required to charge here.
    pub reservation_required: bool,
    /// How many ad-hoc charging options are available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ad_hoc: Option<Number>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for Policy {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, ad_hoc);
        if self.reservation_required && self.ad_hoc.is_some_and(|n| !n.is_zero()) {
            v.report_at(
                "ad_hoc",
                ViolationCode::Inconsistent,
                "a Location that requires a reservation cannot offer ad-hoc charging",
            );
        }
    }
}

ocpi_enum! {
    /// How a driver gets access to a reserved charger.
    ///
    /// Spec: 2.3.0-bookings §mod_bookings_access_method_enum
    pub enum AccessMethod {
        /// Open access to the site.
        Open = "OPEN",
        /// Using a token that was sent in the booking.
        Token = "TOKEN",
        /// The licence plate of the vehicle that wants to charge.
        LicensePlate = "LICENSE_PLATE",
        /// The access code provided.
        AccessCode = "ACCESS_CODE",
        /// Ring the intercom.
        Intercom = "INTERCOM",
        /// A parking ticket is required.
        ParkingTicket = "PARKING_TICKET",
    }
}

ocpi_enum! {
    /// Why a booking was canceled.
    ///
    /// Spec: 2.3.0-bookings §mod_bookings_canceled_reason_enum
    pub enum CanceledReason {
        /// No power available at the site. Set by the CPO.
        PowerOutage = "POWER_OUTAGE",
        /// The charger is broken. Set by the CPO.
        BrokenCharger = "BROKEN_CHARGER",
        /// The chargers are full because someone is not leaving. Set by the CPO.
        Full = "FULL",
        /// The reserved charger is not physically reachable.
        Blocked = "BLOCKED",
        /// The vehicle cannot arrive in time because of traffic. Set by the MSP.
        Traffic = "TRAFFIC",
        /// The vehicle broke down. Set by the MSP.
        BrokenVehicle = "BROKEN_VEHICLE",
        /// The driver gave no reason. Set by the MSP.
        NoCanceled = "NO_CANCELED",
        /// Any other or unknown reason.
        Unknown = "UNKNOWN",
    }
}

ocpi_enum! {
    /// The state of one booking request.
    ///
    /// Spec: 2.3.0-bookings §mod_bookings_request_status_enum
    pub enum ReservationRequestStatus {
        /// Pending processing by the CPO.
        Pending = "PENDING",
        /// Accepted by the CPO.
        Accepted = "ACCEPTED",
        /// Declined by the CPO.
        Declined = "DECLINED",
        /// The request failed with an error.
        Failed = "FAILED",
    }
}

ocpi_enum! {
    /// The state of a booking.
    ///
    /// Spec: 2.3.0-bookings §mod_bookings_reservation_status_enum
    pub enum ReservationStatus {
        /// Pending processing by the CPO. The initial state.
        Pending = "PENDING",
        /// Accepted by the CPO.
        Reserved = "RESERVED",
        /// Canceled.
        Canceled = "CANCELED",
        /// The request failed with an error.
        Failed = "FAILED",
        /// Nobody showed up within the no-show window.
        NoShow = "NO_SHOW",
        /// A session was started with the communicated token before the booking expired.
        Fulfilled = "FULFILLED",
        /// Rejected after processing, e.g. because the requested slot was unavailable.
        Rejected = "REJECTED",
        /// Any other or unknown state.
        Unknown = "UNKNOWN",
    }
}

impl ReservationStatus {
    /// Whether the booking can still change.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Canceled | Self::Failed | Self::NoShow | Self::Fulfilled | Self::Rejected)
    }

    /// Whether `next` is a state this booking may move to.
    ///
    /// > *A Booking starts in a `PENDING` state when initially requested by the eMSP. … From
    /// > `RESERVED`, the Booking can transition to `FULFILLED`, `CANCELED` or `NO_SHOW`.*
    ///
    /// Spec: 2.3.0-bookings §mod_bookings_bookings_module
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Pending => matches!(
                next,
                Self::Reserved | Self::Rejected | Self::Failed | Self::Canceled | Self::Unknown
            ),
            Self::Reserved => {
                matches!(next, Self::Fulfilled | Self::Canceled | Self::NoShow | Self::Unknown)
            }
            Self::Unknown => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> DateTime {
        s.parse().unwrap()
    }

    #[test]
    fn the_lifecycle_is_the_one_the_spec_describes() {
        use ReservationStatus::{Canceled, Fulfilled, NoShow, Pending, Rejected, Reserved};
        assert!(Pending.can_transition_to(Reserved));
        assert!(Pending.can_transition_to(Rejected));
        assert!(Reserved.can_transition_to(Fulfilled));
        assert!(Reserved.can_transition_to(Canceled));
        assert!(Reserved.can_transition_to(NoShow));
        // A booking that was never reserved cannot be fulfilled.
        assert!(!Pending.can_transition_to(Fulfilled));
        // Terminal states are terminal.
        assert!(!Fulfilled.can_transition_to(Canceled));
        assert!(Fulfilled.is_terminal() && NoShow.is_terminal());
        assert!(!Pending.is_terminal() && !Reserved.is_terminal());
    }

    #[test]
    fn a_timeslot_must_be_a_forward_interval_with_coherent_power() {
        let slot = Timeslot::builder()
            .start_date_time(dt("2024-06-01T10:00:00Z"))
            .end_date_time(dt("2024-06-01T12:00:00Z"))
            .min_power(Number::from(11_000u32))
            .max_power(Number::from(22_000u32))
            .build();
        assert!(slot.validate().is_ok());
        assert_eq!(slot.duration_minutes(), Some(120));

        let backwards = Timeslot { end_date_time: dt("2024-06-01T09:00:00Z"), ..slot.clone() };
        assert!(backwards.validate().is_err());
        assert_eq!(backwards.duration_minutes(), None);

        let impossible = Timeslot { max_power: Some(Number::from(1000u32)), ..slot };
        assert!(impossible.validate().is_err());
    }

    #[test]
    fn a_calendar_accommodates_a_slot_that_fits_inside_an_available_one() {
        let calendar = Calendar::builder()
            .id("CAL1")
            .begin_from(dt("2024-06-01T00:00:00Z"))
            .end_before(dt("2024-06-02T00:00:00Z"))
            .available_timeslots(vec![
                Timeslot::builder()
                    .start_date_time(dt("2024-06-01T08:00:00Z"))
                    .end_date_time(dt("2024-06-01T18:00:00Z"))
                    .build(),
            ])
            .last_updated(dt("2024-05-01T00:00:00Z"))
            .build();
        assert!(calendar.validate().is_ok());

        let fits = Timeslot::builder()
            .start_date_time(dt("2024-06-01T10:00:00Z"))
            .end_date_time(dt("2024-06-01T12:00:00Z"))
            .build();
        assert!(calendar.can_accommodate(&fits));

        let overruns = Timeslot::builder()
            .start_date_time(dt("2024-06-01T17:00:00Z"))
            .end_date_time(dt("2024-06-01T19:00:00Z"))
            .build();
        assert!(!calendar.can_accommodate(&overruns));
    }

    #[test]
    fn the_change_and_cancel_windows_are_computed_from_the_terms() {
        let terms = BookingTerms::builder()
            .supported_access_methods(vec![AccessMethod::Open])
            .change_until_minutes(Number::from(60u32))
            .cancel_until_minutes(Number::from(30u32))
            .build();
        let start = dt("2024-06-01T12:00:00Z");
        assert!(terms.may_change_at(dt("2024-06-01T10:00:00Z"), start));
        assert!(!terms.may_change_at(dt("2024-06-01T11:30:00Z"), start), "inside the 60 minutes");
        assert!(terms.may_cancel_at(dt("2024-06-01T11:30:00Z"), start));
        assert!(!terms.may_cancel_at(dt("2024-06-01T11:45:00Z"), start));

        let frozen = BookingTerms { change_not_allowed: Some(true), ..terms };
        assert!(!frozen.may_change_at(dt("2024-06-01T00:00:00Z"), start));
    }

    #[test]
    fn an_access_method_that_needs_a_value_must_have_one() {
        let bare = AccessInformation {
            method: AccessMethod::AccessCode,
            value: None,
            extensions: Extensions::new(),
        };
        assert_eq!(bare.validate().unwrap_err().as_slice()[0].pointer, "/value");

        let open =
            AccessInformation { method: AccessMethod::Open, value: None, extensions: Extensions::new() };
        assert!(open.validate().is_ok(), "OPEN needs nothing");
    }

    #[test]
    fn a_location_that_requires_a_reservation_offers_no_ad_hoc_charging() {
        let contradiction = Policy {
            reservation_required: true,
            ad_hoc: Some(Number::from(2u32)),
            extensions: Extensions::new(),
        };
        assert!(contradiction.validate().is_err());
        let coherent = Policy { ad_hoc: Some(Number::ZERO), ..contradiction };
        assert!(coherent.validate().is_ok());
    }
}
