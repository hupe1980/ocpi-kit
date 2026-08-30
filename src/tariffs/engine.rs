//! The pricing algorithm.
//!
//! # How a Tariff is applied
//!
//! > *When the list of Tariff Elements contains more than one Element that has a Price Component
//! > for a certain dimension, then the first Tariff Element with a Price Component for that
//! > dimension in the list with matching Tariff Restrictions will be used. Only one Price
//! > Component per dimension can be active at any point in time.*
//!
//! So the lookup is **per dimension, per charging period**: for each period and each dimension it
//! consumed, walk the Tariff's elements in order and take the first one that both prices that
//! dimension and whose restrictions match at that moment.
//!
//! # `step_size`
//!
//! The rules are subtle and the specification spells them out with worked examples, which this
//! module implements literally:
//!
//! > *`step_size` SHALL only be taken into account once per session for the TariffDimensionType
//! > `ENERGY` and once for `PARKING_TIME` and `TIME` combined.*
//!
//! > *The `step_size` uses the total amount of a certain unit used during a session, not only the
//! > last ChargingPeriod. … If the `step_size` differs for the different TariffElements, the
//! > `step_size` of the last relevant PriceComponent is used.*
//!
//! > *In the cases that `TIME` and `PARKING_TIME` Tariff Elements are both used, `step_size` is
//! > only taken into account for the total parking duration.*
//!
//! In other words: `ENERGY` is rounded once, on the session total, with the step of the last
//! energy component used, and the extra lands in the last segment. The time dimensions are
//! rounded once between them, on the total of whichever of `TIME`/`PARKING_TIME` was active last,
//! with that dimension's last step size — so charging time followed by parking time is billed
//! exactly and only the parking is rounded up.

use crate::types::{DateTime, LocalDate, LocalTime, Number};
use crate::v2_3_0::tariffs::{
    DayOfWeek, PriceComponent, ReservationRestrictionType, Tariff, TariffDimensionType, TariffElement,
    TariffRestrictions,
};

use super::PricingError;
use super::breakdown::{
    AppliedComponent, CostBreakdown, DimensionCost, PriceLimitApplied, PricedSegment, TaxLine,
};
use super::input::{PricedPeriod, PricedSession};
use super::policy::PricingPolicy;

/// Prices sessions against Tariffs.
///
/// ```
/// use ocpi_kit::tariffs::{PricingEngine, PricedPeriod, PricedSession, TimeZone};
/// use ocpi_kit::v2_3_0::tariffs::*;
/// use ocpi_kit::types::{DateTime, Number};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // "Simple Tariff example € 2 per hour": 2.00/h excl. VAT, 10% VAT, billed per minute.
/// let tariff = Tariff::builder()
///     .country_code("DE").party_id("ALL").id("11").currency("EUR")
///     .elements(vec![TariffElement::builder()
///         .price_components(vec![PriceComponent {
///             component_type: TariffDimensionType::Time,
///             price: "2.00".parse()?,
///             vat: Some("10.0".parse()?),
///             step_size: 60,
///             extensions: Default::default(),
///         }])
///         .build()])
///     .tax_included(TaxIncluded::No)
///     .last_updated("2015-06-29T20:39:09Z".parse::<DateTime>()?)
///     .build();
///
/// let session = PricedSession::new("2024-01-15T10:00:00Z".parse()?, TimeZone::utc())
///     .with_period(PricedPeriod {
///         charging_hours: "2.5".parse()?,
///         ..PricedPeriod::new("2024-01-15T10:00:00Z".parse()?)
///     })
///     .ending("2024-01-15T12:30:00Z".parse()?);
///
/// let breakdown = PricingEngine::new().price(&session, &[tariff])?;
/// assert_eq!(breakdown.total_excl_vat.to_string(), "5.00");
/// assert_eq!(breakdown.total_incl_vat.to_string(), "5.50");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default)]
pub struct PricingEngine {
    policy: PricingPolicy,
}

impl PricingEngine {
    /// An engine with the default [`PricingPolicy`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An engine with a specific policy.
    #[must_use]
    pub fn with_policy(policy: PricingPolicy) -> Self {
        Self { policy }
    }

