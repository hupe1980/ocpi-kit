//! The normalised view of a session that the pricing engine works on.
//!
//! A CDR and a Session carry the same information for pricing purposes, and OCPI 2.2.1 and 2.3.0
//! differ only in the shape of `Price`, which is an *output* of pricing rather than an input. So
//! the engine takes one [`PricedSession`], and each wire type knows how to produce one.

use crate::types::{DateTime, Number};

use super::TimeZone;

/// One charging period, reduced to what pricing needs.
#[derive(Clone, Debug, PartialEq)]
pub struct PricedPeriod {
    /// Start of the period. The period ends when the next one starts.
    pub start: DateTime,
    /// Energy charged during this period, in kWh.
    pub energy_kwh: Number,
    /// Time spent charging during this period, in hours.
    pub charging_hours: Number,
    /// Time spent parked and not charging during this period, in hours.
    pub parking_hours: Number,
    /// Time the EVSE was reserved during this period, in hours.
    pub reservation_hours: Number,
    /// The highest current drawn during this period, in A, if measured.
    pub max_current_a: Option<Number>,
    /// The lowest current drawn during this period, in A, if measured.
    pub min_current_a: Option<Number>,
    /// The highest power drawn during this period, in kW, if measured.
    pub max_power_kw: Option<Number>,
    /// The lowest power drawn during this period, in kW, if measured.
    pub min_power_kw: Option<Number>,
    /// The Tariff that applies to this period, when the CPO said which one.
    pub tariff_id: Option<String>,
}

impl PricedPeriod {
    /// A period with a start time and nothing consumed.
    #[must_use]
    pub fn new(start: DateTime) -> Self {
        Self {
            start,
            energy_kwh: Number::ZERO,
            charging_hours: Number::ZERO,
            parking_hours: Number::ZERO,
            reservation_hours: Number::ZERO,
            max_current_a: None,
            min_current_a: None,
            max_power_kw: None,
            min_power_kw: None,
            tariff_id: None,
        }
    }

    /// The current to evaluate a `min_current`/`max_current` restriction against.
    ///
    /// > *`min_current`: Sum of the minimum current (in Amperes) over all phases … When the EV is
    /// > charging with more than, or equal to, the defined amount of current, this TariffElement
    /// > is/becomes active.*
    ///
    /// The restrictions describe the current *during* the period, so the measured maximum is used
    /// for a lower bound and the measured minimum for an upper bound — the pair that makes the
    /// restriction hold for the whole period.
    #[must_use]
    pub fn current_for_lower_bound(&self) -> Option<Number> {
        self.max_current_a.or(self.min_current_a)
    }

    /// The current to evaluate an upper bound against. See
    /// [`current_for_lower_bound`](Self::current_for_lower_bound).
    #[must_use]
    pub fn current_for_upper_bound(&self) -> Option<Number> {
        self.min_current_a.or(self.max_current_a)
    }

    /// The power to evaluate a `min_power` restriction against, in kW.
    #[must_use]
    pub fn power_for_lower_bound(&self) -> Option<Number> {
        self.max_power_kw.or(self.min_power_kw)
    }

    /// The power to evaluate a `max_power` restriction against, in kW.
    #[must_use]
    pub fn power_for_upper_bound(&self) -> Option<Number> {
        self.min_power_kw.or(self.max_power_kw)
    }
}

/// A session reduced to what the pricing engine needs.
///
/// Build one with [`PricedSession::from_cdr`] or [`PricedSession::from_session`], or by hand for
/// a "what would this cost?" calculation that has no CDR yet.
#[derive(Clone, Debug, PartialEq)]
pub struct PricedSession {
    /// When the session started, in UTC.
    pub start: DateTime,
    /// When the session ended, in UTC, if it has.
    pub end: Option<DateTime>,
    /// The charging periods, in order.
    pub periods: Vec<PricedPeriod>,
    /// The time zone of the Location, which the local-time restrictions are expressed in.
    pub time_zone: TimeZone,
    /// The `ProfileType` the driver selected, which decides which `Tariff.type` applies.
    pub profile_type: Option<crate::v2_3_0::sessions::ProfileType>,
    /// Whether the driver used ad-hoc payment rather than a contract.
    pub ad_hoc_payment: bool,
    /// Whether a reservation that was made expired before charging started.
    ///
    /// Selects between the `RESERVATION` and `RESERVATION_EXPIRES` tariff elements.
    pub reservation_expired: bool,
}

