//! Checking a CDR against what its own tariff says it should have cost.
//!
//! This is the reason OCPI carries both the tariff and the metering data: an eMSP receiving a CDR
//! can recompute the cost from what crossed the wire, and a CPO can check its own before sending
//! it. What that recomputation needs to be useful is not a yes/no — it is *which number* differs
//! and by how much, because "your invoice is wrong by €0.03" is a support ticket and "your
//! PARKING_TIME total is 3 minutes longer than your own Charging Periods add up to" is a bug
//! report.
//!
//! So this compares three separate things, and reports each one it can:
//!
//! 1. **The totals.** `total_cost` against the priced total, before and after tax.
//! 2. **The per-dimension costs.** `total_energy_cost`, `total_time_cost`, `total_parking_cost`,
//!    `total_fixed_cost` and `total_reservation_cost`, each against the dimension it names. Two
//!    implementations that disagree on a total usually agree on three dimensions out of four, and
//!    the one they disagree on is the answer.
//! 3. **The CDR against itself.** `total_energy`, `total_time` and `total_parking_time` against
//!    what the Charging Periods actually add up to. A CDR whose headline quantities do not match
//!    its own periods is malformed whatever anybody's tariff says, and no cost comparison can
//!    tell you that.
//!
//! ```no_run
//! use ocpi_kit::tariffs::{PricedSession, PricingEngine, TimeZone, verify_cdr};
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let cdr: ocpi_kit::v2_3_0::cdrs::Cdr = todo!();
//! let session = PricedSession::from_cdr(&cdr, TimeZone::named("Europe/Amsterdam")?);
//! let breakdown = PricingEngine::new().price(&session, &cdr.tariffs)?;
//!
//! let check = verify_cdr(&cdr, &breakdown);
//! if !check.agrees() {
//!     for divergence in &check.divergences {
//!         eprintln!("{divergence}");
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Spec: 2.3.0 §mod_cdrs_cdr_object

use core::fmt;

use crate::types::Number;
use crate::v2_3_0::cdrs::Cdr;
use crate::v2_3_0::tariffs::TariffDimensionType;

use super::breakdown::CostBreakdown;

/// Which figure of a CDR a [`Divergence`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Discrepancy {
    /// `total_cost.before_taxes` against the priced total excluding tax.
    TotalBeforeTaxes,
    /// The sum of `total_cost.taxes` against the priced tax.
    TotalAfterTaxes,
    /// `total_energy_cost` against the priced `ENERGY`.
    EnergyCost,
    /// `total_time_cost` against the priced `TIME`.
    TimeCost,
    /// `total_parking_cost` against the priced `PARKING_TIME`.
    ParkingCost,
    /// `total_fixed_cost` against the priced `FLAT`.
    FixedCost,
    /// `total_reservation_cost` against the priced reserved time.
    ReservationCost,
    /// `total_energy` against the `ENERGY` its own Charging Periods add up to.
    EnergyVolume,
    /// `total_time` against the CDR's own `start_date_time` and `end_date_time`.
    ///
    /// *"Total duration of the charging session (including the duration of charging and not
    /// charging), in hours"* — the session, not the `TIME` dimension, so its own two timestamps
    /// settle it with no tariff involved.
    TimeVolume,
    /// `total_parking_time` against the `PARKING_TIME` its own Charging Periods add up to.
    ParkingVolume,
}

impl Discrepancy {
    /// A short, stable, machine-readable slug.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TotalBeforeTaxes => "total_before_taxes",
            Self::TotalAfterTaxes => "total_after_taxes",
            Self::EnergyCost => "energy_cost",
            Self::TimeCost => "time_cost",
            Self::ParkingCost => "parking_cost",
            Self::FixedCost => "fixed_cost",
            Self::ReservationCost => "reservation_cost",
            Self::EnergyVolume => "energy_volume",
            Self::TimeVolume => "time_volume",
            Self::ParkingVolume => "parking_volume",
        }
    }

    /// Whether this is a quantity the CDR reports about itself rather than a cost.
    ///
    /// A quantity divergence is a defect in the CDR — its own numbers disagree — and needs no
    /// tariff to establish. A cost divergence is a disagreement between two implementations.
    #[must_use]
    pub const fn is_self_inconsistency(self) -> bool {
        matches!(self, Self::EnergyVolume | Self::TimeVolume | Self::ParkingVolume)
    }
}

