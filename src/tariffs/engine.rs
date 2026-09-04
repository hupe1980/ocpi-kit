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
//! rounded once between them: `PARKING_TIME` absorbs it whenever the session has any, and `TIME`
//! only when it does not — so charging time followed by parking time is billed exactly and only
//! the parking is rounded up, which is the specification's own worked example.
//!
//! # What the engine assumes about its input, and what it does when that fails
//!
//! A Charging Period is the unit of pricing: it carries totals, not a curve, so there is no way
//! to know how much of its energy fell before a tariff switched and how much after. The
//! specification puts the obligation on the CPO instead:
//!
//! > *A CPO SHALL at least start (and add) a ChargingPeriod every moment/event that has relevance
//! > for the total costs of a CDR. … When an energy changes in price after 17:00. The CPO has to
//! > start a new Charging Period at 17:00.*
//!
//! Every implementation therefore assumes well-formed periods, and prices each one at the rate
//! that applied when it began. What is unusual here is that **this engine checks**: it re-evaluates
//! the restrictions at the moment each period ends, and when a different Price Component would
//! have applied by then it records a
//! [`PeriodSpansPriceChange`](super::PricingNoteCode::PeriodSpansPriceChange) note naming the
//! dimension and the moment.
//!
//! That turns a silent few cents into a reviewable line. The total beside it is still the best
//! answer available from the data — nothing is guessed or interpolated — but the reader is told
//! that the CDR broke a `SHALL`, which is exactly the finding an invoice reconciliation exists to
//! produce.

use crate::types::{DateTime, LocalDate, LocalTime, Number};
use crate::v2_3_0::tariffs::{
    DayOfWeek, PriceComponent, ReservationRestrictionType, Tariff, TariffDimensionType, TariffElement,
    TariffRestrictions, TaxIncluded,
};