    /// The policy in use.
    #[must_use]
    pub const fn policy(&self) -> &PricingPolicy {
        &self.policy
    }

    /// Prices `session` against `tariffs`.
    ///
    /// When several tariffs are given, each charging period uses the one its `tariff_id` names;
    /// a period with no `tariff_id` uses the first tariff that is valid at that moment and whose
    /// `type` matches the session's charging preference.
    ///
    /// # Errors
    ///
    /// Returns [`PricingError`] when no applicable tariff can be found, or when the session's
    /// time zone cannot be resolved.
    pub fn price(&self, session: &PricedSession, tariffs: &[Tariff]) -> Result<CostBreakdown, PricingError> {
        if tariffs.is_empty() {
            return Err(PricingError::NoTariff);
        }
        let mut notes = Vec::new();

        // Per dimension, the segments that were priced.
        let mut segments: Vec<(TariffDimensionType, PricedSegment, u32)> = Vec::new();
        let mut flat_charged = false;

        for (index, period) in session.periods.iter().enumerate() {
            let tariff = Self::select_tariff(session, period, tariffs)?;
            let context = RestrictionContext::build(session, index, period)?;

            for (dimension, quantity) in period_quantities(period) {
                if quantity.is_zero() {
                    continue;
                }
                let Some(found) = find_component(tariff, dimension, &context) else {
                    notes.push(format!(
                        "no {dimension} Price Component in tariff {} matched at {}; \
                         the specification says there are then no costs for that dimension",
                        tariff.id, period.start
                    ));
                    continue;
                };
                segments.push((
                    dimension,
                    PricedSegment {
                        start: period.start,
                        quantity,
                        price: found.component.price,
                        vat_percentage: found.component.vat,
                        cost: Number::ZERO, // filled in after quantisation
                        applied: found.applied(tariff, &context),
                    },
                    found.component.step_size,
                ));
            }

            // "FLAT: Flat fee without unit for step_size" — charged once for the session, by the
            // first element that prices it and whose restrictions match.
            if !flat_charged && let Some(found) = find_component(tariff, TariffDimensionType::Flat, &context)
            {
                flat_charged = true;
                segments.push((
                    TariffDimensionType::Flat,
                    PricedSegment {
                        start: period.start,
                        quantity: Number::ONE,
                        price: found.component.price,
                        vat_percentage: found.component.vat,
                        cost: Number::ZERO,
                        applied: found.applied(tariff, &context),
                    },
                    1,
                ));
            }
        }

        let dimensions = self.quantise_and_cost(segments);
        let tariff = Self::select_tariff_for_limits(session, tariffs)?;
        Ok(self.finish(dimensions, tariff, notes))
    }

    /// Applies `step_size` per the rules on this module, then costs every segment.
    fn quantise_and_cost(
        &self,
        segments: Vec<(TariffDimensionType, PricedSegment, u32)>,
    ) -> Vec<DimensionCost> {
        use TariffDimensionType::{Energy, Flat, ParkingTime, Time};

        // Which time dimension was active last decides which one absorbs the rounding.
        let last_time_dimension = segments.iter().rfind(|(d, _, _)| d.is_time_based()).map(|(d, _, _)| *d);

        let mut by_dimension: Vec<(TariffDimensionType, Vec<(PricedSegment, u32)>)> = Vec::new();
        for (dimension, segment, step) in segments {
            match by_dimension.iter_mut().find(|(d, _)| *d == dimension) {
                Some((_, list)) => list.push((segment, step)),
                None => by_dimension.push((dimension, vec![(segment, step)])),
            }
        }

        let mut out = Vec::with_capacity(by_dimension.len());
        for (dimension, mut list) in by_dimension {
            let measured: Number = list.iter().map(|(s, _)| s.quantity).sum();

            // Only ENERGY and the last time-based dimension are quantised; a FLAT has no unit.
            let quantise = match dimension {
                Energy => true,
                Time | ParkingTime => last_time_dimension == Some(dimension),
                Flat => false,
            };
            let billed = if quantise {
                // "the step_size of the last relevant PriceComponent is used"
                let step = list.last().map_or(1, |(_, step)| *step);
                let unit_scale = match dimension {
                    Energy => 1000, // kWh measured, Wh counted
                    _ => 3600,      // hours measured, seconds counted
                };
                self.policy.quantisation.apply(measured, step, unit_scale)
            } else {
                measured
            };

            // "The extra minutes are then added to the last period with a Price Component with a
            //  time-based dimension" — the surplus is billed at the last segment's rate.
            if billed != measured
                && let Some((last, _)) = list.last_mut()
            {
                last.quantity = last.quantity + (billed - measured);
            }

            let mut cost = Number::ZERO;
            let mut vat = Number::ZERO;
            let mut priced_segments = Vec::with_capacity(list.len());
            for (mut segment, _) in list {
                segment.cost = self.policy.round_component(segment.quantity * segment.price);
                cost = cost + segment.cost;
                if let Some(percentage) = segment.vat_percentage {
                    vat = vat + self.policy.round_component(segment.cost * percentage / Number::from(100u32));
                }
                priced_segments.push(segment);
            }

            out.push(DimensionCost {
                dimension,
                measured,
                billed,
                cost: self.policy.round_component(cost),
                vat: self.policy.round_component(vat),
                segments: priced_segments,
            });
        }
        out
    }