impl PricedSession {
    /// A session with no periods, for building up by hand.
    #[must_use]
    pub fn new(start: DateTime, time_zone: TimeZone) -> Self {
        Self {
            start,
            end: None,
            periods: Vec::new(),
            time_zone,
            profile_type: None,
            ad_hoc_payment: false,
            reservation_expired: false,
        }
    }

    /// Adds a charging period.
    #[must_use]
    pub fn with_period(mut self, period: PricedPeriod) -> Self {
        self.periods.push(period);
        self
    }

    /// Sets the end of the session.
    #[must_use]
    pub const fn ending(mut self, end: DateTime) -> Self {
        self.end = Some(end);
        self
    }

    /// The total energy across all periods, in kWh.
    #[must_use]
    pub fn total_energy_kwh(&self) -> Number {
        self.periods.iter().map(|p| p.energy_kwh).sum()
    }

    /// The total charging time across all periods, in hours.
    #[must_use]
    pub fn total_charging_hours(&self) -> Number {
        self.periods.iter().map(|p| p.charging_hours).sum()
    }

    /// The total parking time across all periods, in hours.
    #[must_use]
    pub fn total_parking_hours(&self) -> Number {
        self.periods.iter().map(|p| p.parking_hours).sum()
    }

    /// The total reservation time across all periods, in hours.
    #[must_use]
    pub fn total_reservation_hours(&self) -> Number {
        self.periods.iter().map(|p| p.reservation_hours).sum()
    }

    /// The energy charged before `index`, for a `min_kwh`/`max_kwh` restriction.
    #[must_use]
    pub fn energy_before(&self, index: usize) -> Number {
        self.periods.iter().take(index).map(|p| p.energy_kwh).sum()
    }

    /// The session duration up to the start of period `index`, in seconds.
    ///
    /// > *`min_duration`: Minimum duration in seconds the Charging Session MUST last.*
    #[must_use]
    pub fn duration_before(&self, index: usize) -> i64 {
        self.periods.get(index).map_or(0, |p| p.start.unix_timestamp() - self.start.unix_timestamp())
    }

    /// The end of period `index`: the start of the next one, or the end of the session.
    #[must_use]
    pub fn period_end(&self, index: usize) -> Option<DateTime> {
        self.periods.get(index + 1).map(|p| p.start).or(self.end)
    }
}

#[cfg(feature = "v2_3_0")]
mod from_v2_3_0 {
    use super::{PricedPeriod, PricedSession};
    use crate::tariffs::TimeZone;
    use crate::types::Number;
    use crate::v2_3_0::cdrs::{Cdr, CdrDimensionType, ChargingPeriod};
    use crate::v2_3_0::sessions::Session;

    fn period_from(source: &ChargingPeriod) -> PricedPeriod {
        let volume = |t: CdrDimensionType| source.volume(t).unwrap_or(Number::ZERO);
        PricedPeriod {
            start: source.start_date_time,
            energy_kwh: volume(CdrDimensionType::Energy),
            charging_hours: volume(CdrDimensionType::Time),
            parking_hours: volume(CdrDimensionType::ParkingTime),
            reservation_hours: volume(CdrDimensionType::ReservationTime),
            max_current_a: source.volume(CdrDimensionType::MaxCurrent),
            min_current_a: source.volume(CdrDimensionType::MinCurrent),
            max_power_kw: source.volume(CdrDimensionType::MaxPower),
            min_power_kw: source.volume(CdrDimensionType::MinPower),
            tariff_id: source.tariff_id.as_ref().map(|t| t.as_str().to_owned()),
        }
    }