use super::PricingError;
use super::breakdown::{
    AppliedComponent, CostBreakdown, DimensionCost, PriceLimitApplied, PricedSegment, PricingNote,
    PricingNoteCode, TaxBasis, TaxLine,
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
        let mut notes: Vec<PricingNote> = Vec::new();

        // Per dimension, the segments that were priced.
        let mut segments: Vec<Segment> = Vec::new();
        let mut flat_charged = false;

        // Nothing below multiplies a value this has not seen. See `PricingError::OutOfRange`.
        check_range(session, tariffs)?;

        // A pricing input can be built by hand as well as read off a CDR, so the order the
        // rest of this function depends on is checked here rather than assumed.
        if let Some(at) = session.first_out_of_order() {
            notes.push(PricingNote::new(
                PricingNoteCode::PeriodsOutOfOrder,
                Some(at),
                "this Charging Period does not start after the one before it; `step_size` and \
                 every duration-based restriction are evaluated against the order given, which \
                 is not a timeline this session could have had",
            ));
        }

        for (index, period) in session.periods.iter().enumerate() {
            let tariff = Self::select_tariff(session, period, tariffs)?;
            let context = RestrictionContext::build(session, index, period)?;
            let end_context = RestrictionContext::build_at_end(session, index, period)?;

            for (dimension, quantity, reserving) in period_quantities(period) {
                if quantity.is_zero() {
                    continue;
                }
                // A CDR may carry TIME and RESERVATION_TIME in the same ChargingPeriod, and the
                // two are priced by different Tariff Elements — one restricted with
                // `reservation`, one not. Looking both up against a single "this period is a
                // reservation" flag would bill the charging minutes at the reservation rate, so
                // each quantity is looked up in the context that describes *it*.
                let context = context.reserving(reserving);
                let Some(found) = find_component(tariff, dimension, &context) else {
                    notes.push(PricingNote::new(
                        PricingNoteCode::NoPriceComponent,
                        Some(period.start),
                        format!(
                            "no {dimension} Price Component in tariff {} matched{}; \
                             the specification says there are then no costs for that dimension",
                            tariff.id,
                            if reserving { " for the reserved time" } else { "" },
                        ),
                    ));
                    continue;
                };

                // The period is priced at the rate that applied when it began. If a different
                // one would apply by the time it ends, the CPO should have split it here.
                if let Some(end) = end_context.as_ref().map(|c| c.reserving(reserving))
                    && let Some(later) = find_component(tariff, dimension, &end)
                    && (later.element_index, later.component_index)
                        != (found.element_index, found.component_index)
                {
                    notes.push(PricingNote::new(
                        PricingNoteCode::PeriodSpansPriceChange,
                        Some(period.start),
                        format!(
                            "the {dimension} Charging Period starting here outlasts the Price \
                             Component that prices it: element {} applies at the start and \
                             element {} by the time the period ends. A CPO SHALL start a new \
                             Charging Period at a price change, so this one should have been \
                             split; its {dimension} is billed in full at the earlier rate, \
                             because nothing in the period says how it divides",
                            found.element_index, later.element_index,
                        ),
                    ));
                }
                segments.push(Segment {
                    dimension,
                    step_size: found.component.step_size,
                    priced: PricedSegment {
                        start: period.start,
                        quantity,
                        price: found.component.price,
                        vat_percentage: found.component.vat,
                        cost: Number::ZERO, // filled in after quantisation
                        tax: Number::ZERO,
                        tax_basis: basis_of(tariff),
                        applied: found.applied(tariff, &context),
                    },
                });
            }

            // "FLAT: Flat fee without unit for step_size" — charged once for the session, by the
            // first element that prices it and whose restrictions match.
            //
            // A period can be both charging and reserved at once, and the start fee of a session
            // is not a reservation cost, so the ordinary context is tried first and the
            // reservation one only for a period that is nothing but a reservation.
            if !flat_charged {
                let charging = context.reserving(false);
                let reserved = context.reserving(true);
                let found = find_component(tariff, TariffDimensionType::Flat, &charging)
                    .map(|f| (f, charging))
                    .or_else(|| {
                        (!period.reservation_hours.is_zero())
                            .then(|| {
                                find_component(tariff, TariffDimensionType::Flat, &reserved)
                                    .map(|f| (f, reserved))
                            })
                            .flatten()
                    });
                if let Some((found, context)) = found {
                    flat_charged = true;
                    segments.push(Segment {
                        dimension: TariffDimensionType::Flat,
                        step_size: 1,
                        priced: PricedSegment {
                            start: period.start,
                            quantity: Number::ONE,
                            price: found.component.price,
                            vat_percentage: found.component.vat,
                            cost: Number::ZERO,
                            tax: Number::ZERO,
                            tax_basis: basis_of(tariff),
                            applied: found.applied(tariff, &context),
                        },
                    });
                }
            }
        }

        let dimensions = self.quantise_and_cost(segments, &mut notes);
        let tariff = Self::select_tariff_for_limits(session, tariffs)?;
        Ok(self.finish(dimensions, tariff, notes))
    }

    /// Applies `step_size` per the rules on this module, then costs every segment.
    fn quantise_and_cost(&self, segments: Vec<Segment>, notes: &mut Vec<PricingNote>) -> Vec<DimensionCost> {
        use TariffDimensionType::{Energy, Flat, ParkingTime, Time};

        // "In the cases that TIME and PARKING_TIME Tariff Elements are both used, `step_size` is
        //  only taken into account for the total parking duration."
        //
        // Unconditional, so PARKING_TIME absorbs the rounding whenever it appears at all. The
        // spec's own worked example justifies it sequentially instead — "the charging duration is
        // not rounded up, as it is followed by another time based period" — and the two readings
        // agree on every session that charges and then parks, which is every session. They part
        // only on one that parks and then charges, and there the sentence is what governs.
        let quantised_time_dimension =
            if segments.iter().any(|s| s.dimension == TariffDimensionType::ParkingTime) {
                Some(ParkingTime)
            } else {
                segments.iter().find(|s| s.dimension.is_time_based()).map(|s| s.dimension)
            };

        let mut by_dimension: Vec<(TariffDimensionType, Vec<Segment>)> = Vec::new();
        for segment in segments {
            match by_dimension.iter_mut().find(|(d, _)| *d == segment.dimension) {
                Some((_, list)) => list.push(segment),
                None => by_dimension.push((segment.dimension, vec![segment])),
            }
        }

        let mut out = Vec::with_capacity(by_dimension.len());
        for (dimension, mut list) in by_dimension {
            let measured: Number = list.iter().map(|s| s.priced.quantity).sum();

            // Only ENERGY and the last time-based dimension are quantised; a FLAT has no unit.
            let quantise = match dimension {
                Energy => true,
                Time | ParkingTime => quantised_time_dimension == Some(dimension),
                Flat => false,
            };
            let billed = if quantise {
                // "the step_size of the last relevant PriceComponent is used"
                let step = list.last().map_or(1, |s| s.step_size);
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
                && let Some(last) = list.last_mut()
            {
                last.priced.quantity = last.priced.quantity + (billed - measured);
            }

            let mut cost = Number::ZERO;
            let mut vat = Number::ZERO;
            let mut priced_segments = Vec::with_capacity(list.len());
            for segment in list {
                let mut priced = segment.priced;
                let stated = self.policy.round_component(priced.quantity * priced.price);
                let (net, tax) = self.split_tax(stated, priced.vat_percentage, priced.tax_basis, notes);
                priced.cost = net;
                priced.tax = tax;
                priced.quantity = self.policy.round_quantity(priced.quantity);
                cost = cost + priced.cost;
                vat = vat + priced.tax;
                priced_segments.push(priced);
            }

            out.push(DimensionCost {
                dimension,
                // Reported, not charged: the costs above came from the exact quantities.
                measured: self.policy.round_quantity(measured),
                billed: self.policy.round_quantity(billed),
                cost: self.policy.round_component(cost),
                vat: self.policy.round_component(vat),
                segments: priced_segments,
            });
        }
        out
    }

    /// Splits one component's amount into what is owed before tax and the tax on top of it.
    ///
    /// > *`price`: Price per unit for this dimension. This is including or excluding taxes
    /// > according to the `tax_included` field of the Tariff that this PriceComponent is
    /// > contained in.*
    ///
    /// Which makes `quantity × price` two different numbers depending on one field of the parent
    /// Tariff. Treating a gross amount as a net one and then adding the VAT — which is what an
    /// engine that ignores `tax_included` does — overcharges a Canadian or American session by
    /// exactly the tax.
    fn split_tax(
        &self,
        stated: Number,
        percentage: Option<Number>,
        basis: TaxBasis,
        notes: &mut Vec<PricingNote>,
    ) -> (Number, Number) {
        match (basis, percentage) {
            (TaxBasis::Excluded | TaxBasis::Mixed, Some(rate)) => {
                (stated, self.policy.round_component(stated * rate / Number::from(100u32)))
            }
            (TaxBasis::Excluded | TaxBasis::Mixed, None) => (stated, Number::ZERO),
            (TaxBasis::Included, Some(rate)) => {
                // gross = net × (1 + rate/100), so net = gross ÷ (1 + rate/100). The tax is the
                // remainder rather than a second rounding of the rate, so that net + tax is
                // exactly the amount the price component states — which is the number the driver
                // was quoted.
                let divisor = Number::ONE + rate / Number::from(100u32);
                if divisor.is_zero() {
                    return (stated, Number::ZERO);
                }
                let net = self.policy.round_component(stated / divisor);
                (net, stated - net)
            }
            (TaxBasis::Included, None) => {
                note_once(
                    notes,
                    PricingNoteCode::TaxIncludedWithoutRate,
                    "this Tariff's prices include tax and no Price Component says at what rate, \
                     so the two totals cannot be told apart: both are reported as the amount the \
                     Tariff states, which is the gross one. This is the ordinary North American \
                     shape, where the CPO does not know the rate when it publishes the Tariff",
                );
                (stated, Number::ZERO)
            }
            (TaxBasis::NotApplicable, rate) => {
                if rate.is_some() {
                    note_once(
                        notes,
                        PricingNoteCode::TaxRateIgnored,
                        "this Tariff says no taxes are applicable and a Price Component names a \
                         VAT percentage anyway; the Tariff-level statement governs and the \
                         percentage is ignored",
                    );
                }
                (stated, Number::ZERO)
            }
        }
    }

    /// Totals the dimensions, groups the VAT and applies `min_price`/`max_price`.
    ///
    /// # Why the tax lines move with the total
    ///
    /// The specification states the two price limits as independent rules:
    ///
    /// > *The total cost of a Charging Session before taxes can never be lower than the value of
    /// > the min_price's `before_taxes` field. The total cost of a Charging Session after taxes
    /// > can never be lower than the value of the min_price's `after_taxes` field.*
    ///
    /// Applying them literally and stopping there produces an incoherent document. A €0.50
    /// session with 21% VAT under a `min_price.before_taxes` of €5.00 becomes €5.00 net — but its
    /// tax lines still describe the €0.50 that was actually metered, so they no longer sum to the
    /// difference between the two totals. Nobody can file that.
    ///
    /// So a clamp moves the taxes with the base, in proportion, and the invariant
    /// `sum(taxes) == total_incl - total_excl` holds on every breakdown this engine produces. The
    /// specification says nothing about this because it does not describe a breakdown at all;
    /// this is the arithmetic that makes one usable, and a
    /// [`TotalClamped`](super::PricingNoteCode::TotalClamped) note records that it happened.
    fn finish(
        &self,
        dimensions: Vec<DimensionCost>,
        tariff: &Tariff,
        mut notes: Vec<PricingNote>,
    ) -> CostBreakdown {
        let mut taxes: Vec<TaxLine> = Vec::new();
        for dimension in &dimensions {
            for segment in &dimension.segments {
                // A tariff that says no tax applies gets no tax lines, even where a component
                // named a rate: publishing "21% VAT — 0.00" beside a total the tariff says has no
                // tax in it is noise in a document somebody reads to settle a dispute. The
                // contradiction itself is reported, once, as a `TaxRateIgnored` note.
                let quiet = segment.tax.is_zero()
                    && (segment.vat_percentage.is_none() || segment.tax_basis == TaxBasis::NotApplicable);
                if quiet {
                    continue;
                }
                let percentage = segment.vat_percentage;
                // The amount comes off the segment rather than being recomputed from the rate:
                // on a tax-inclusive tariff the two are not the same number, and the segment's is
                // the one that adds up to what the price component states.
                match taxes.iter_mut().find(|t| t.percentage == percentage) {
                    Some(line) => {
                        line.taxable = line.taxable + segment.cost;
                        line.amount = line.amount + segment.tax;
                    }
                    None => {
                        taxes.push(TaxLine { percentage, taxable: segment.cost, amount: segment.tax });
                    }
                }
            }
        }
        taxes.retain(|line| !(line.amount.is_zero() && line.percentage.is_none()));
        taxes.sort_by_key(|a| a.percentage);

        let tax_basis = summarise_basis(&dimensions);
        if tax_basis == TaxBasis::Mixed {
            note_once(
                &mut notes,
                PricingNoteCode::MixedTaxBasis,
                "the Charging Periods of this session were priced by Tariffs that disagree about \
                 whether their prices include tax, so the two totals mean something different for \
                 different parts of it",
            );
        }

        let raw_excl: Number = dimensions.iter().map(|d| d.cost).sum();
        let raw_vat: Number = taxes.iter().map(|t| t.amount).sum();
        let mut total_excl = self.policy.round_currency(raw_excl);
        let mut total_incl = self.policy.round_currency(raw_excl + raw_vat);
        let mut limit_applied = None;

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

        let mut base_ratio = Number::ONE;
        if let Some(applied) = limit_applied {
            // The pre-tax total moved, so the tax owed on it moved too — unless an explicit
            // after-tax bound already decided the inclusive total, in which case that wins and
            // the tax is whatever is left between them.
            base_ratio = if raw_excl.is_zero() {
                Number::ONE
            } else {
                total_excl.checked_div(raw_excl).unwrap_or(Number::ONE)
            };
            let bounded_after_tax = match applied {
                PriceLimitApplied::Minimum => tariff.min_price.as_ref().and_then(|p| p.after_taxes),
                PriceLimitApplied::Maximum => tariff.max_price.as_ref().and_then(|p| p.after_taxes),
            };
            if bounded_after_tax.is_none() {
                // Nothing was metered to scale from: the effective rate is unknowable.
                let scaled_vat = if raw_excl.is_zero() {
                    Number::ZERO
                } else {
                    raw_vat.checked_mul(base_ratio).unwrap_or(raw_vat)
                };
                total_incl = self.policy.round_currency(total_excl + scaled_vat);
            }
            total_incl = total_incl.max(total_excl);
            notes.push(PricingNote::new(
                PricingNoteCode::TotalClamped,
                None,
                format!(
                    "the session metered {raw_excl} before tax, which the tariff's {} price \
                     limit moved to {total_excl}; the tax lines were moved in proportion so they \
                     still account for the difference between the two totals",
                    match applied {
                        PriceLimitApplied::Minimum => "minimum",
                        PriceLimitApplied::Maximum => "maximum",
                    },
                ),
            ));
        }

        // A tariff with a negative `vat` is malformed — `Validate` reports it — but the engine
        // does not require validated input, and a breakdown where the session costs less with tax
        // than without is not a document anybody can use.
        if total_incl < total_excl {
            notes.push(PricingNote::new(
                PricingNoteCode::NegativeTax,
                None,
                format!(
                    "the price components of this tariff describe {} of tax, which no tariff can \
                     mean; the inclusive total is held at the exclusive one. A VAT percentage \
                     below zero is what causes this, and `Tariff::validate` names the component",
                    total_incl - total_excl,
                ),
            ));
            total_incl = total_excl;
        }

        self.present_taxes(&mut taxes, total_incl - total_excl, base_ratio, total_excl, &mut notes);

        CostBreakdown {
            dimensions,
            total_excl_vat: total_excl,
            total_incl_vat: total_incl,
            taxes,
            tax_basis,
            limit_applied,
            notes,
        }
    }

    /// Puts the tax lines into the shape a breakdown publishes them in.
    ///
    /// Two things happen here, and both are needed for the document to hold together.
    ///
    /// The lines are **rounded to currency precision**, like the totals beside them. They are
    /// accumulated at the finer `component_decimals`, so without this a 2% VAT on €2,502,360
    /// prints as `500.4720` next to totals that differ by `500.47`. That is a rounding
    /// discrepancy of half a cent and an audit finding.
    ///
    /// And they are made to sum to **exactly** `owed`, with the last line absorbing the residue,
    /// rather than each being rounded independently and hoping. `base_ratio` carries any
    /// `min_price`/`max_price` clamp through to the `taxable` bases, so those keep describing the
    /// amount the tax was actually charged on.
    fn present_taxes(
        &self,
        taxes: &mut Vec<TaxLine>,
        owed: Number,
        base_ratio: Number,
        taxable_base: Number,
        notes: &mut Vec<PricingNote>,
    ) {
        let current: Number = taxes.iter().map(|t| t.amount).sum();
        if taxes.is_empty() || current.is_zero() {
            if owed.is_zero() {
                for line in taxes.iter_mut() {
                    line.taxable = self
                        .policy
                        .round_currency(line.taxable.checked_mul(base_ratio).unwrap_or(line.taxable));
                    line.amount = Number::ZERO;
                }
                return;
            }
            // A price limit's `after_taxes` bound above a session that named no VAT at all: the
            // amount is a fact, the rate is not knowable, and inventing one would be a lie in a
            // document somebody files.
            notes.push(PricingNote::new(
                PricingNoteCode::UnattributedTax,
                None,
                format!(
                    "{owed} of tax is owed that no price component in this session accounts for; \
                     it comes from a price limit's `after_taxes` bound, which names an amount but \
                     not a rate",
                ),
            ));
            taxes.clear();
            taxes.push(TaxLine { percentage: None, taxable: taxable_base, amount: owed });
            return;
        }

        let mut running = Number::ZERO;
        let last = taxes.len() - 1;
        for (i, line) in taxes.iter_mut().enumerate() {
            line.taxable =
                self.policy.round_currency(line.taxable.checked_mul(base_ratio).unwrap_or(line.taxable));
            if i == last {
                line.amount = owed - running;
            } else {
                line.amount = self.policy.round_currency(
                    line.amount.checked_mul(owed).and_then(|v| v.checked_div(current)).unwrap_or(owed),
                );
                running = running + line.amount;
            }
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
            && let Some(t) = active.iter().find(|t| t.tariff_type.as_ref() == Some(&wanted))
        {
            return Ok(t);
        }
        if let Some(t) = active.iter().find(|t| t.tariff_type.is_none()) {
            return Ok(t);
        }
        Ok(active[0])
    }
}

/// One stretch of one dimension, with what pricing still needs to know about it.
struct Segment {
    dimension: TariffDimensionType,
    /// `step_size` of the Price Component that priced it, in the dimension's own unit.
    step_size: u32,
    priced: PricedSegment,
}

/// The tax basis a Tariff states.
const fn basis_of(tariff: &Tariff) -> TaxBasis {
    match tariff.tax_included {
        TaxIncluded::No => TaxBasis::Excluded,
        TaxIncluded::Yes => TaxBasis::Included,
        TaxIncluded::NotApplicable => TaxBasis::NotApplicable,
    }
}

/// The basis of the whole breakdown: [`TaxBasis::Mixed`] when its segments disagree.
fn summarise_basis(dimensions: &[DimensionCost]) -> TaxBasis {
    let mut seen: Option<TaxBasis> = None;
    for basis in dimensions.iter().flat_map(|d| d.segments.iter().map(|s| s.tax_basis)) {
        match seen {
            None => seen = Some(basis),
            Some(first) if first == basis => {}
            Some(_) => return TaxBasis::Mixed,
        }
    }
    seen.unwrap_or(TaxBasis::Excluded)
}

/// Records a note unless one with the same code is already there.
///
/// A tariff-level fact — "these prices include tax and no rate is given" — is one finding about
/// the session, not one per priced segment. Repeating it would make a breakdown unreadable
/// exactly when it most needs reading.
fn note_once(notes: &mut Vec<PricingNote>, code: PricingNoteCode, message: &str) {
    if !notes.iter().any(|n| n.code == code) {
        notes.push(PricingNote::new(code, None, message));
    }
}

/// The largest magnitude this engine will price.
///
/// A trillion of any currency unit, kWh or hour is past every real session by many orders of
/// magnitude, and it leaves the products and sums below three orders of magnitude of headroom
/// inside what a `Decimal` holds (about 7.9 × 10^28). See [`PricingError::OutOfRange`].
const MAX_MAGNITUDE: i64 = 1_000_000_000_000;

/// Refuses input that would make the arithmetic below overflow.
///
/// `rust_decimal` **panics** on overflow, and both the quantities and the prices here come off a
/// peer's CDR and a peer's Tariff. A panic in a reconciliation run over a month of CDRs loses the
/// whole run; a panic inside a hub's task loses somebody else's message. So the bound is checked
/// once, up front, and reported with the value that broke it.
fn check_range(session: &PricedSession, tariffs: &[Tariff]) -> Result<(), PricingError> {
    let limit = Number::from(MAX_MAGNITUDE);
    let check = |what: &str, value: Number| -> Result<(), PricingError> {
        if value.abs() > limit {
            return Err(PricingError::OutOfRange { what: what.to_owned(), value, limit });
        }
        Ok(())
    };
    for (index, period) in session.periods.iter().enumerate() {
        check(&format!("the ENERGY of Charging Period {index}"), period.energy_kwh)?;
        check(&format!("the TIME of Charging Period {index}"), period.charging_hours)?;
        check(&format!("the PARKING_TIME of Charging Period {index}"), period.parking_hours)?;
        check(&format!("the RESERVATION_TIME of Charging Period {index}"), period.reservation_hours)?;
    }
    for tariff in tariffs {
        for (e, element) in tariff.elements.iter().enumerate() {
            for (c, component) in element.price_components.iter().enumerate() {
                check(
                    &format!("the price of tariff {} element {e} component {c}", tariff.id),
                    component.price,
                )?;
                if let Some(vat) = component.vat {
                    check(&format!("the VAT of tariff {} element {e} component {c}", tariff.id), vat)?;
                }
            }
        }
        for (name, limit_price) in
            [("min_price", tariff.min_price.as_ref()), ("max_price", tariff.max_price.as_ref())]
        {
            if let Some(price) = limit_price {
                check(&format!("the {name}.before_taxes of tariff {}", tariff.id), price.before_taxes)?;
                if let Some(after) = price.after_taxes {
                    check(&format!("the {name}.after_taxes of tariff {}", tariff.id), after)?;
                }
            }
        }
    }
    Ok(())
}

/// The quantities one period consumed, in the order the dimensions are evaluated.
///
/// The third element says whether that quantity is **reserved** time rather than consumed time,
/// which is what decides between a Tariff Element restricted with `reservation` and one without.
/// Reserved time is a `TIME` quantity like any other — OCPI has no `RESERVATION` tariff dimension,
/// only a `reservation` *restriction* — so it shares the dimension, and therefore the `step_size`
/// budget, with charging time.
fn period_quantities(period: &PricedPeriod) -> [(TariffDimensionType, Number, bool); 4] {
    [
        (TariffDimensionType::Energy, period.energy_kwh, false),
        (TariffDimensionType::Time, period.charging_hours, false),
        (TariffDimensionType::Time, period.reservation_hours, true),
        (TariffDimensionType::ParkingTime, period.parking_hours, false),
    ]
}

/// Everything a restriction can be evaluated against, at one moment of one session.
#[derive(Clone, Copy)]
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
            local_time: local.time,
            local_date: local.date,
            weekday: DayOfWeek::from_iso_number(local.iso_weekday).unwrap_or(DayOfWeek::Monday),
            energy_so_far: session.energy_before(index),
            duration_so_far_seconds: session.duration_before(index),
            current_lower: period.current_for_lower_bound(),
            current_upper: period.current_for_upper_bound(),
            power_lower: period.power_for_lower_bound(),
            power_upper: period.power_for_upper_bound(),
            is_reservation: false,
            reservation_expired: session.reservation_expired,
        })
    }

    /// The context at the last instant the period covers, or `None` when the period has no known
    /// end — an open session's final period.
    ///
    /// One second before the end rather than at it, because a period is half-open: one that runs
    /// up to exactly 17:00 does not span the 17:00 boundary, and must not be reported as if it
    /// did.
    fn build_at_end(
        session: &PricedSession,
        index: usize,
        period: &PricedPeriod,
    ) -> Result<Option<Self>, PricingError> {
        let Some(end) = session.period_end(index) else { return Ok(None) };
        let Some(last_instant) = DateTime::from_unix_timestamp(end.unix_timestamp() - 1).ok() else {
            return Ok(None);
        };
        if last_instant <= period.start {
            // A period of a second or less cannot span anything.
            return Ok(None);
        }
        let local = session.time_zone.to_local(last_instant)?;
        Ok(Some(Self {
            local_time: local.time,
            local_date: local.date,
            weekday: DayOfWeek::from_iso_number(local.iso_weekday).unwrap_or(DayOfWeek::Monday),
            // By the end of the period, everything it consumed has been consumed.
            energy_so_far: session.energy_before(index) + period.energy_kwh,
            duration_so_far_seconds: last_instant.unix_timestamp() - session.start.unix_timestamp(),
            current_lower: period.current_for_lower_bound(),
            current_upper: period.current_for_upper_bound(),
            power_lower: period.power_for_lower_bound(),
            power_upper: period.power_for_upper_bound(),
            is_reservation: false,
            reservation_expired: session.reservation_expired,
        }))
    }

    /// This context, viewed as pricing reserved time or consumed time.
    const fn reserving(&self, is_reservation: bool) -> Self {
        Self { is_reservation, ..*self }
    }

    fn describe(&self) -> String {
        format!(
            "{}at {} {} local ({}), {} kWh and {}s into the session",
            if self.is_reservation { "pricing reserved time " } else { "" },
            self.local_date,
            self.local_time,
            self.weekday,
            self.energy_so_far,
            self.duration_so_far_seconds
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
            reservation: context.is_reservation,
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
        // "a Tariff Element without restrictions … will act as fallback" — for a session, not for
        // a reservation. Reservation costs are the one thing the specification says an element
        // has to *declare*: "When this field is present, the TariffElement describes reservation
        // costs." An element that does not carry the restriction is not about reservations, and
        // that has to hold for an unrestricted element too, or the same tariff would price
        // reserved time at the charging rate for one shape of element and not for another.
        //
        // A reservation nothing prices is therefore free, with a `NoPriceComponent` note — which
        // is exactly what the specification says an unpriced dimension costs, and it errs towards
        // not billing a driver for something the CPO never published a price for.
        return !context.is_reservation;
    };
    matches(restrictions, context)
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