    /// Totals the dimensions, groups the VAT and applies `min_price`/`max_price`.
    fn finish(&self, dimensions: Vec<DimensionCost>, tariff: &Tariff, notes: Vec<String>) -> CostBreakdown {
        let mut taxes: Vec<TaxLine> = Vec::new();
        for dimension in &dimensions {
            for segment in &dimension.segments {
                let Some(percentage) = segment.vat_percentage else { continue };
                let amount = self.policy.round_component(segment.cost * percentage / Number::from(100u32));
                match taxes.iter_mut().find(|t| t.percentage == percentage) {
                    Some(line) => {
                        line.taxable = line.taxable + segment.cost;
                        line.amount = line.amount + amount;
                    }
                    None => taxes.push(TaxLine { percentage, taxable: segment.cost, amount }),
                }
            }
        }
        taxes.sort_by_key(|a| a.percentage);

        let raw_excl: Number = dimensions.iter().map(|d| d.cost).sum();
        let raw_vat: Number = taxes.iter().map(|t| t.amount).sum();
        let mut total_excl = self.policy.round_currency(raw_excl);
        let mut total_incl = self.policy.round_currency(raw_excl + raw_vat);
        let mut limit_applied = None;

        // "The total cost of a Charging Session before taxes can never be lower than the value of
        //  the min_price's before_taxes field" — and the same for after taxes, independently.
        if let Some(min) = tariff.min_price.as_ref() {
            if total_excl < min.before_taxes {
                total_excl = self.policy.round_currency(min.before_taxes);
                limit_applied = Some(PriceLimitApplied::Minimum);
            }
            if let Some(after) = min.after_taxes
                && total_incl < after
            {
                total_incl = self.policy.round_currency(after);
                limit_applied = Some(PriceLimitApplied::Minimum);
            }
        }
        if let Some(max) = tariff.max_price.as_ref() {
            if total_excl > max.before_taxes {
                total_excl = self.policy.round_currency(max.before_taxes);
                limit_applied = Some(PriceLimitApplied::Maximum);
            }
            if let Some(after) = max.after_taxes
                && total_incl > after
            {
                total_incl = self.policy.round_currency(after);
                limit_applied = Some(PriceLimitApplied::Maximum);
            }
        }
        // A clamp on the pre-tax total that left the inclusive total alone would be incoherent;
        // recompute it from the clamped base unless an explicit after-tax bound decided it.
        if limit_applied.is_some() && total_incl < total_excl {
            total_incl = total_excl;
        }

        CostBreakdown {
            dimensions,
            total_excl_vat: total_excl,
            total_incl_vat: total_incl,
            taxes,
            limit_applied,
            notes,
        }
    }

