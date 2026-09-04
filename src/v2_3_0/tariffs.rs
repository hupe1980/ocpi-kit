//! The *Tariffs* module of OCPI 2.3.0: what charging costs.
//!
//! *Module Identifier: `tariffs`* — Data owner: CPO.
//!
//! A Tariff is a list of [`TariffElement`]s; each is a group of [`PriceComponent`]s that share
//! [`TariffRestrictions`]. Evaluating them against a session is the job of
//! [`crate::tariffs`], the pricing engine.
//!
//! > *NOTE: There are no parameters related to price rounding in the Tariff object or any of its
//! > constituent objects. Nor does the specification text of this module give any requirements
//! > about how to do price rounding.*
//!
//! Spec: 2.3.0 §mod_tariffs_tariffs_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    CiString, CountryCode, Currency, DateTime, DisplayText, Extensions, LocalDate, LocalTime, Number,
    PartyId, PartyRef, Url, Validate, Validator, ViolationCode,
};
use crate::{ocpi_enum, ocpi_lenient_enum};

use super::locations::EnergyMix;

/// A tariff: one or more [`TariffElement`]s that price a charging session.
///
/// > *When the list of Tariff Elements contains more than one Element that has a Price Component
/// > for a certain dimension, then the first Tariff Element with a Price Component for that
/// > dimension in the list with matching Tariff Restrictions will be used.*
///
/// Spec: 2.3.0 §mod_tariffs_tariff_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Tariff {
    /// ISO-3166 alpha-2 country code of the CPO that owns this Tariff.
    pub country_code: CountryCode,
    /// ID of the CPO that 'owns' this Tariff.
    pub party_id: PartyId,
    /// Uniquely identifies the tariff within the CPO's platform.
    pub id: CiString<36>,
    /// ISO-4217 code of the currency of this tariff.
    pub currency: Currency,
    /// The type of the tariff. When omitted, this tariff is valid for all sessions.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub tariff_type: Option<TariffType>,
    /// Multi-language alternative tariff info texts.
    ///
    /// > *When a Tariff contains both the `tariff_alt_text` and `elements` fields, the
    /// > `tariff_alt_text` SHALL only contain additional tariff information in human-readable
    /// > text, not the price information that is also available via the `elements` field.*
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tariff_alt_text: Vec<DisplayText>,
    /// URL to a web page explaining the tariff in human-readable form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tariff_alt_url: Option<Url>,
    /// A Charging Session with this tariff will cost at least this amount.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_price: Option<PriceLimit>,
    /// A Charging Session with this tariff will cost at most this amount.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price: Option<PriceLimit>,
    /// The amount a Payment Terminal Provider should preauthorize for a Session with this
    /// Tariff.
    ///
    /// Defined by the OCPI 2.3.0 **`payments` release branch**, together with the module it is
    /// about, so it lives behind the `payments` feature — the same shape as the `bookings`
    /// branch's additions to core objects.
    ///
    /// Spec: 2.3.0 payments branch §mod_tariffs_preauthorize_amount_field
    #[cfg(feature = "payments")]
    #[cfg_attr(docsrs, doc(cfg(feature = "payments")))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preauthorize_amount: Option<Number>,
    /// The Tariff Elements. Cardinality `+`.
    pub elements: Vec<TariffElement>,
    /// Whether taxes are included in the amounts in this Tariff. New in OCPI 2.3.0.
    pub tax_included: TaxIncluded,
    /// When this tariff becomes active, in UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date_time: Option<DateTime>,
    /// When this tariff stops being valid, in UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date_time: Option<DateTime>,
    /// Details on the energy supplied with this tariff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_mix: Option<EnergyMix>,
    /// Timestamp when this Tariff was last updated (or created).
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Tariff {
    /// The CPO that owns this Tariff.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }

    /// Whether this tariff is in its validity window at `instant`.
    ///
    /// An absent `start_date_time` means "already active"; an absent `end_date_time` means
    /// "still active".
    #[must_use]
    pub fn is_active_at(&self, instant: DateTime) -> bool {
        self.start_date_time.is_none_or(|s| instant >= s) && self.end_date_time.is_none_or(|e| instant < e)
    }

    /// Whether this is the "Free of Charge" shape the spec prescribes.
    ///
    /// > *To define a "Free of Charge" tariff in OCPI, a Tariff containing one Tariff Element
    /// > with no restrictions containing one Price Component with `type` = `FLAT` and
    /// > `price` = `0.00` has to be provided.*
    #[must_use]
    pub fn is_free_of_charge(&self) -> bool {
        match self.elements.as_slice() {
            [element] if element.restrictions.is_none() => match element.price_components.as_slice() {
                [pc] => pc.component_type == TariffDimensionType::Flat && pc.price.is_zero(),
                _ => false,
            },
            _ => false,
        }
    }
}