impl fmt::Display for Discrepancy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One figure the CDR and the recomputation disagree about.
#[derive(Clone, Debug, PartialEq)]
pub struct Divergence {
    /// Which figure.
    pub what: Discrepancy,
    /// What the CDR says.
    pub claimed: Number,
    /// What pricing its own Charging Periods against its own Tariff produces.
    pub computed: Number,
}

impl Divergence {
    /// `claimed - computed`: positive when the CDR asks for more than the tariff supports.
    #[must_use]
    pub fn difference(&self) -> Number {
        self.claimed - self.computed
    }
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let subject = match self.what {
            Discrepancy::TimeVolume => "its own start and end timestamps give",
            Discrepancy::EnergyVolume | Discrepancy::ParkingVolume => "its own Charging Periods add up to",
            _ => "pricing its Charging Periods gives",
        };
        write!(
            f,
            "{}: the CDR says {}, {subject} {} (difference {})",
            self.what,
            self.claimed,
            self.computed,
            self.difference()
        )
    }
}

/// The result of checking a CDR against a recomputation of it.
#[derive(Clone, Debug, PartialEq)]
pub struct CdrVerification {
    /// Every figure that differs by more than the tolerance, in a stable order.
    pub divergences: Vec<Divergence>,
    /// The tolerance the comparison used.
    pub tolerance: Number,
}

impl CdrVerification {
    /// Whether every figure the CDR reports is within tolerance of the recomputation.
    #[must_use]
    pub fn agrees(&self) -> bool {
        self.divergences.is_empty()
    }

    /// Whether the CDR's own quantities disagree with its own Charging Periods.
    ///
    /// This is the half that needs no tariff and admits no interpretation: a CDR whose
    /// `total_energy` is not what its periods add up to is malformed, and a total that happens to
    /// come out right does not make it less so.
    #[must_use]
    pub fn is_self_inconsistent(&self) -> bool {
        self.divergences.iter().any(|d| d.what.is_self_inconsistency())
    }

    /// The divergence for one figure, if there is one.
    #[must_use]
    pub fn divergence(&self, what: Discrepancy) -> Option<&Divergence> {
        self.divergences.iter().find(|d| d.what == what)
    }
}

impl fmt::Display for CdrVerification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.agrees() {
            return f.write_str("the CDR agrees with its own tariff and its own Charging Periods");
        }
        for (i, divergence) in self.divergences.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{divergence}")?;
        }
        Ok(())
    }
}

/// Checks a CDR against a [`CostBreakdown`] computed from it, comparing exactly.
///
/// Exact is the right default: both sides are decimals, and a cent that appears from nowhere is
/// the thing this exists to find. Use [`verify_cdr_within`] where a contract allows a rounding
/// difference.
///
/// # Panics
///
/// Never. Every comparison is between two decimals that are already bounded by
/// [`PricingEngine::price`](super::PricingEngine::price).
#[must_use]
pub fn verify_cdr(cdr: &Cdr, breakdown: &CostBreakdown) -> CdrVerification {
    verify_cdr_within(cdr, breakdown, Number::ZERO)
}

