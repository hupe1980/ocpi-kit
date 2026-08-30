//! An auditable pricing engine: what a charging session costs, and exactly why.
//!
//! OCPI is the only protocol that carries both the tariff and the metering data, which means the
//! cost of a session is computable from what crosses the wire. Doing that well is worth a lot:
//! it is how an eMSP checks a CPO's invoice, how a CPO checks its own, and how the Payments
//! module's financial advice confirmations get reconciled against the CDRs they belong to.
//!
//! # What makes this engine different
//!
//! **The answer is auditable.** [`CostBreakdown`] does not just say `12.28`; it says which
//! quantity was billed for each dimension, what `step_size` did to it, which Tariff Element and
//! which Price Component priced it, and why that element was selected.
//!
//! **The arithmetic is exact.** Every value is a [`Number`](crate::types::Number), a decimal.
//! There is no `f64` anywhere in this module.
//!
//! **The undefined parts are parameters.** The specification says nothing about rounding, on
//! purpose, and OCPI 3.0 removes `step_size` altogether. Both are settings on
//! [`PricingPolicy`] rather than assumptions baked into the code.
//!
//! # Example
//!
//! ```
//! use ocpi_kit::tariffs::{PricedPeriod, PricedSession, PricingEngine, TimeZone};
//! use ocpi_kit::types::DateTime;
//! # use ocpi_kit::v2_3_0::tariffs::*;
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let tariff = Tariff::builder().country_code("DE").party_id("ALL").id("1").currency("EUR")
//! #   .elements(vec![TariffElement::builder().price_components(vec![
//! #       PriceComponent { component_type: TariffDimensionType::Energy, price: "0.25".parse()?,
//! #                        vat: Some("10".parse()?), step_size: 1, extensions: Default::default() }
//! #   ]).build()])
//! #   .tax_included(TaxIncluded::No).last_updated("2024-01-01T00:00:00Z".parse::<DateTime>()?).build();
//! let session = PricedSession::new("2024-01-15T10:00:00Z".parse()?, TimeZone::named("Europe/Berlin")?)
//!     .with_period(PricedPeriod {
//!         energy_kwh: "20".parse()?,
//!         ..PricedPeriod::new("2024-01-15T10:00:00Z".parse()?)
//!     });
//!
//! let breakdown = PricingEngine::new().price(&session, &[tariff])?;
//! assert_eq!(breakdown.total_excl_vat.to_string(), "5.00");
//! assert_eq!(breakdown.total_incl_vat.to_string(), "5.50");
//! # Ok(())
//! # }
//! ```
//!
//! Spec: 2.3.0 §mod_tariffs_tariffs_module, §mod_cdrs_step_size

mod breakdown;
mod engine;
mod input;
mod policy;

pub use breakdown::{
    AppliedComponent, CostBreakdown, DimensionCost, PriceLimitApplied, PricedSegment, PricingNote,
    PricingNoteCode, TaxLine,
};
pub use engine::PricingEngine;
pub use input::{PricedPeriod, PricedSession};
pub use policy::{PricingPolicy, Quantisation};

use core::fmt;

use crate::types::DateTime;

/// The IANA time zone a Location is in, which the local-time restrictions are expressed in.
///
/// > *`start_time`: Start time of day in local time, the time zone is defined in the `time_zone`
/// > field of the Location.*
///
/// A tariff that costs more after 17:00 is wrong by an hour for half the year unless the
/// conversion goes through the real zone rules, so this resolves the name against the IANA
/// database rather than assuming a fixed offset.
#[derive(Clone)]
pub struct TimeZone {
    name: String,
    zone: Option<tz::TimeZoneRef<'static>>,
}

impl TimeZone {
    /// Resolves an IANA time zone name, such as `Europe/Oslo`.
    ///
    /// # Errors
    ///
    /// Returns [`PricingError::TimeZone`] when the name is not in the IANA database.
    pub fn named(name: &str) -> Result<Self, PricingError> {
        if name.eq_ignore_ascii_case("UTC") {
            return Ok(Self::utc());
        }
        let zone = tzdb::tz_by_name(name)
            .ok_or_else(|| PricingError::TimeZone(format!("unknown IANA time zone {name:?}")))?;
        Ok(Self { name: name.to_owned(), zone: Some(zone) })
    }

    /// UTC, for tests and for a Location whose zone is not known.
    #[must_use]
    pub fn utc() -> Self {
        Self { name: "UTC".to_owned(), zone: None }
    }