impl Validate for Tariff {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self, v, country_code, party_id, id, currency, tariff_type as "type", tariff_alt_text,
            tariff_alt_url, min_price, max_price, elements, tax_included, start_date_time,
            end_date_time, energy_mix, last_updated,
        );
        #[cfg(feature = "payments")]
        validate_fields!(self, v, preauthorize_amount);
        if self.elements.is_empty() {
            v.report_at(
                "elements",
                ViolationCode::EmptyRequiredList,
                "a Tariff has cardinality `+` elements: at least one is required",
            );
        }
        if let (Some(start), Some(end)) = (self.start_date_time, self.end_date_time)
            && end <= start
        {
            v.report_at(
                "end_date_time",
                ViolationCode::Inconsistent,
                "a tariff's validity window must be non-empty",
            );
        }
        if let (Some(min), Some(max)) = (self.min_price.as_ref(), self.max_price.as_ref())
            && max.before_taxes < min.before_taxes
        {
            v.report_at(
                "max_price",
                ViolationCode::Inconsistent,
                "max_price.before_taxes is below min_price.before_taxes",
            );
        }
        // "A reservation can only have: FLAT and TIME TariffDimensions."
        for (i, element) in self.elements.iter().enumerate() {
            let Some(restrictions) = element.restrictions.as_ref() else { continue };
            if restrictions.reservation.is_none() {
                continue;
            }
            for (j, pc) in element.price_components.iter().enumerate() {
                if !matches!(pc.component_type, TariffDimensionType::Flat | TariffDimensionType::Time) {
                    v.enter("elements");
                    v.enter(&i.to_string());
                    v.enter("price_components");
                    v.enter(&j.to_string());
                    v.report_at(
                        "type",
                        ViolationCode::Inconsistent,
                        format!(
                            "a reservation Tariff Element can only have FLAT and TIME dimensions, \
                             not {}",
                            pc.component_type
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

/// A group of [`PriceComponent`]s that share a set of restrictions.
///
/// > *That the Price Components share the same restrictions does not mean that at any time, they
/// > either all apply or all do not apply. The reason is that applicable Price Components are
/// > looked up separately for each dimension.*
///
/// Spec: 2.3.0 §mod_tariffs_tariffelement_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct TariffElement {
    /// How each priced dimension is priced. Cardinality `+`.
    pub price_components: Vec<PriceComponent>,
    /// Under which circumstances these Price Components apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restrictions: Option<TariffRestrictions>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl TariffElement {
    /// The Price Component for one dimension, if this element prices it.
    #[must_use]
    pub fn component(&self, dimension: TariffDimensionType) -> Option<&PriceComponent> {
        self.price_components.iter().find(|c| c.component_type == dimension)
    }
}

impl Validate for TariffElement {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, price_components, restrictions);
        if self.price_components.is_empty() {
            v.report_at(
                "price_components",
                ViolationCode::EmptyRequiredList,
                "a TariffElement has cardinality `+` price_components: at least one is required",
            );
        }
        let mut seen: Vec<TariffDimensionType> = Vec::new();
        for pc in &self.price_components {
            if seen.contains(&pc.component_type) {
                v.report_at(
                    "price_components",
                    ViolationCode::Inconsistent,
                    format!(
                        "{} is priced twice in one Tariff Element; only one Price Component per \
                         dimension can be active at a time",
                        pc.component_type
                    ),
                );
            }
            seen.push(pc.component_type);
        }
    }
}

/// How consumption of one dimension translates into money owed.
///
/// Spec: 2.3.0 §mod_tariffs_pricecomponent_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct PriceComponent {
    /// The dimension that is being priced.
    #[serde(rename = "type")]
    pub component_type: TariffDimensionType,
    /// Price per unit for this dimension, including or excluding taxes according to the
    /// containing Tariff's `tax_included` field.
    pub price: Number,
    /// Applicable VAT percentage for this dimension. If omitted, no VAT is applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vat: Option<Number>,
    /// Minimum amount to be billed: the dimension is billed in blocks of this size.
    ///
    /// > *NOTE: The `step_size` field is no longer present in OCPI 3.0. … Users of OCPI 2.2.1
    /// > looking to be ready for a transition to OCPI 3.0 … are advised to effectively avoid
    /// > using `step_size` by setting `step_size` to 1 always.*
    ///
    /// The unit is the dimension's `step_size` multiplier: 1 Wh for `ENERGY`, 1 second for the
    /// time dimensions, and nothing for `FLAT`. See [`TariffDimensionType::step_size_unit`].
    pub step_size: u32,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl PriceComponent {
    /// A price component with no VAT and a `step_size` of 1.
    #[must_use]
    pub fn new(component_type: TariffDimensionType, price: Number) -> Self {
        Self { component_type, price, vat: None, step_size: 1, extensions: Extensions::new() }
    }
}

impl Validate for PriceComponent {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, component_type as "type", price, vat);
        // FLAT is "a flat fee without unit for step_size", so its value carries no meaning —
        // the specification's own free-of-charge example writes `"step_size": 0` there. For a
        // dimension that does have a unit, a step of zero would bill nothing.
        if self.step_size == 0 && self.component_type.step_size_unit().is_some() {
            v.report_at(
                "step_size",
                ViolationCode::OutOfRange,
                format!(
                    "a step_size of 0 would bill no {}; the smallest meaningful value is 1",
                    self.component_type
                ),
            );
        }
        if self.vat.is_some_and(Number::is_negative) {
            v.report_at("vat", ViolationCode::OutOfRange, "a VAT percentage cannot be negative");
        }
    }
}