/// Checks a CDR against a [`CostBreakdown`], ignoring differences at or below `tolerance`.
///
/// # What is deliberately not compared
///
/// `total_fixed_cost` is *"the total sum of all the fixed costs … **except** fixed price
/// components of parking and reservation"*, and `total_parking_cost` and
/// `total_reservation_cost` are each *"including fixed price components"*. OCPI gives no way to
/// say which `FLAT` component belongs to which of the three, so when a session has a `FLAT` **and**
/// parking or reserved time, the attribution is not derivable from the documents and all three
/// comparisons are skipped rather than reported as disagreements this crate cannot adjudicate.
#[must_use]
pub fn verify_cdr_within(cdr: &Cdr, breakdown: &CostBreakdown, tolerance: Number) -> CdrVerification {
    let mut divergences = Vec::new();
    let mut compare = |what: Discrepancy, claimed: Number, computed: Number| {
        if (claimed - computed).abs() > tolerance {
            divergences.push(Divergence { what, claimed, computed });
        }
    };

    compare(Discrepancy::TotalBeforeTaxes, cdr.total_cost.before_taxes, breakdown.total_excl_vat);
    compare(Discrepancy::TotalAfterTaxes, cdr.total_cost.after_taxes(), breakdown.total_incl_vat);

    let parking_cost = breakdown.dimension_total(TariffDimensionType::ParkingTime);
    let flat_cost = breakdown.dimension_total(TariffDimensionType::Flat);
    let (charging_time_cost, reservation_cost) = split_time_cost(breakdown);
    // A `FLAT` beside parking or reserved time cannot be attributed to one of the three fields
    // that claim it; see the note above.
    let attributable = flat_cost.is_zero() || (parking_cost.is_zero() && reservation_cost.is_zero());

    let mut costs = vec![
        (
            Discrepancy::EnergyCost,
            cdr.total_energy_cost.as_ref(),
            breakdown.dimension_total(TariffDimensionType::Energy),
        ),
        // "the cost related to duration of **charging**" — so not the reserved time that shares
        // the TIME dimension with it.
        (Discrepancy::TimeCost, cdr.total_time_cost.as_ref(), charging_time_cost),
    ];
    if attributable {
        costs.push((Discrepancy::ParkingCost, cdr.total_parking_cost.as_ref(), parking_cost));
        costs.push((Discrepancy::FixedCost, cdr.total_fixed_cost.as_ref(), flat_cost));
        costs.push((Discrepancy::ReservationCost, cdr.total_reservation_cost.as_ref(), reservation_cost));
    }
    for (what, claimed, computed) in costs {
        // A cost the CDR does not break out is not a disagreement: the fields are optional, and
        // a CPO that omits one has not claimed anything about it.
        if let Some(price) = claimed {
            compare(what, price.before_taxes, computed);
        }
    }

    // The CDR against itself. `PricedSession::from_cdr` reads the same Charging Periods, so the
    // measured quantities in the breakdown *are* what those periods add up to.
    let measured =
        |dimension: TariffDimensionType| breakdown.dimension(dimension).map_or(Number::ZERO, |d| d.measured);
    compare(Discrepancy::EnergyVolume, cdr.total_energy, measured(TariffDimensionType::Energy));
    if let Some(parking) = cdr.total_parking_time {
        compare(Discrepancy::ParkingVolume, parking, measured(TariffDimensionType::ParkingTime));
    }
    // `total_time` is *"the total duration of the charging session (including the duration of
    // charging and not charging)"* — the session, not the TIME dimension — so it is checked
    // against the CDR's own two timestamps. Nothing but the CDR is needed to settle it.
    compare(Discrepancy::TimeVolume, cdr.total_time, session_hours(cdr));

    divergences.sort_by_key(|d| d.what);
    CdrVerification { divergences, tolerance }
}

/// The session's own duration in hours, from its two timestamps.
fn session_hours(cdr: &Cdr) -> Number {
    let seconds = cdr.end_date_time.unix_timestamp() - cdr.start_date_time.unix_timestamp();
    Number::from(seconds) / Number::from(3600u32)
}