    impl PricedSession {
        /// Builds the pricing input from an OCPI 2.3.0 CDR.
        ///
        /// The CDR does not carry the Location's time zone — it is not one of the fields
        /// `CdrLocation` keeps — so it has to be supplied. Use the `time_zone` of the
        /// [`Location`](crate::v2_3_0::locations::Location) the session took place at.
        #[must_use]
        pub fn from_cdr(cdr: &Cdr, time_zone: TimeZone) -> Self {
            Self {
                start: cdr.start_date_time,
                end: Some(cdr.end_date_time),
                periods: cdr.charging_periods.iter().map(period_from).collect(),
                time_zone,
                profile_type: None,
                ad_hoc_payment: false,
                reservation_expired: false,
            }
        }

        /// Builds the pricing input from an OCPI 2.3.0 Session.
        #[must_use]
        pub fn from_session(session: &Session, time_zone: TimeZone) -> Self {
            Self {
                start: session.start_date_time,
                end: session.end_date_time,
                periods: session.charging_periods.iter().map(period_from).collect(),
                time_zone,
                profile_type: None,
                ad_hoc_payment: false,
                reservation_expired: false,
            }
        }
    }
}

#[cfg(feature = "v2_2_1")]
mod from_v2_2_1 {
    use super::{PricedPeriod, PricedSession};
    use crate::tariffs::TimeZone;
    use crate::types::Number;
    use crate::v2_2_1::cdrs::{Cdr, CdrDimensionType};
    use crate::v2_2_1::sessions::Session;

    impl PricedSession {
        /// Builds the pricing input from an OCPI 2.2.1 CDR.
        ///
        /// The charging period types are wire-identical between 2.2.1 and 2.3.0, so this reuses
        /// the same reduction.
        #[must_use]
        pub fn from_cdr_v2_2_1(cdr: &Cdr, time_zone: TimeZone) -> Self {
            let periods = cdr
                .charging_periods
                .iter()
                .map(|source| {
                    let volume = |t: CdrDimensionType| source.volume(t).unwrap_or(Number::ZERO);
                    PricedPeriod {
                        start: source.start_date_time,
                        energy_kwh: volume(CdrDimensionType::Energy),
                        charging_hours: volume(CdrDimensionType::Time),
                        parking_hours: volume(CdrDimensionType::ParkingTime),
                        reservation_hours: volume(CdrDimensionType::ReservationTime),
                        max_current_a: source.volume(CdrDimensionType::MaxCurrent),
                        min_current_a: source.volume(CdrDimensionType::MinCurrent),
                        max_power_kw: source.volume(CdrDimensionType::MaxPower),
                        min_power_kw: source.volume(CdrDimensionType::MinPower),
                        tariff_id: source.tariff_id.as_ref().map(|t| t.as_str().to_owned()),
                    }
                })
                .collect();
            Self {
                start: cdr.start_date_time,
                end: Some(cdr.end_date_time),
                periods,
                time_zone,
                profile_type: None,
                ad_hoc_payment: false,
                reservation_expired: false,
            }
        }

        /// Builds the pricing input from an OCPI 2.2.1 Session.
        #[must_use]
        pub fn from_session_v2_2_1(session: &Session, time_zone: TimeZone) -> Self {
            let mut out = Self::new(session.start_date_time, time_zone);
            out.end = session.end_date_time;
            out.periods = session
                .charging_periods
                .iter()
                .map(|source| {
                    let volume = |t: CdrDimensionType| source.volume(t).unwrap_or(Number::ZERO);
                    PricedPeriod {
                        start: source.start_date_time,
                        energy_kwh: volume(CdrDimensionType::Energy),
                        charging_hours: volume(CdrDimensionType::Time),
                        parking_hours: volume(CdrDimensionType::ParkingTime),
                        reservation_hours: volume(CdrDimensionType::ReservationTime),
                        max_current_a: source.volume(CdrDimensionType::MaxCurrent),
                        min_current_a: source.volume(CdrDimensionType::MinCurrent),
                        max_power_kw: source.volume(CdrDimensionType::MaxPower),
                        min_power_kw: source.volume(CdrDimensionType::MinPower),
                        tariff_id: source.tariff_id.as_ref().map(|t| t.as_str().to_owned()),
                    }
                })
                .collect();
            out
        }
    }
}