/// A minimum or maximum total cost for a Charging Session. New in OCPI 2.3.0.
///
/// > *As the taxes on a Charging Session might be different for different parts of the Session,
/// > there might be situations where the minimum cost after taxes is reached earlier or later
/// > than the minimum price before taxes. So as a rule, they both apply.*
///
/// Spec: 2.3.0 §mod_tariffs_pricelimit_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PriceLimit {
    /// Maximum or minimum cost excluding taxes.
    pub before_taxes: Number,
    /// Maximum or minimum cost including taxes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_taxes: Option<Number>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl PriceLimit {
    /// A limit on the pre-tax amount only.
    #[must_use]
    pub fn before_taxes(amount: Number) -> Self {
        Self { before_taxes: amount, after_taxes: None, extensions: Extensions::new() }
    }
}

impl Validate for PriceLimit {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, before_taxes, after_taxes);
        if self.after_taxes.is_some_and(|a| a < self.before_taxes) {
            v.report_at(
                "after_taxes",
                ViolationCode::Inconsistent,
                "the amount including taxes cannot be lower than the amount excluding them",
            );
        }
    }
}

/// When a [`TariffElement`] is active during a Charging Session.
///
/// > *When more than one restriction is set, they are to be treated as a logical AND. So a Tariff
/// > Element is active if and only if all of the properties in its TariffRestrictions match.*
///
/// Spec: 2.3.0 §mod_tariffs_tariffrestrictions_class
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct TariffRestrictions {
    /// Start time of day in local time, e.g. `13:30`; valid from this time of the day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<LocalTime>,
    /// End time of day in local time.
    ///
    /// > *If `end_time` < `start_time` then the period wraps around to the next day. To stop at
    /// > end of the day use: 00:00.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<LocalTime>,
    /// Start date in local time; valid from this day, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<LocalDate>,
    /// End date in local time; valid until this day, exclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<LocalDate>,
    /// Minimum consumed energy in kWh, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_kwh: Option<Number>,
    /// Maximum consumed energy in kWh, exclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_kwh: Option<Number>,
    /// Sum of the minimum current over all phases, in A, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_current: Option<Number>,
    /// Sum of the maximum current over all phases, in A, exclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_current: Option<Number>,
    /// Minimum power in kW, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_power: Option<Number>,
    /// Maximum power in kW, exclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_power: Option<Number>,
    /// Minimum duration in seconds the Charging Session must last, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration: Option<u64>,
    /// Maximum duration in seconds the Charging Session must last, exclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<u64>,
    /// Which days of the week this Tariff Element is active.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub day_of_week: Vec<DayOfWeek>,
    /// When present, this Tariff Element describes reservation costs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation: Option<ReservationRestrictionType>,
    /// When present, this Tariff Element describes **booking** costs.
    ///
    /// Added by the OCPI 2.3.0 `bookings` release branch, so it is behind the `bookings` feature.
    ///
    /// Spec: 2.3.0-bookings §mod_tariffs_tariffrestrictions_class
    #[cfg(feature = "bookings")]
    #[cfg_attr(docsrs, doc(cfg(feature = "bookings")))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub booking: Option<BookingRestrictionType>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl TariffRestrictions {
    /// Whether no restriction at all is set, making this the element's fallback.
    ///
    /// > *It is advised to always add a "default" Price Component per dimension. This can be
    /// > achieved by adding a Tariff Element without restrictions after all other occurrences.*
    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        self == &Self::default()
    }

    /// Whether this element prices a reservation rather than a charging session.
    #[must_use]
    pub const fn is_reservation(&self) -> bool {
        self.reservation.is_some()
    }
}