    /// The tariff that applies to one charging period.
    fn select_tariff<'a>(
        session: &PricedSession,
        period: &PricedPeriod,
        tariffs: &'a [Tariff],
    ) -> Result<&'a Tariff, PricingError> {
        if let Some(id) = period.tariff_id.as_deref() {
            return tariffs
                .iter()
                .find(|t| t.id.eq_ignore_case(id))
                .ok_or_else(|| PricingError::UnknownTariff(id.to_owned()));
        }
        Self::select_by_preference(session, period.start, tariffs)
    }

    /// The tariff whose `min_price`/`max_price` bound the session as a whole.
    fn select_tariff_for_limits<'a>(
        session: &PricedSession,
        tariffs: &'a [Tariff],
    ) -> Result<&'a Tariff, PricingError> {
        Self::select_by_preference(session, session.start, tariffs)
    }

    fn select_by_preference<'a>(
        session: &PricedSession,
        at: DateTime,
        tariffs: &'a [Tariff],
    ) -> Result<&'a Tariff, PricingError> {
        use crate::v2_3_0::sessions::ProfileType;
        use crate::v2_3_0::tariffs::TariffType;
        let wanted = if session.ad_hoc_payment {
            Some(TariffType::AdHocPayment)
        } else {
            match session.profile_type {
                Some(ProfileType::Cheap) => Some(TariffType::ProfileCheap),
                Some(ProfileType::Fast) => Some(TariffType::ProfileFast),
                Some(ProfileType::Green) => Some(TariffType::ProfileGreen),
                Some(ProfileType::Regular) => Some(TariffType::Regular),
                None => None,
            }
        };
        let active: Vec<&Tariff> = tariffs.iter().filter(|t| t.is_active_at(at)).collect();
        if active.is_empty() {
            return Err(PricingError::NoActiveTariff(at));
        }
        // Prefer a tariff whose `type` matches the preference; then one with no `type`, which
        // "is valid for all sessions"; then whatever is left.
        if let Some(wanted) = wanted
            && let Some(t) = active.iter().find(|t| t.tariff_type == Some(wanted))
        {
            return Ok(t);
        }
        if let Some(t) = active.iter().find(|t| t.tariff_type.is_none()) {
            return Ok(t);
        }
        Ok(active[0])
    }
}

/// The quantities one period consumed, in the order the dimensions are evaluated.
fn period_quantities(period: &PricedPeriod) -> [(TariffDimensionType, Number); 3] {
    [
        (TariffDimensionType::Energy, period.energy_kwh),
        (TariffDimensionType::Time, period.charging_hours + period.reservation_hours),
        (TariffDimensionType::ParkingTime, period.parking_hours),
    ]
}

/// Everything a restriction can be evaluated against, at one moment of one session.
struct RestrictionContext {
    local_time: LocalTime,
    local_date: LocalDate,
    weekday: DayOfWeek,
    energy_so_far: Number,
    duration_so_far_seconds: i64,
    current_lower: Option<Number>,
    current_upper: Option<Number>,
    power_lower: Option<Number>,
    power_upper: Option<Number>,
    is_reservation: bool,
    reservation_expired: bool,
}

impl RestrictionContext {
    fn build(session: &PricedSession, index: usize, period: &PricedPeriod) -> Result<Self, PricingError> {
        let local = session.time_zone.to_local(period.start)?;
        Ok(Self {
            local_time: LocalTime::new(local.hour(), local.minute())
                .map_err(|e| PricingError::TimeZone(e.to_string()))?,
            local_date: LocalDate::from_date(local.date()),
            weekday: DayOfWeek::from_iso_number(local.weekday().number_from_monday())
                .unwrap_or(DayOfWeek::Monday),
            energy_so_far: session.energy_before(index),
            duration_so_far_seconds: session.duration_before(index),
            current_lower: period.current_for_lower_bound(),
            current_upper: period.current_for_upper_bound(),
            power_lower: period.power_for_lower_bound(),
            power_upper: period.power_for_upper_bound(),
            is_reservation: !period.reservation_hours.is_zero(),
            reservation_expired: session.reservation_expired,
        })
    }

    fn describe(&self) -> String {
        format!(
            "at {} {} local ({}), {} kWh and {}s into the session",
            self.local_date, self.local_time, self.weekday, self.energy_so_far, self.duration_so_far_seconds
        )
    }
}