    /// The IANA name this zone was built from.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The UTC offset in effect at `instant`, in seconds.
    ///
    /// # Errors
    ///
    /// Returns [`PricingError::TimeZone`] when the instant is outside the range the zone's rules
    /// cover.
    pub fn offset_seconds_at(&self, instant: DateTime) -> Result<i32, PricingError> {
        let Some(zone) = self.zone else { return Ok(0) };
        zone.find_local_time_type(instant.unix_timestamp())
            .map(tz::LocalTimeType::ut_offset)
            .map_err(|e| PricingError::TimeZone(format!("{}: {e}", self.name)))
    }

    /// `instant` as a wall-clock time in this zone.
    ///
    /// # Errors
    ///
    /// Returns [`PricingError::TimeZone`] when the offset cannot be determined.
    pub fn to_local(&self, instant: DateTime) -> Result<time::OffsetDateTime, PricingError> {
        let offset = self.offset_seconds_at(instant)?;
        Ok(instant.as_offset_date_time() + time::Duration::seconds(i64::from(offset)))
    }
}

impl fmt::Debug for TimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TimeZone({})", self.name)
    }
}

impl PartialEq for TimeZone {
    fn eq(&self, other: &Self) -> bool {
        self.name.eq_ignore_ascii_case(&other.name)
    }
}