impl Validate for TariffRestrictions {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            start_time,
            end_time,
            start_date,
            end_date,
            min_kwh,
            max_kwh,
            min_current,
            max_current,
            min_power,
            max_power,
            day_of_week,
            reservation,
        );
        #[cfg(feature = "bookings")]
        validate_fields!(self, v, booking);
        // A wrap-around window is legal for times, but not for dates or magnitudes.
        for (lo_name, lo, hi_name, hi) in [
            ("min_kwh", self.min_kwh, "max_kwh", self.max_kwh),
            ("min_current", self.min_current, "max_current", self.max_current),
            ("min_power", self.min_power, "max_power", self.max_power),
        ] {
            if let (Some(lo_v), Some(hi_v)) = (lo, hi)
                && hi_v <= lo_v
            {
                v.report_at(
                    hi_name,
                    ViolationCode::Inconsistent,
                    format!("{hi_name} is not above {lo_name}, so this element can never apply"),
                );
            }
        }
        if let (Some(lo), Some(hi)) = (self.min_duration, self.max_duration)
            && hi <= lo
        {
            v.report_at(
                "max_duration",
                ViolationCode::Inconsistent,
                "max_duration is not above min_duration, so this element can never apply",
            );
        }
        if let (Some(start), Some(end)) = (self.start_date, self.end_date)
            && end <= start
        {
            v.report_at(
                "end_date",
                ViolationCode::Inconsistent,
                "end_date is exclusive and must be after start_date",
            );
        }
        let mut seen: Vec<DayOfWeek> = Vec::new();
        for d in &self.day_of_week {
            if seen.contains(d) {
                v.report_at(
                    "day_of_week",
                    ViolationCode::Inconsistent,
                    format!("{d} is listed more than once"),
                );
            }
            seen.push(*d);
        }
    }
}

ocpi_enum! {
    /// A day of the week, as used in [`TariffRestrictions::day_of_week`].
    ///
    /// Spec: 2.3.0 §mod_tariffs_dayofweek_enum
    pub enum DayOfWeek {
        /// Monday.
        Monday = "MONDAY",
        /// Tuesday.
        Tuesday = "TUESDAY",
        /// Wednesday.
        Wednesday = "WEDNESDAY",
        /// Thursday.
        Thursday = "THURSDAY",
        /// Friday.
        Friday = "FRIDAY",
        /// Saturday.
        Saturday = "SATURDAY",
        /// Sunday.
        Sunday = "SUNDAY",
    }
}

impl DayOfWeek {
    /// The ISO weekday number, Monday = 1 through Sunday = 7.
    ///
    /// This is the same numbering `RegularHours.weekday` uses.
    #[must_use]
    pub const fn iso_number(self) -> u8 {
        match self {
            Self::Monday => 1,
            Self::Tuesday => 2,
            Self::Wednesday => 3,
            Self::Thursday => 4,
            Self::Friday => 5,
            Self::Saturday => 6,
            Self::Sunday => 7,
        }
    }

    /// The day for an ISO weekday number, Monday = 1 through Sunday = 7.
    #[must_use]
    pub const fn from_iso_number(n: u8) -> Option<Self> {
        Some(match n {
            1 => Self::Monday,
            2 => Self::Tuesday,
            3 => Self::Wednesday,
            4 => Self::Thursday,
            5 => Self::Friday,
            6 => Self::Saturday,
            7 => Self::Sunday,
            _ => return None,
        })
    }
}

ocpi_enum! {
    /// Whether a Tariff Element prices a reservation.
    ///
    /// > *A reservation starts when the reservation is made, and ends when the driver starts
    /// > charging on the reserved EVSE/Location, or when the reservation expires.*
    ///
    /// Spec: 2.3.0 §mod_tariffs_reservation_restriction_type
    pub enum ReservationRestrictionType {
        /// Costs for a reservation.
        Reservation = "RESERVATION",
        /// Costs for a reservation that expires before the driver starts charging.
        ReservationExpires = "RESERVATION_EXPIRES",
    }
}

