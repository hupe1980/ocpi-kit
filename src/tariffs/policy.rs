//! The decisions OCPI deliberately leaves open: rounding, quantisation, and currency precision.
//!
//! > *NOTE: There are no parameters related to price rounding in the Tariff object or any of its
//! > constituent objects. Nor does the specification text of this module give any requirements
//! > about how to do price rounding. The reason for this that price rounding has to be done
//! > according to rules and restrictions set by applicable laws, contracts between the parties
//! > using OCPI and the currency used. The OCPI specification stays out of these matters.*
//!
//! A pricing engine cannot stay out of them, so this crate makes them a **parameter** rather than
//! a hidden constant. [`PricingPolicy::default`] is a defensible starting point; a party with a
//! contract that says otherwise changes it and gets an auditable answer either way.

use rust_decimal::RoundingStrategy;

use crate::types::Number;

/// How and when to round money.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PricingPolicy {
    /// Decimal places each dimension's cost is rounded to before the totals are summed.
    ///
    /// Default 4, which is the precision the specification names for OCPI `number`s:
    /// *"Unless mentioned otherwise, numbers use 4 decimals."*
    pub component_decimals: u32,
    /// Decimal places the final total is rounded to.
    ///
    /// Default 2: the minor unit of the great majority of currencies. Set to 0 for JPY, 3 for
    /// KWD and the other three-decimal currencies.
    pub currency_decimals: u32,
    /// How a value exactly halfway between two representable amounts is resolved.
    ///
    /// Default [`RoundingStrategy::MidpointAwayFromZero`] — "round half up" — which is what
    /// invoicing legislation in most of Europe assumes.
    pub rounding: RoundingStrategy,
    /// Whether to apply `step_size` block billing.
    ///
    /// Default [`Quantisation::StepSize`]. OCPI 3.0 removes `step_size` in favour of
    /// full-precision metering, so this is a pluggable stage rather than a hard-wired one:
    ///
    /// > *NOTE: The `step_size` field is no longer present in OCPI 3.0. In OCPI 3.0, Parties are
    /// > advised to measure quantities as precise as required by calibration law and use the full
    /// > precision of such measurements in cost computation.*
    pub quantisation: Quantisation,
}

impl Default for PricingPolicy {
    fn default() -> Self {
        Self {
            component_decimals: 4,
            currency_decimals: 2,
            rounding: RoundingStrategy::MidpointAwayFromZero,
            quantisation: Quantisation::StepSize,
        }
    }
}

impl PricingPolicy {
    /// A policy for a currency with no minor unit, such as JPY.
    #[must_use]
    pub fn zero_decimal_currency() -> Self {
        Self { currency_decimals: 0, ..Self::default() }
    }

    /// A policy that ignores `step_size`, as OCPI 3.0 will.
    #[must_use]
    pub fn without_step_size(mut self) -> Self {
        self.quantisation = Quantisation::None;
        self
    }

    /// Rounds an intermediate per-dimension cost.
    #[must_use]
    pub fn round_component(&self, value: Number) -> Number {
        Number::new(value.get().round_dp_with_strategy(self.component_decimals, self.rounding))
    }

    /// Rounds a final, presentable amount.
    #[must_use]
    pub fn round_currency(&self, value: Number) -> Number {
        Number::new(value.get().round_dp_with_strategy(self.currency_decimals, self.rounding))
    }
}

/// Whether consumed quantities are billed in `step_size` blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Quantisation {
    /// Round each quantity up to the next multiple of the applicable `step_size`, as OCPI 2.x
    /// prescribes.
    StepSize,
    /// Bill the measured quantity exactly, as OCPI 3.0 will.
    None,
}

impl Quantisation {
    /// Rounds `quantity` up to the next multiple of `step_size` units.
    ///
    /// `unit_scale` converts the quantity into the unit `step_size` counts: 1000 for `ENERGY`
    /// (kWh measured, Wh counted) and 3600 for the time dimensions (hours measured, seconds
    /// counted).
    ///
    /// > *Consumed amounts are rounded up to the smallest multiple of `step_size` that is greater
    /// > than the consumed amount.*
    #[must_use]
    pub fn apply(self, quantity: Number, step_size: u32, unit_scale: u32) -> Number {
        // A `step_size` of 0 carries no meaning — that is the `FLAT` case, and the
        // specification's own free-of-charge example writes 0 there. A `step_size` of 1 does:
        // for `ENERGY` it means "billed per 1 Wh", so 115.2 Wh becomes 116 Wh.
        if self == Self::None || step_size == 0 {
            return quantity;
        }
        let scale = Number::from(unit_scale);
        let step = Number::from(step_size);
        let in_units = quantity * scale;
        let blocks = (in_units / step).get().ceil();
        Number::new(blocks) * step / scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Number {
        s.parse().unwrap()
    }

    #[test]
    fn energy_is_quantised_in_watt_hours() {
        // "If someone charges their EV with 115.2 Wh, then they are billed for 116 Wh"
        assert_eq!(Quantisation::StepSize.apply(n("0.1152"), 1, 1000), n("0.116"));
        // "When step_size = 25, then the same amount would be billed for 101 to 125 Wh"
        assert_eq!(Quantisation::StepSize.apply(n("0.1152"), 25, 1000), n("0.125"));
        // "When step_size = 500, then the same amount will be billed for 1 to 500 Wh"
        assert_eq!(Quantisation::StepSize.apply(n("0.1152"), 500, 1000), n("0.5"));
    }

    #[test]
    fn time_is_quantised_in_seconds() {
        // 8 minutes with a 300-second step is billed as 10 minutes.
        let eight_minutes = n("8") / n("60");
        let billed = Quantisation::StepSize.apply(eight_minutes, 300, 3600);
        assert_eq!(billed, n("10") / n("60"));
        // 5.4 kWh with a 500 Wh step becomes 5.5 kWh, the spec's own example.
        assert_eq!(Quantisation::StepSize.apply(n("5.4"), 500, 1000), n("5.5"));
    }

    #[test]
    fn an_exact_multiple_is_left_alone() {
        assert_eq!(Quantisation::StepSize.apply(n("5.5"), 500, 1000), n("5.5"));
        assert_eq!(Quantisation::StepSize.apply(n("2"), 1, 1000), n("2"));
    }

    #[test]
    fn disabling_quantisation_bills_the_measured_amount() {
        assert_eq!(Quantisation::None.apply(n("5.4"), 500, 1000), n("5.4"));
        let policy = PricingPolicy::default().without_step_size();
        assert_eq!(policy.quantisation, Quantisation::None);
    }

    #[test]
    fn rounding_half_goes_away_from_zero_by_default() {
        let p = PricingPolicy::default();
        assert_eq!(p.round_currency(n("2.005")), n("2.01"));
        assert_eq!(p.round_currency(n("-2.005")), n("-2.01"));
        assert_eq!(p.round_component(n("0.00005")), n("0.0001"));
        assert_eq!(PricingPolicy::zero_decimal_currency().round_currency(n("2.5")), n("3"));
    }
}