/// The `TIME` dimension split into charging time and reserved time.
///
/// Reserved time shares the dimension with charging time — OCPI has no reservation dimension, only
/// a reservation *restriction* — so the audit trail is what tells them apart:
/// [`AppliedComponent::reservation`](super::AppliedComponent::reservation) is set on the segments
/// priced as a reservation.
fn split_time_cost(breakdown: &CostBreakdown) -> (Number, Number) {
    let mut charging = Number::ZERO;
    let mut reserved = Number::ZERO;
    for segment in breakdown.dimension(TariffDimensionType::Time).into_iter().flat_map(|d| d.segments.iter())
    {
        if segment.applied.reservation {
            reserved = reserved + segment.cost;
        } else {
            charging = charging + segment.cost;
        }
    }
    (charging, reserved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tariffs::{PricedPeriod, PricedSession, PricingEngine, TimeZone};
    use crate::types::DateTime;
    use crate::v2_3_0::tariffs::{PriceComponent, Tariff, TariffElement, TaxIncluded};
    use crate::v2_3_0::types::Price;

    fn n(s: &str) -> Number {
        s.parse().unwrap()
    }

    fn tariff() -> Tariff {
        Tariff::builder()
            .country_code("NL")
            .party_id("TNM")
            .id("T1")
            .currency("EUR")
            .elements(vec![
                TariffElement::builder()
                    .price_components(vec![PriceComponent {
                        component_type: TariffDimensionType::Energy,
                        price: n("0.25"),
                        vat: None,
                        step_size: 1,
                        extensions: crate::types::Extensions::new(),
                    }])
                    .build(),
            ])
            .tax_included(TaxIncluded::No)
            .last_updated("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .build()
    }

    fn priced(energy: &str) -> CostBreakdown {
        let session =
            PricedSession::new("2024-01-15T10:00:00Z".parse::<DateTime>().unwrap(), TimeZone::utc())
                .with_period(PricedPeriod {
                    energy_kwh: n(energy),
                    ..PricedPeriod::new("2024-01-15T10:00:00Z".parse::<DateTime>().unwrap())
                });
        PricingEngine::new().price(&session, &[tariff()]).unwrap()
    }

    fn cdr(total: &str, energy: &str) -> Cdr {
        let mut cdr = crate::testkit::sample::cdr("CDR1").unwrap();
        cdr.total_cost = Price::new(n(total));
        cdr.total_energy = n(energy);
        cdr.total_energy_cost = Some(Price::new(n(total)));
        // The sample CDR runs for an hour, and `total_time` is the session's duration.
        cdr.total_time = Number::ONE;
        cdr.total_time_cost = None;
        cdr.total_parking_time = None;
        cdr.charging_periods.clear();
        cdr
    }

    #[test]
    fn a_matching_cdr_agrees() {
        let breakdown = priced("20");
        let check = verify_cdr(&cdr("5.00", "20"), &breakdown);
        assert!(check.agrees(), "{check}");
    }

    #[test]
    fn a_cost_that_differs_names_the_dimension() {
        let breakdown = priced("20");
        let check = verify_cdr(&cdr("5.50", "20"), &breakdown);
        assert!(!check.agrees());
        assert_eq!(check.divergence(Discrepancy::EnergyCost).unwrap().difference(), n("0.50"));
        assert!(!check.is_self_inconsistent(), "the CDR is consistent with itself, just dearer");
    }

    #[test]
    fn a_cdr_whose_own_total_energy_does_not_match_its_periods_is_self_inconsistent() {
        let breakdown = priced("20");
        let check = verify_cdr(&cdr("5.00", "21"), &breakdown);
        assert!(check.is_self_inconsistent());
        assert_eq!(check.divergence(Discrepancy::EnergyVolume).unwrap().difference(), n("1"));
    }

    /// The check that needs no tariff: a CDR whose own `total_time` is not its own duration.
    #[test]
    fn a_total_time_that_is_not_the_session_duration_is_self_inconsistent() {
        let breakdown = priced("20");
        let mut wrong = cdr("5.00", "20");
        wrong.total_time = n("1.5"); // the sample CDR runs for an hour
        let check = verify_cdr(&wrong, &breakdown);
        assert!(check.is_self_inconsistent(), "{check}");
        assert_eq!(check.divergence(Discrepancy::TimeVolume).unwrap().difference(), n("0.5"));
        assert!(check.to_string().contains("its own start and end timestamps"), "{check}");
    }

    /// A `FLAT` beside parking or reserved time cannot be attributed to one of the three fields
    /// that each claim to contain it, so those comparisons are skipped rather than invented.
    #[test]
    fn an_unattributable_flat_fee_suppresses_the_three_comparisons_that_claim_it() {
        use crate::tariffs::{PricedPeriod, PricedSession, TimeZone};
        let mut t = tariff();
        t.elements[0].price_components.push(PriceComponent {
            component_type: TariffDimensionType::Flat,
            price: n("0.50"),
            vat: None,
            step_size: 0,
            extensions: crate::types::Extensions::new(),
        });
        t.elements[0].price_components.push(PriceComponent {
            component_type: TariffDimensionType::ParkingTime,
            price: n("2.00"),
            vat: None,
            step_size: 1,
            extensions: crate::types::Extensions::new(),
        });
        let start: DateTime = "2024-01-15T10:00:00Z".parse().unwrap();
        let session = PricedSession::new(start, TimeZone::utc()).with_period(PricedPeriod {
            energy_kwh: n("20"),
            parking_hours: n("1"),
            ..PricedPeriod::new(start)
        });
        let breakdown = PricingEngine::new().price(&session, &[t]).unwrap();

        let mut with_claims = cdr("7.50", "20");
        with_claims.total_fixed_cost = Some(Price::new(n("999")));
        with_claims.total_parking_cost = Some(Price::new(n("999")));
        let check = verify_cdr(&with_claims, &breakdown);
        assert!(check.divergence(Discrepancy::FixedCost).is_none(), "{check}");
        assert!(check.divergence(Discrepancy::ParkingCost).is_none(), "{check}");
    }

    #[test]
    fn a_tolerance_absorbs_a_rounding_difference() {
        let breakdown = priced("20");
        let check = verify_cdr_within(&cdr("5.01", "20"), &breakdown, n("0.01"));
        assert!(check.agrees(), "{check}");
    }
}