// `#[cfg]` gates the whole expansion, so it stays out here; the `doc(cfg)` badge has to go *in*,
// on the enum the macro emits. Attached to the invocation instead it documents nothing, and
// rustdoc rejects it: "rustdoc does not generate documentation for macro invocations".
#[cfg(feature = "bookings")]
ocpi_enum! {
    /// What kind of booking cost a Tariff Element describes.
    ///
    /// Spec: 2.3.0-bookings §mod_tariffs_booking_restriction_type
    #[cfg_attr(docsrs, doc(cfg(feature = "bookings")))]
    pub enum BookingRestrictionType {
        /// Costs for a booking.
        Booking = "BOOKING",
        /// Costs for a booking that does not start within the booked period.
        BookingExpires = "BOOKING_EXPIRES",
        /// Costs for cancelling a booking.
        BookingCancellationFees = "BOOKING_CANCELLATION_FEES",
        /// Costs for charging after the booking has completed.
        BookingOvertime = "BOOKING_OVERTIME",
    }
}

ocpi_enum! {
    /// The dimensions a [`PriceComponent`] can price.
    ///
    /// Spec: 2.3.0 §mod_tariffs_tariffdimensiontype_enum
    pub enum TariffDimensionType {
        /// Defined in kWh; `step_size` multiplier 1 Wh.
        Energy = "ENERGY",
        /// Flat fee, without a unit for `step_size`.
        Flat = "FLAT",
        /// Time not charging, in hours; `step_size` multiplier 1 second.
        ParkingTime = "PARKING_TIME",
        /// Time charging, in hours; `step_size` multiplier 1 second.
        ///
        /// > *Can also be used in combination with a RESERVATION restriction to describe the
        /// > price of the reservation time.*
        Time = "TIME",
    }
}

impl TariffDimensionType {
    /// The unit that `step_size` is a multiple of, or `None` for `FLAT`.
    ///
    /// > *`ENERGY` has the `step_size` multiplier: 1 Wh … `PARKING_TIME` has the `step_size`
    /// > multiplier: 1 second.*
    #[must_use]
    pub const fn step_size_unit(self) -> Option<&'static str> {
        match self {
            Self::Energy => Some("Wh"),
            Self::ParkingTime | Self::Time => Some("s"),
            Self::Flat => None,
        }
    }

    /// Whether this dimension is measured over time rather than over energy.
    ///
    /// The distinction matters for `step_size`: the spec says it is *"only taken into account
    /// once per session for ENERGY and once for PARKING_TIME and TIME combined"*.
    #[must_use]
    pub const fn is_time_based(self) -> bool {
        matches!(self, Self::Time | Self::ParkingTime)
    }
}

ocpi_lenient_enum! {
    /// The kind of session a Tariff applies to.
    ///
    /// Spec: 2.3.0 §mod_tariffs_tariff_type
    pub enum TariffType {
        /// Valid when ad-hoc payment is used at the Charge Point.
        AdHocPayment = "AD_HOC_PAYMENT",
        /// Valid when the Charging Preference `CHEAP` is set for the session.
        ProfileCheap = "PROFILE_CHEAP",
        /// Valid when the Charging Preference `FAST` is set for the session.
        ProfileFast = "PROFILE_FAST",
        /// Valid when the Charging Preference `GREEN` is set for the session.
        ProfileGreen = "PROFILE_GREEN",
        /// Valid when using an RFID without a Charging Preference, or with `REGULAR`.
        Regular = "REGULAR",
    }
}