/// The Price Component that priced one dimension of one period, with where it came from.
struct Found<'a> {
    component: &'a PriceComponent,
    element_index: usize,
    component_index: usize,
}

impl Found<'_> {
    fn applied(&self, tariff: &Tariff, context: &RestrictionContext) -> AppliedComponent {
        AppliedComponent {
            tariff_id: tariff.id.as_str().to_owned(),
            element_index: self.element_index,
            component_index: self.component_index,
            because: context.describe(),
        }
    }
}

/// The first Tariff Element that prices `dimension` and whose restrictions match.
fn find_component<'a>(
    tariff: &'a Tariff,
    dimension: TariffDimensionType,
    context: &RestrictionContext,
) -> Option<Found<'a>> {
    for (element_index, element) in tariff.elements.iter().enumerate() {
        if !restrictions_match(element, context) {
            continue;
        }
        for (component_index, component) in element.price_components.iter().enumerate() {
            if component.component_type == dimension {
                return Some(Found { component, element_index, component_index });
            }
        }
    }
    None
}

fn restrictions_match(element: &TariffElement, context: &RestrictionContext) -> bool {
    let Some(restrictions) = element.restrictions.as_ref() else {
        // "a Tariff Element without restrictions … will act as fallback"
        return !context.is_reservation || element_prices_reservation_dimension(element);
    };
    matches(restrictions, context)
}

/// A reservation period may only be priced by an element that is about reservations, or by a
/// fallback element with `FLAT`/`TIME` components.
fn element_prices_reservation_dimension(element: &TariffElement) -> bool {
    element
        .price_components
        .iter()
        .any(|c| matches!(c.component_type, TariffDimensionType::Flat | TariffDimensionType::Time))
}

/// Whether every restriction that is set matches. *"they are to be treated as a logical AND."*
fn matches(r: &TariffRestrictions, context: &RestrictionContext) -> bool {
    // "When this field is present, the TariffElement describes reservation costs."
    match r.reservation {
        Some(ReservationRestrictionType::Reservation) => {
            if !context.is_reservation || context.reservation_expired {
                return false;
            }
        }
        Some(ReservationRestrictionType::ReservationExpires) => {
            if !context.is_reservation || !context.reservation_expired {
                return false;
            }
        }
        None => {
            if context.is_reservation {
                // A non-reservation element does not price a reservation period.
                return false;
            }
        }
    }

    if let (Some(start), Some(end)) = (r.start_time, r.end_time) {
        if !context.local_time.is_within(start, end) {
            return false;
        }
    } else if let Some(start) = r.start_time {
        if context.local_time < start {
            return false;
        }
    } else if let Some(end) = r.end_time
        && context.local_time >= end
    {
        return false;
    }

    // "start_date … valid from this day (inclusive)"; "end_date … valid until this day (exclusive)"
    if r.start_date.is_some_and(|d| context.local_date < d) {
        return false;
    }
    if r.end_date.is_some_and(|d| context.local_date >= d) {
        return false;
    }

    // "min_kwh … valid from this amount of energy (inclusive) being used"
    if r.min_kwh.is_some_and(|min| context.energy_so_far < min) {
        return false;
    }
    // "max_kwh … valid until this amount of energy (exclusive) being used"
    if r.max_kwh.is_some_and(|max| context.energy_so_far >= max) {
        return false;
    }

    if let Some(min) = r.min_current
        && context.current_lower.is_none_or(|c| c < min)
    {
        return false;
    }
    if let Some(max) = r.max_current
        && context.current_upper.is_none_or(|c| c >= max)
    {
        return false;
    }
    if let Some(min) = r.min_power
        && context.power_lower.is_none_or(|p| p < min)
    {
        return false;
    }
    if let Some(max) = r.max_power
        && context.power_upper.is_none_or(|p| p >= max)
    {
        return false;
    }

    if let Some(min) = r.min_duration
        && context.duration_so_far_seconds < i64_of(min)
    {
        return false;
    }
    if let Some(max) = r.max_duration
        && context.duration_so_far_seconds >= i64_of(max)
    {
        return false;
    }

    if !r.day_of_week.is_empty() && !r.day_of_week.contains(&context.weekday) {
        return false;
    }

    true
}

fn i64_of(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