/// Why a session could not be priced.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PricingError {
    /// No tariff was given at all.
    #[error("no tariff to price against")]
    NoTariff,
    /// A charging period named a `tariff_id` that was not among the tariffs given.
    #[error("charging period refers to tariff {0:?}, which was not provided")]
    UnknownTariff(String),
    /// None of the tariffs given was valid at the moment in question.
    #[error("no tariff is active at {0}")]
    NoActiveTariff(DateTime),
    /// The Location's time zone could not be resolved.
    #[error("time zone: {0}")]
    TimeZone(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Number;
    use crate::v2_3_0::tariffs::{
        PriceComponent, PriceLimit, Tariff, TariffDimensionType, TariffElement, TariffRestrictions,
        TaxIncluded,
    };

    fn n(s: &str) -> Number {
        s.parse().unwrap()
    }
    fn dt(s: &str) -> DateTime {
        s.parse().unwrap()
    }

    /// Minutes as a fraction of an hour, the unit OCPI measures time in.
    fn minutes(count: u32) -> Number {
        Number::from(count) / Number::from(60u32)
    }

    /// Compares two quantities at the precision a [`CostBreakdown`] reports them to.
    ///
    /// Durations in hours are repeating decimals — 35 minutes is 0.58333… — and the breakdown
    /// reports them rounded, so an exact comparison would be asserting on digits the artefact
    /// deliberately does not carry. See `PricingPolicy::quantity_decimals`.
    #[track_caller]
    fn assert_close(actual: Number, expected: Number) {
        let dp = PricingPolicy::default().quantity_decimals;
        assert_eq!(actual.round_dp(dp), expected.round_dp(dp), "expected {expected}, got {actual}");
    }

    fn component(
        dimension: TariffDimensionType,
        price: &str,
        vat: Option<&str>,
        step_size: u32,
    ) -> PriceComponent {
        PriceComponent {
            component_type: dimension,
            price: n(price),
            vat: vat.map(n),
            step_size,
            extensions: crate::types::Extensions::new(),
        }
    }

    fn tariff(elements: Vec<TariffElement>) -> Tariff {
        Tariff::builder()
            .country_code("DE")
            .party_id("ALL")
            .id("1")
            .currency("EUR")
            .elements(elements)
            .tax_included(TaxIncluded::No)
            .last_updated(dt("2015-06-29T20:39:09Z"))
            .build()
    }

    fn element(components: Vec<PriceComponent>) -> TariffElement {
        TariffElement::builder().price_components(components).build()
    }

    fn restricted(components: Vec<PriceComponent>, r: TariffRestrictions) -> TariffElement {
        TariffElement::builder().price_components(components).restrictions(r).build()
    }

    // ---------------------------------------------------------------------------------------
    // The specification's own worked examples.
    // ---------------------------------------------------------------------------------------

    #[test]
    fn spec_example_simple_025_per_kwh() {
        // "This tariff will result in costs of € 5.00 (excl. VAT) or € 5.50 (incl. VAT) when
        //  20 kWh are charged."
        let t = tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.25", Some("10.0"), 1)])]);
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("20"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("5.00"));
        assert_eq!(b.total_incl_vat, n("5.50"));
    }

    #[test]
    fn spec_example_energy_plus_start_fee() {
        // "Start fee € 0.50 excl. VAT with 20% VAT; energy € 0.25/kWh with 10% VAT.
        //  20 kWh → € 5.50 (excl. VAT) or € 6.10 (incl. VAT)."
        let t = tariff(vec![element(vec![
            component(TariffDimensionType::Flat, "0.50", Some("20.0"), 1),
            component(TariffDimensionType::Energy, "0.25", Some("10.0"), 1),
        ])]);
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("20"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("5.50"));
        assert_eq!(b.total_incl_vat, n("6.10"));
        assert_eq!(b.dimension_total(TariffDimensionType::Flat), n("0.50"));
    }

    #[test]
    fn spec_example_flat_fee_is_charged_once_for_the_whole_session() {
        let t = tariff(vec![element(vec![
            component(TariffDimensionType::Flat, "0.50", None, 1),
            component(TariffDimensionType::Energy, "0.25", None, 1),
        ])]);
        let session = PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc())
            .with_period(PricedPeriod {
                energy_kwh: n("10"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            })
            .with_period(PricedPeriod {
                energy_kwh: n("10"),
                ..PricedPeriod::new(dt("2024-01-15T11:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.dimension_total(TariffDimensionType::Flat), n("0.50"), "not 1.00");
        assert_eq!(b.total_excl_vat, n("5.50"));
    }

    #[test]
    fn spec_example_energy_step_size_rounds_the_session_total_once() {
        // "Energy costs € 0.20 per kWh before 17:00 and € 0.27 per kWh after 17:00. Both Price
        //  Components have a step_size of 500 Wh. If a driver charges 4.3 kWh before 17:00 and
        //  1.1 kWh after 17:00, a total of 5.4 kWh is charged. The step_size rounds this up to
        //  5.5 kWh total. It does NOT round the energy used after 17:00 to 1.5 kWh."
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Energy, "0.20", None, 500)],
                TariffRestrictions { end_time: Some("17:00".parse().unwrap()), ..Default::default() },
            ),
            element(vec![component(TariffDimensionType::Energy, "0.27", None, 500)]),
        ]);
        let session = PricedSession::new(dt("2024-01-15T15:00:00Z"), TimeZone::utc())
            .with_period(PricedPeriod {
                energy_kwh: n("4.3"),
                ..PricedPeriod::new(dt("2024-01-15T15:00:00Z"))
            })
            .with_period(PricedPeriod {
                energy_kwh: n("1.1"),
                ..PricedPeriod::new(dt("2024-01-15T17:30:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        let energy = b.dimension(TariffDimensionType::Energy).unwrap();
        assert_eq!(energy.measured, n("5.4"));
        assert_eq!(energy.billed, n("5.5"), "the session total is rounded, once");
        // 4.3 @ 0.20 + 1.2 @ 0.27 = 0.86 + 0.324 = 1.184
        assert_eq!(energy.segments[1].quantity, n("1.2"), "the surplus lands in the last segment");
        assert_eq!(b.total_excl_vat, n("1.18"));
    }

    #[test]
    fn charging_time_in_a_reserved_period_is_not_billed_at_the_reservation_rate() {
        // A ChargingPeriod may carry TIME and RESERVATION_TIME at once. The two are priced by
        // different Tariff Elements — one restricted with `reservation`, one not — so treating
        // the whole period as "a reservation" would bill the charging minutes at the
        // reservation rate, which is usually the more expensive of the two.
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Time, "4.00", None, 1)],
                TariffRestrictions {
                    reservation: Some(crate::v2_3_0::tariffs::ReservationRestrictionType::Reservation),
                    ..Default::default()
                },
            ),
            element(vec![component(TariffDimensionType::Time, "1.00", None, 1)]),
        ]);
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                charging_hours: n("1"),
                reservation_hours: n("0.5"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });

        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        // 1 h charging @ 1.00 + 0.5 h reserved @ 4.00 = 3.00, not 1.5 h @ 4.00 = 6.00.
        assert_eq!(b.total_excl_vat, n("3.00"));

        let time = b.dimension(TariffDimensionType::Time).unwrap();
        assert_eq!(time.segments.len(), 2, "the two are separate segments in the audit trail");
        assert_eq!(time.segments[0].price, n("1.00"));
        assert_eq!(time.segments[1].price, n("4.00"));
        // They still share the TIME dimension, and so the one step_size budget the spec allows.
        assert_eq!(time.measured, n("1.5"));
    }

    /// Every breakdown this engine produces must be internally consistent.
    ///
    /// The tax lines are what a party files and what a partner checks; if they do not account for
    /// the difference between the two totals, the document is not evidence of anything.
    fn assert_taxes_add_up(b: &CostBreakdown) {
        let summed: Number = b.taxes.iter().map(|t| t.amount).sum();
        assert_eq!(
            summed,
            b.total_incl_vat - b.total_excl_vat,
            "tax lines {:?} do not account for {} - {}",
            b.taxes,
            b.total_incl_vat,
            b.total_excl_vat,
        );
    }

    #[test]
    fn a_minimum_price_carries_its_tax_with_it() {
        // A €0.50 session with 21% VAT under a €5.00 minimum. Charging €5.00 net and remitting no
        // VAT is not a thing a party may do, and tax lines describing the €0.50 that was actually
        // metered would not add up to the totals printed beside them.
        let mut t =
            tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.25", Some("21"), 1)])]);
        t.min_price = Some(PriceLimit {
            before_taxes: n("5.00"),
            after_taxes: None,
            extensions: crate::types::Extensions::new(),
        });
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("2"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });

        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("5.00"));
        assert_eq!(b.total_incl_vat, n("6.05"), "21% of the clamped base, not of the metered one");
        assert_taxes_add_up(&b);
        assert_eq!(b.taxes.len(), 1);
        assert_eq!(b.taxes[0].percentage, Some(n("21")));
        assert_eq!(b.limit_applied, Some(PriceLimitApplied::Minimum));
        assert_eq!(b.notes_with(PricingNoteCode::TotalClamped).count(), 1);
    }

    #[test]
    fn a_maximum_price_carries_its_tax_with_it_too() {
        let mut t =
            tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.25", Some("20"), 1)])]);
        t.max_price = Some(PriceLimit {
            before_taxes: n("10.00"),
            after_taxes: None,
            extensions: crate::types::Extensions::new(),
        });
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("100"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });

        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("10.00"));
        assert_eq!(b.total_incl_vat, n("12.00"));
        assert_taxes_add_up(&b);
    }

    #[test]
    fn several_vat_rates_keep_their_proportions_through_a_clamp() {
        // A start fee at one rate and energy at another: the clamp must not silently pick one.
        let mut t = tariff(vec![element(vec![
            component(TariffDimensionType::Flat, "1.00", Some("20"), 0),
            component(TariffDimensionType::Energy, "0.25", Some("10"), 1),
        ])]);
        t.max_price = Some(PriceLimit {
            before_taxes: n("3.00"),
            after_taxes: None,
            extensions: crate::types::Extensions::new(),
        });
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("40"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });

        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("3.00"));
        assert_taxes_add_up(&b);
        assert_eq!(b.taxes.len(), 2, "both rates survive the clamp: {:?}", b.taxes);
        assert!(b.taxes.iter().all(|t| t.percentage.is_some()));
    }

    #[test]
    fn tax_that_no_rate_explains_is_reported_rather_than_invented() {
        // A `min_price.after_taxes` above a session whose components named no VAT at all. The
        // amount is a fact; the rate is not knowable, and making one up would be a lie in a
        // document somebody files.
        let mut t = tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.25", None, 1)])]);
        t.min_price = Some(PriceLimit {
            before_taxes: n("5.00"),
            after_taxes: Some(n("6.00")),
            extensions: crate::types::Extensions::new(),
        });
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("2"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });

        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!((b.total_excl_vat, b.total_incl_vat), (n("5.00"), n("6.00")));
        assert_taxes_add_up(&b);
        assert_eq!(b.taxes.len(), 1);
        assert_eq!(b.taxes[0].percentage, None, "the amount is known, the rate is not");
        assert_eq!(b.notes_with(PricingNoteCode::UnattributedTax).count(), 1);
    }

    #[test]
    fn a_charging_period_that_outlasts_its_price_is_reported() {
        // "A CPO SHALL at least start (and add) a ChargingPeriod every moment/event that has
        //  relevance for the total costs of a CDR. … When an energy changes in price after 17:00.
        //  The CPO has to start a new Charging Period at 17:00."
        //
        // One period 16:50 -> 17:30 with 10 kWh: the CPO should have split it at 17:00. Nothing
        // in the period says how the energy divides, so it is billed at the earlier rate — and
        // the reader is told, because that is the finding a reconciliation exists to produce.
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Energy, "0.40", None, 1)],
                TariffRestrictions { start_time: Some("17:00".parse().unwrap()), ..Default::default() },
            ),
            element(vec![component(TariffDimensionType::Energy, "0.20", None, 1)]),
        ]);
        let session = PricedSession::new(dt("2024-01-15T16:50:00Z"), TimeZone::utc())
            .with_period(PricedPeriod {
                energy_kwh: n("10"),
                ..PricedPeriod::new(dt("2024-01-15T16:50:00Z"))
            })
            .ending(dt("2024-01-15T17:30:00Z"));

        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("2.00"), "billed at the rate that applied when it began");
        assert!(b.needs_review());
        let spans: Vec<_> = b.notes_with(PricingNoteCode::PeriodSpansPriceChange).collect();
        assert_eq!(spans.len(), 1, "{:?}", b.notes);
        assert_eq!(spans[0].at, Some(dt("2024-01-15T16:50:00Z")));
        assert!(spans[0].message.contains("ENERGY"), "{}", spans[0].message);
    }

    #[test]
    fn a_period_that_ends_exactly_on_the_boundary_is_not_reported() {
        // A period is half-open. One running up to exactly 17:00 does not span 17:00, and a false
        // positive here would train everybody to ignore the note.
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Energy, "0.40", None, 1)],
                TariffRestrictions { start_time: Some("17:00".parse().unwrap()), ..Default::default() },
            ),
            element(vec![component(TariffDimensionType::Energy, "0.20", None, 1)]),
        ]);
        let session = PricedSession::new(dt("2024-01-15T16:50:00Z"), TimeZone::utc())
            .with_period(PricedPeriod { energy_kwh: n("5"), ..PricedPeriod::new(dt("2024-01-15T16:50:00Z")) })
            .with_period(PricedPeriod { energy_kwh: n("5"), ..PricedPeriod::new(dt("2024-01-15T17:00:00Z")) })
            .ending(dt("2024-01-15T17:30:00Z"));

        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("3.00"), "5 kWh at 0.20 then 5 at 0.40");
        assert!(!b.needs_review(), "a well-formed CDR needs no review: {:?}", b.notes);
    }

    #[test]
    fn periods_given_out_of_order_are_reported() {
        // Charging periods arrive from a CSMS through a CPO's own aggregation, and a merge that
        // loses the sort is invisible in every field-by-field check: every period is valid.
        let t = tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.25", None, 1)])]);
        let session = PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc())
            .with_period(PricedPeriod { energy_kwh: n("1"), ..PricedPeriod::new(dt("2024-01-15T11:00:00Z")) })
            .with_period(PricedPeriod {
                energy_kwh: n("1"),
                ..PricedPeriod::new(dt("2024-01-15T10:30:00Z"))
            });

        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("0.50"), "the quantities are all there, so it still prices");
        let out_of_order: Vec<_> = b.notes_with(PricingNoteCode::PeriodsOutOfOrder).collect();
        assert_eq!(out_of_order.len(), 1, "{:?}", b.notes);
        assert_eq!(out_of_order[0].at, Some(dt("2024-01-15T10:30:00Z")));
    }

    #[test]
    fn a_tariff_that_describes_negative_tax_does_not_produce_a_negative_bill() {
        // A VAT percentage below zero is malformed — `Tariff::validate` says so — but the engine
        // does not require validated input, and "costs less with tax than without" is not a
        // document anybody can use.
        let t = tariff(vec![element(vec![component(TariffDimensionType::Energy, "1.00", Some("-20"), 1)])]);
        assert!(crate::types::Validate::validate(&t).is_err(), "the tariff is reported as malformed");

        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("10"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();

        assert_eq!(b.total_excl_vat, n("10.00"));
        assert_eq!(b.total_incl_vat, n("10.00"), "held at the exclusive total, not 8.00");
        assert_eq!(b.total_vat(), Number::ZERO);
        assert_eq!(b.notes_with(PricingNoteCode::NegativeTax).count(), 1, "{:?}", b.notes);
    }

    #[test]
    fn a_breakdown_survives_being_written_down() {
        // A cost breakdown is an audit artefact: it gets stored, sent to a partner, and shown to
        // a driver disputing an invoice. If a quantity in it does not survive a JSON round-trip,
        // the copy the driver sees is not the one the engine computed.
        let t = tariff(vec![element(vec![
            component(TariffDimensionType::Time, "1.00", Some("21"), 600),
            component(TariffDimensionType::ParkingTime, "2.00", Some("21"), 600),
            component(TariffDimensionType::Energy, "0.25", Some("21"), 1),
        ])]);
        let session = PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc())
            .with_period(PricedPeriod {
                charging_hours: minutes(7),
                energy_kwh: n("1") / n("3"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            })
            .with_period(PricedPeriod {
                parking_hours: minutes(16),
                ..PricedPeriod::new(dt("2024-01-15T10:07:00Z"))
            });
        let breakdown = PricingEngine::new().price(&session, &[t]).unwrap();

        for dimension in &breakdown.dimensions {
            assert!(
                dimension.measured.json_round_trips(),
                "{} measured {}",
                dimension.dimension,
                dimension.measured
            );
            assert!(
                dimension.billed.json_round_trips(),
                "{} billed {}",
                dimension.dimension,
                dimension.billed
            );
            for segment in &dimension.segments {
                assert!(segment.quantity.json_round_trips(), "quantity {}", segment.quantity);
                assert!(segment.cost.json_round_trips(), "cost {}", segment.cost);
            }
        }
        let json = serde_json::to_string(&breakdown).unwrap();
        let back: CostBreakdown = serde_json::from_str(&json).unwrap();
        assert_eq!(back, breakdown, "the stored copy is the computed one");
    }

    #[test]
    fn spec_example_parking_absorbs_the_time_rounding() {
        // "Time spent charging costs € 1.00 per hour and time spent parking € 2.00 per hour.
        //  Both have a step_size of 10 minutes. If a driver charges 21 minutes, and keeps his EV
        //  connected while it is full for another 16 minutes, then the step_size rounds the
        //  parking duration up to 20 minutes … Note that the charging duration is not rounded up,
        //  as it is followed by another time based period."
        let t = tariff(vec![element(vec![
            component(TariffDimensionType::Time, "1.00", None, 600),
            component(TariffDimensionType::ParkingTime, "2.00", None, 600),
        ])]);
        let session = PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc())
            .with_period(PricedPeriod {
                charging_hours: minutes(21),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            })
            .with_period(PricedPeriod {
                parking_hours: minutes(16),
                ..PricedPeriod::new(dt("2024-01-15T10:21:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        let charging = b.dimension(TariffDimensionType::Time).unwrap();
        let parking = b.dimension(TariffDimensionType::ParkingTime).unwrap();
        assert_eq!(charging.billed, charging.measured, "charging is not rounded");
        assert_close(parking.billed, minutes(20));
        // 21/60 * 1.00 + 20/60 * 2.00 = 0.35 + 0.6667
        assert_eq!(b.total_excl_vat, n("1.02"));
    }

    #[test]
    fn spec_example_time_alone_is_rounded_with_the_last_step_size() {
        // "An EV driver plugs in at 16:35 and charges for 35 minutes. … the total charging time is
        //  rounded up from 35 to 45 minutes. … 25 minutes @ 1.20/h = 0.50, 20 minutes @ 2.40/h =
        //  0.80. Total 1.30."
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Time, "1.20", None, 1800)],
                TariffRestrictions { end_time: Some("17:00".parse().unwrap()), ..Default::default() },
            ),
            element(vec![component(TariffDimensionType::Time, "2.40", None, 900)]),
        ]);
        let session = PricedSession::new(dt("2024-01-15T16:35:00Z"), TimeZone::utc())
            .with_period(PricedPeriod {
                charging_hours: minutes(25),
                ..PricedPeriod::new(dt("2024-01-15T16:35:00Z"))
            })
            .with_period(PricedPeriod {
                charging_hours: minutes(10),
                ..PricedPeriod::new(dt("2024-01-15T17:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        let time = b.dimension(TariffDimensionType::Time).unwrap();
        assert_close(time.measured, minutes(35));
        assert_close(time.billed, minutes(45));
        assert_close(time.segments[1].quantity, minutes(20));
        assert_eq!(b.total_excl_vat, n("1.30"));
    }

    #[test]
    fn spec_example_min_price_raises_a_cheap_session() {
        // "if less than 2 kWh is charged, € 0.50 (excl. VAT) or € 0.55 (incl. VAT) will be billed."
        let mut t =
            tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.25", Some("10.0"), 1)])]);
        t.min_price = Some(PriceLimit {
            before_taxes: n("0.50"),
            after_taxes: Some(n("0.55")),
            extensions: crate::types::Extensions::new(),
        });
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("1"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t.clone()]).unwrap();
        assert_eq!(b.total_excl_vat, n("0.50"));
        assert_eq!(b.total_incl_vat, n("0.55"));
        assert_eq!(b.limit_applied, Some(PriceLimitApplied::Minimum));

        // 20 kWh is above the minimum, so it is billed normally.
        let big = PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
            energy_kwh: n("20"),
            ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
        });
        let b = PricingEngine::new().price(&big, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("5.00"));
        assert_eq!(b.limit_applied, None);
    }

    #[test]
    fn spec_example_max_price_caps_an_expensive_session() {
        // "For a charging session where 50 kWh are charged, this tariff will result in costs of
        //  € 10.00 (excl. VAT) or € 11.00 (incl. VAT) due to the price limit."
        let mut t = tariff(vec![element(vec![
            component(TariffDimensionType::Flat, "0.50", Some("20.0"), 1),
            component(TariffDimensionType::Energy, "0.25", Some("10.0"), 1),
        ])]);
        t.max_price = Some(PriceLimit {
            before_taxes: n("10.00"),
            after_taxes: Some(n("11.00")),
            extensions: crate::types::Extensions::new(),
        });
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("50"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t.clone()]).unwrap();
        assert_eq!(b.total_excl_vat, n("10.00"));
        assert_eq!(b.total_incl_vat, n("11.00"));
        assert_eq!(b.limit_applied, Some(PriceLimitApplied::Maximum));

        // "If only 30 kWh were charged, the costs would be € 8.00 (excl. VAT) and € 8.85 (incl.)"
        let smaller =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("30"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&smaller, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("8.00"));
        assert_eq!(b.total_incl_vat, n("8.85"));
    }

    #[test]
    fn spec_example_max_power_restrictions_switch_the_rate() {
        // "1 kWh at 6 kW: € 0.20; 40 kWh at 48 kW: € 20.00; 0.5 kWh at 4 kW: € 0.10 → € 20.30"
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Energy, "0.20", None, 1)],
                TariffRestrictions { max_power: Some(n("16")), ..Default::default() },
            ),
            restricted(
                vec![component(TariffDimensionType::Energy, "0.35", None, 1)],
                TariffRestrictions { max_power: Some(n("32")), ..Default::default() },
            ),
            element(vec![component(TariffDimensionType::Energy, "0.50", None, 1)]),
        ]);
        let period = |start: &str, kwh: &str, power: &str| PricedPeriod {
            energy_kwh: n(kwh),
            max_power_kw: Some(n(power)),
            min_power_kw: Some(n(power)),
            ..PricedPeriod::new(dt(start))
        };
        let session = PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc())
            .with_period(period("2024-01-15T10:00:00Z", "1", "6"))
            .with_period(period("2024-01-15T10:10:00Z", "40", "48"))
            .with_period(period("2024-01-15T11:00:00Z", "0.5", "4"));
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("20.30"));
    }

    #[test]
    fn spec_example_max_duration_makes_the_first_half_hour_free() {
        // "First 30 minutes of charging is free; € 0.25/kWh after 30 minutes; € 0.40/kWh after 60.
        //  5 kWh free + 1.2 kWh at 0.25 = € 0.30."
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Energy, "0.00", None, 1)],
                TariffRestrictions { max_duration: Some(1800), ..Default::default() },
            ),
            restricted(
                vec![component(TariffDimensionType::Energy, "0.25", None, 1)],
                TariffRestrictions { max_duration: Some(3600), ..Default::default() },
            ),
            element(vec![component(TariffDimensionType::Energy, "0.40", None, 1)]),
        ]);
        let session = PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc())
            .with_period(PricedPeriod { energy_kwh: n("5"), ..PricedPeriod::new(dt("2024-01-15T10:00:00Z")) })
            .with_period(PricedPeriod {
                energy_kwh: n("1.2"),
                ..PricedPeriod::new(dt("2024-01-15T10:30:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("0.30"));
    }

    #[test]
    fn a_reservation_is_priced_by_its_own_tariff_element() {
        // "Reservation € 5.00 per hour (excl. VAT) billed per minute; start fee € 0.50;
        //  energy € 0.25/kWh. A session started 15 minutes after the reservation, 20 kWh:
        //  € 6.75 excl. VAT."
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Time, "5.00", Some("20.0"), 60)],
                TariffRestrictions {
                    reservation: Some(crate::v2_3_0::tariffs::ReservationRestrictionType::Reservation),
                    ..Default::default()
                },
            ),
            element(vec![
                component(TariffDimensionType::Flat, "0.50", Some("20.0"), 1),
                component(TariffDimensionType::Energy, "0.25", Some("10.0"), 1),
            ]),
        ]);
        let session = PricedSession::new(dt("2024-01-15T09:45:00Z"), TimeZone::utc())
            .with_period(PricedPeriod {
                reservation_hours: minutes(15),
                ..PricedPeriod::new(dt("2024-01-15T09:45:00Z"))
            })
            .with_period(PricedPeriod {
                energy_kwh: n("20"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, n("6.75"));
        assert_eq!(b.total_incl_vat, n("7.60"));
    }

    #[test]
    fn a_free_of_charge_tariff_costs_nothing() {
        let t = tariff(vec![element(vec![component(TariffDimensionType::Flat, "0.00", None, 0)])]);
        assert!(t.is_free_of_charge());
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("20"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        assert_eq!(b.total_excl_vat, Number::ZERO);
        assert!(!b.notes.is_empty(), "the unpriced ENERGY is surfaced as a note");
    }

    #[test]
    fn local_time_restrictions_follow_the_locations_time_zone() {
        // 17:00 in Berlin is 15:00 UTC in summer and 16:00 UTC in winter. A session at 15:30 UTC
        // in July is after 17:00 local, and the same UTC time in January is not.
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Energy, "0.20", None, 1)],
                TariffRestrictions { end_time: Some("17:00".parse().unwrap()), ..Default::default() },
            ),
            element(vec![component(TariffDimensionType::Energy, "0.40", None, 1)]),
        ]);
        let berlin = TimeZone::named("Europe/Berlin").unwrap();
        let price_at = |instant: &str| {
            let session = PricedSession::new(dt(instant), berlin.clone())
                .with_period(PricedPeriod { energy_kwh: n("1"), ..PricedPeriod::new(dt(instant)) });
            PricingEngine::new().price(&session, std::slice::from_ref(&t)).unwrap().total_excl_vat
        };
        assert_eq!(price_at("2024-07-15T15:30:00Z"), n("0.40"), "17:30 CEST is after 17:00");
        assert_eq!(price_at("2024-01-15T15:30:00Z"), n("0.20"), "16:30 CET is before 17:00");
    }

    #[test]
    fn day_of_week_restrictions_use_local_days() {
        use crate::v2_3_0::tariffs::DayOfWeek;
        let t = tariff(vec![
            restricted(
                vec![component(TariffDimensionType::Energy, "0.10", None, 1)],
                TariffRestrictions {
                    day_of_week: vec![DayOfWeek::Saturday, DayOfWeek::Sunday],
                    ..Default::default()
                },
            ),
            element(vec![component(TariffDimensionType::Energy, "0.30", None, 1)]),
        ]);
        let price_on = |instant: &str| {
            let session = PricedSession::new(dt(instant), TimeZone::utc())
                .with_period(PricedPeriod { energy_kwh: n("1"), ..PricedPeriod::new(dt(instant)) });
            PricingEngine::new().price(&session, std::slice::from_ref(&t)).unwrap().total_excl_vat
        };
        assert_eq!(price_on("2024-01-13T10:00:00Z"), n("0.10"), "a Saturday");
        assert_eq!(price_on("2024-01-15T10:00:00Z"), n("0.30"), "a Monday");
    }

    #[test]
    fn the_breakdown_names_the_component_that_priced_each_segment() {
        let t = tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.25", None, 1)])]);
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("20"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        let applied: Vec<_> = b.applied_components().collect();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].tariff_id, "1");
        assert_eq!(applied[0].element_index, 0);
        assert_eq!(applied[0].component_index, 0);
        assert!(applied[0].because.contains("local"), "{}", applied[0].because);
    }

    #[test]
    fn a_period_can_name_which_tariff_applies_to_it() {
        let cheap = Tariff {
            id: "cheap".parse().unwrap(),
            ..tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.10", None, 1)])])
        };
        let dear = Tariff {
            id: "dear".parse().unwrap(),
            ..tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.90", None, 1)])])
        };
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("1"),
                tariff_id: Some("dear".to_owned()),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[cheap.clone(), dear]).unwrap();
        assert_eq!(b.total_excl_vat, n("0.90"));

        let missing =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("1"),
                tariff_id: Some("nope".to_owned()),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        assert_eq!(
            PricingEngine::new().price(&missing, &[cheap]).unwrap_err(),
            PricingError::UnknownTariff("nope".to_owned())
        );
    }

    #[test]
    fn the_result_converts_to_a_price_of_either_version() {
        let t = tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.25", Some("10.0"), 1)])]);
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("20"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let b = PricingEngine::new().price(&session, &[t]).unwrap();
        let new = b.to_price_v2_3_0();
        assert_eq!(new.before_taxes, n("5.00"));
        assert_eq!(new.after_taxes(), n("5.50"));
        let old = b.to_price_v2_2_1();
        assert_eq!((old.excl_vat, old.incl_vat.unwrap()), (n("5.00"), n("5.50")));
    }

    #[test]
    fn disabling_step_size_bills_the_measured_quantity() {
        let t = tariff(vec![element(vec![component(TariffDimensionType::Energy, "0.25", None, 500)])]);
        let session =
            PricedSession::new(dt("2024-01-15T10:00:00Z"), TimeZone::utc()).with_period(PricedPeriod {
                energy_kwh: n("5.4"),
                ..PricedPeriod::new(dt("2024-01-15T10:00:00Z"))
            });
        let with_steps = PricingEngine::new().price(&session, std::slice::from_ref(&t)).unwrap();
        assert_eq!(with_steps.dimension(TariffDimensionType::Energy).unwrap().billed, n("5.5"));

        let ocpi_3_style = PricingEngine::with_policy(PricingPolicy::default().without_step_size());
        let exact = ocpi_3_style.price(&session, &[t]).unwrap();
        assert_eq!(exact.dimension(TariffDimensionType::Energy).unwrap().billed, n("5.4"));
        assert_eq!(exact.total_excl_vat, n("1.35"));
    }

    #[test]
    fn an_unknown_time_zone_is_an_error_not_a_silent_utc() {
        assert!(matches!(TimeZone::named("Mars/Olympus"), Err(PricingError::TimeZone(_))));
        assert_eq!(TimeZone::named("UTC").unwrap(), TimeZone::utc());
    }
}