ocpi_enum! {
    /// Whether taxes are included in the amounts of a Tariff. New in OCPI 2.3.0.
    ///
    /// This is what makes North American tax handling expressible: a CPO there often does not
    /// know the rate when it publishes the Tariff, so it publishes pre-tax prices and says `NO`.
    ///
    /// Spec: 2.3.0 §mod_tariffs_taxincluded_enum
    pub enum TaxIncluded {
        /// Taxes are included in the prices in this Tariff.
        Yes = "YES",
        /// Taxes are not included and will be added on top of the prices in this Tariff.
        No = "NO",
        /// No taxes are applicable to this Tariff.
        NotApplicable = "N/A",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tariff(elements: Vec<TariffElement>) -> Tariff {
        Tariff::builder()
            .country_code("DE")
            .party_id("ALL")
            .id("12")
            .currency("EUR")
            .elements(elements)
            .tax_included(TaxIncluded::No)
            .last_updated("2018-12-17T11:16:55Z".parse::<DateTime>().unwrap())
            .build()
    }

    fn flat(price: &str) -> PriceComponent {
        PriceComponent::new(TariffDimensionType::Flat, price.parse().unwrap())
    }

    #[test]
    fn free_of_charge_has_the_exact_shape_the_spec_prescribes() {
        let free = tariff(vec![TariffElement::builder().price_components(vec![flat("0.00")]).build()]);
        assert!(free.is_free_of_charge());

        let with_restriction = tariff(vec![
            TariffElement::builder()
                .price_components(vec![flat("0.00")])
                .restrictions(TariffRestrictions {
                    max_kwh: Some("10".parse().unwrap()),
                    ..Default::default()
                })
                .build(),
        ]);
        assert!(!with_restriction.is_free_of_charge(), "a restricted zero price is not free");
        assert!(
            !tariff(vec![TariffElement::builder().price_components(vec![flat("0.25")]).build()])
                .is_free_of_charge()
        );
    }

    #[test]
    fn reservation_elements_may_only_price_flat_and_time() {
        let bad = tariff(vec![
            TariffElement::builder()
                .price_components(vec![PriceComponent::new(
                    TariffDimensionType::Energy,
                    "0.25".parse().unwrap(),
                )])
                .restrictions(TariffRestrictions {
                    reservation: Some(ReservationRestrictionType::Reservation),
                    ..Default::default()
                })
                .build(),
        ]);
        let err = bad.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].pointer, "/elements/0/price_components/0/type");
    }

    #[test]
    fn a_dimension_cannot_be_priced_twice_in_one_element() {
        let e = TariffElement::builder().price_components(vec![flat("1"), flat("2")]).build();
        assert_eq!(e.validate().unwrap_err().as_slice()[0].code, ViolationCode::Inconsistent);
    }

    #[test]
    fn impossible_restriction_windows_are_reported() {
        let r = TariffRestrictions {
            min_kwh: Some("20".parse().unwrap()),
            max_kwh: Some("10".parse().unwrap()),
            ..Default::default()
        };
        assert_eq!(r.validate().unwrap_err().as_slice()[0].pointer, "/max_kwh");
        // Times may wrap around midnight, so no complaint there.
        let wrap = TariffRestrictions {
            start_time: Some("22:00".parse().unwrap()),
            end_time: Some("06:00".parse().unwrap()),
            ..Default::default()
        };
        assert!(wrap.validate().is_ok());
    }

    #[test]
    fn step_size_units_follow_the_dimension() {
        assert_eq!(TariffDimensionType::Energy.step_size_unit(), Some("Wh"));
        assert_eq!(TariffDimensionType::Time.step_size_unit(), Some("s"));
        assert_eq!(TariffDimensionType::Flat.step_size_unit(), None);
        // FLAT has no unit, so any step_size is meaningless rather than wrong; the spec's own
        // free-of-charge example writes 0 there.
        assert!(PriceComponent { step_size: 0, ..flat("0.00") }.validate().is_ok());
        let no_energy = PriceComponent {
            step_size: 0,
            ..PriceComponent::new(TariffDimensionType::Energy, "0.25".parse().unwrap())
        };
        assert_eq!(no_energy.validate().unwrap_err().as_slice()[0].pointer, "/step_size");
    }

    #[test]
    fn validity_window_is_checked_against_an_instant() {
        let mut t = tariff(vec![TariffElement::builder().price_components(vec![flat("1")]).build()]);
        t.end_date_time = Some("2019-06-30T00:00:00Z".parse().unwrap());
        assert!(t.is_active_at("2019-01-01T00:00:00Z".parse().unwrap()));
        assert!(!t.is_active_at("2019-07-01T00:00:00Z".parse().unwrap()));
    }

    #[test]
    fn iso_weekday_numbering_matches_regular_hours() {
        assert_eq!(DayOfWeek::Monday.iso_number(), 1);
        assert_eq!(DayOfWeek::Sunday.iso_number(), 7);
        assert_eq!(DayOfWeek::from_iso_number(3), Some(DayOfWeek::Wednesday));
        assert_eq!(DayOfWeek::from_iso_number(0), None);
    }
}
