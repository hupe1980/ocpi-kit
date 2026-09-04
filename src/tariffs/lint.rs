//! Lints for a Tariff: the mistakes that are legal, decode cleanly, and still bill the wrong
//! amount.
//!
//! [`Validate`](crate::types::Validate) answers *"does this object conform to the
//! specification?"*. That is a different question from *"does this tariff say what its author
//! meant?"*, and every finding here is a Tariff that passes the first and fails the second: an
//! element that can never be reached, a `step_size` that can never be applied, a restriction
//! window that is empty, a VAT percentage on a tariff that says no tax applies.
//!
//! None of these is an error. A CPO publishing one has a working, conformant Tariff that prices
//! sessions — just not the way the person who wrote it intended, and the way you find out is a
//! driver's complaint or a partner's dispute six weeks later.
//!
//! ```
//! use ocpi_kit::tariffs::{lint, TariffLintCode};
//! # use ocpi_kit::v2_3_0::tariffs::*;
//! # use ocpi_kit::types::DateTime;
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let tariff = Tariff::builder().country_code("DE").party_id("ALL").id("1").currency("EUR")
//! #   .elements(vec![
//! #       TariffElement::builder().price_components(vec![PriceComponent {
//! #           component_type: TariffDimensionType::Energy, price: "0.25".parse()?, vat: None,
//! #           step_size: 1, extensions: Default::default() }]).build(),
//! #       TariffElement::builder().price_components(vec![PriceComponent {
//! #           component_type: TariffDimensionType::Energy, price: "0.40".parse()?, vat: None,
//! #           step_size: 1, extensions: Default::default() }])
//! #        .restrictions(TariffRestrictions { min_kwh: Some("10".parse()?), ..Default::default() })
//! #        .build(),
//! #   ])
//! #   .tax_included(TaxIncluded::No).last_updated("2024-01-01T00:00:00Z".parse::<DateTime>()?).build();
//! // An unrestricted element first, a restricted one after it: the second can never apply.
//! let findings = lint(&tariff);
//! assert_eq!(findings[0].code, TariffLintCode::UnreachableElement);
//! # Ok(())
//! # }
//! ```
//!
//! Spec: 2.3.0 §mod_tariffs_tariff_object

use core::fmt;

use crate::types::Number;
use crate::v2_3_0::tariffs::{Tariff, TariffDimensionType, TariffElement, TariffRestrictions};

/// What kind of problem a [`TariffLint`] describes.
///
/// Machine-readable, for the same reason
/// [`PricingNoteCode`](super::PricingNoteCode) is: a party publishing a few thousand tariffs needs
/// to count these, not read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TariffLintCode {
    /// A later Tariff Element can never price a dimension, because an earlier one always does.
    ///
    /// > *"the first Tariff Element with a Price Component for that dimension in the list with
    /// > matching Tariff Restrictions will be used"*
    ///
    /// An element with no restrictions always matches, so anything after it that prices the same
    /// dimension is dead. This is the single most common way a tiered tariff is written wrongly:
    /// the fallback goes first and the tiers below it never apply.
    UnreachableElement,
    /// One Tariff Element prices the same dimension twice; only the first is ever used.
    DuplicateDimension,
    /// A `TariffRestrictions` object with no restriction set in it.
    ///
    /// It restricts nothing, so the element is a fallback that does not look like one.
    EmptyRestrictions,
    /// A restriction whose bounds exclude everything, so the element can never apply.
    ImpossibleRestriction,
    /// `start_time` equals `end_time`.
    ///
    /// The specification does not say what that means. This crate reads it as **the whole day**,
    /// because that is what the wrap-around rule produces with no special case — but an author
    /// who meant "never" has written the opposite of what they wanted, and one who meant "all
    /// day" can say so unambiguously by leaving both out.
    WholeDayWindow,
    /// A `step_size` of `0` on a dimension that has a unit.
    ///
    /// The specification never describes it, and this crate reads it as "no quantisation". If
    /// per-unit billing was meant, the value is `1`.
    ZeroStepSize,
    /// A `step_size` above 1 on a `FLAT` component, which has no unit to count.
    ///
    /// > *"FLAT: Flat fee without unit for `step_size`"*
    FlatStepSize,
    /// A `TIME` `step_size` that can never be applied, because the tariff also prices
    /// `PARKING_TIME`.
    ///
    /// > *"In the cases that `TIME` and `PARKING_TIME` Tariff Elements are both used, `step_size`
    /// > is only taken into account for the total parking duration."*
    ///
    /// So the number is there, it looks like it does something, and it does not.
    UnusedTimeStepSize,
    /// A reservation element prices a dimension a reservation cannot have.
    ///
    /// > *"A reservation can only have: FLAT and TIME TariffDimensions, where `TIME` is for the
    /// > duration of the reservation."*
    ReservationDimension,
    /// The tariff prices neither `ENERGY` nor `TIME`, so a charging session costs at most a flat
    /// fee however long it runs and however much it takes.
    NothingPricedByUse,
    /// `tax_included` is `N/A` and a Price Component names a VAT percentage anyway.
    ///
    /// The two statements contradict each other; the engine follows the Tariff-level one.
    TaxRateWithoutTax,
    /// `tax_included` is `YES` and no Price Component names a rate, so the tax inside the prices
    /// cannot be split out of them.
    ///
    /// Ordinary and intended under North American tax rules — and worth knowing, because it means
    /// no party can compute this tariff's pre-tax total from the tariff alone.
    TaxIncludedWithoutRate,
    /// A VAT percentage outside 0–100.
    ImplausibleVat,
    /// `min_price` is above `max_price`, so no session can satisfy both.
    PriceLimitsCross,
    /// The tariff's validity window is empty or already over.
    NeverActive,
}

impl TariffLintCode {
    /// A short, stable, machine-readable slug.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnreachableElement => "unreachable_element",
            Self::DuplicateDimension => "duplicate_dimension",
            Self::EmptyRestrictions => "empty_restrictions",
            Self::ImpossibleRestriction => "impossible_restriction",
            Self::WholeDayWindow => "whole_day_window",
            Self::ZeroStepSize => "zero_step_size",
            Self::FlatStepSize => "flat_step_size",
            Self::UnusedTimeStepSize => "unused_time_step_size",
            Self::ReservationDimension => "reservation_dimension",
            Self::NothingPricedByUse => "nothing_priced_by_use",
            Self::TaxRateWithoutTax => "tax_rate_without_tax",
            Self::TaxIncludedWithoutRate => "tax_included_without_rate",
            Self::ImplausibleVat => "implausible_vat",
            Self::PriceLimitsCross => "price_limits_cross",
            Self::NeverActive => "never_active",
        }
    }
}

impl fmt::Display for TariffLintCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One thing about a Tariff worth a second look.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TariffLint {
    /// What kind of finding this is.
    pub code: TariffLintCode,
    /// RFC 6901 JSON Pointer to the part of the Tariff it is about.
    pub pointer: String,
    /// The finding in words, including what the engine will do about it.
    pub message: String,
}

impl fmt::Display for TariffLint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.code, self.pointer, self.message)
    }
}

/// Every lint that applies to `tariff`, in document order.
///
/// Each finding is a Tariff that conforms, decodes and prices sessions — and does not do what
/// it looks like it does. See [`TariffLintCode`] for what each one means.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn lint(tariff: &Tariff) -> Vec<TariffLint> {
    use crate::v2_3_0::tariffs::TaxIncluded;

    let mut out: Vec<TariffLint> = Vec::new();
    // Which dimensions an unrestricted element has already claimed, and which element that was.
    let mut claimed: Vec<(TariffDimensionType, usize)> = Vec::new();
    let prices_parking = tariff
        .elements
        .iter()
        .flat_map(|e| e.price_components.iter())
        .any(|c| c.component_type == TariffDimensionType::ParkingTime);
    let mut prices_by_use = false;

    for (e, element) in tariff.elements.iter().enumerate() {
        let unrestricted = element.restrictions.as_ref().is_none_or(is_empty_restriction);
        let mut seen: Vec<TariffDimensionType> = Vec::new();

        for (c, component) in element.price_components.iter().enumerate() {
            let at = format!("/elements/{e}/price_components/{c}");
            let dimension = component.component_type;
            if matches!(dimension, TariffDimensionType::Energy | TariffDimensionType::Time) {
                prices_by_use = true;
            }

            if seen.contains(&dimension) {
                out.push(TariffLint {
                    code: TariffLintCode::DuplicateDimension,
                    pointer: at.clone(),
                    message: format!(
                        "this element already prices {dimension}; only the first Price Component \
                         for a dimension in an element is ever used"
                    ),
                });
            } else {
                seen.push(dimension);
            }

            if let Some((_, first)) = claimed.iter().find(|(d, _)| *d == dimension) {
                out.push(TariffLint {
                    code: TariffLintCode::UnreachableElement,
                    pointer: at.clone(),
                    message: format!(
                        "element {first} prices {dimension} and its restrictions always match, so \
                         this Price Component can never apply. The unrestricted element is the \
                         fallback and belongs last"
                    ),
                });
            }

            if dimension == TariffDimensionType::Flat {
                if component.step_size > 1 {
                    out.push(TariffLint {
                        code: TariffLintCode::FlatStepSize,
                        pointer: format!("{at}/step_size"),
                        message: format!(
                            "a FLAT fee has no unit to count in blocks of {}; the value is ignored",
                            component.step_size
                        ),
                    });
                }
            } else if component.step_size == 0 {
                out.push(TariffLint {
                    code: TariffLintCode::ZeroStepSize,
                    pointer: format!("{at}/step_size"),
                    message: format!(
                        "the specification never defines a {dimension} `step_size` of 0; this \
                         crate reads it as no quantisation at all. Write 1 for per-unit billing"
                    ),
                });
            } else if dimension == TariffDimensionType::Time && prices_parking && component.step_size > 1 {
                out.push(TariffLint {
                    code: TariffLintCode::UnusedTimeStepSize,
                    pointer: format!("{at}/step_size"),
                    message: format!(
                        "this tariff also prices PARKING_TIME, and `step_size` is then only taken \
                         into account for the total parking duration, so blocks of {} seconds of \
                         charging time will never be applied",
                        component.step_size
                    ),
                });
            }

            if let Some(vat) = component.vat {
                if vat.is_negative() || vat > Number::from(100u32) {
                    out.push(TariffLint {
                        code: TariffLintCode::ImplausibleVat,
                        pointer: format!("{at}/vat"),
                        message: format!("a VAT percentage of {vat} is outside 0-100"),
                    });
                }
                if tariff.tax_included == TaxIncluded::NotApplicable {
                    out.push(TariffLint {
                        code: TariffLintCode::TaxRateWithoutTax,
                        pointer: format!("{at}/vat"),
                        message: "the Tariff says no taxes are applicable and this component names \
                                  a rate; the Tariff-level statement governs and the rate is \
                                  ignored"
                            .to_owned(),
                    });
                }
            }
        }

        if let Some(restrictions) = element.restrictions.as_ref() {
            lint_restrictions(restrictions, e, element, &mut out);
        }

        if unrestricted {
            for dimension in seen {
                if !claimed.iter().any(|(d, _)| *d == dimension) {
                    claimed.push((dimension, e));
                }
            }
        }
    }

    if !prices_by_use {
        out.push(TariffLint {
            code: TariffLintCode::NothingPricedByUse,
            pointer: "/elements".to_owned(),
            message: "no Price Component prices ENERGY or TIME, so a session of any length and \
                      any amount of energy costs the same"
                .to_owned(),
        });
    }

    if tariff.tax_included == TaxIncluded::Yes
        && !tariff.elements.iter().flat_map(|e| e.price_components.iter()).any(|c| c.vat.is_some())
    {
        out.push(TariffLint {
            code: TariffLintCode::TaxIncludedWithoutRate,
            pointer: "/tax_included".to_owned(),
            message: "the prices include tax and no Price Component says at what rate, so nobody \
                      reading this Tariff can compute a pre-tax total from it"
                .to_owned(),
        });
    }

    if let (Some(min), Some(max)) = (tariff.min_price.as_ref(), tariff.max_price.as_ref()) {
        if min.before_taxes > max.before_taxes {
            out.push(TariffLint {
                code: TariffLintCode::PriceLimitsCross,
                pointer: "/min_price/before_taxes".to_owned(),
                message: format!(
                    "the minimum before taxes ({}) is above the maximum ({}), so no session can \
                     satisfy both and the maximum is applied last",
                    min.before_taxes, max.before_taxes
                ),
            });
        }
        if let (Some(min_after), Some(max_after)) = (min.after_taxes, max.after_taxes)
            && min_after > max_after
        {
            out.push(TariffLint {
                code: TariffLintCode::PriceLimitsCross,
                pointer: "/min_price/after_taxes".to_owned(),
                message: format!("the minimum after taxes ({min_after}) is above the maximum ({max_after})"),
            });
        }
    }

    if let (Some(start), Some(end)) = (tariff.start_date_time, tariff.end_date_time)
        && end <= start
    {
        out.push(TariffLint {
            code: TariffLintCode::NeverActive,
            pointer: "/end_date_time".to_owned(),
            message: format!("this Tariff ends at {end}, at or before it starts at {start}"),
        });
    }

    out
}

fn lint_restrictions(
    r: &TariffRestrictions,
    index: usize,
    element: &TariffElement,
    out: &mut Vec<TariffLint>,
) {
    let at = format!("/elements/{index}/restrictions");
    if is_empty_restriction(r) {
        out.push(TariffLint {
            code: TariffLintCode::EmptyRestrictions,
            pointer: at.clone(),
            message: "a TariffRestrictions with nothing set restricts nothing; this element is a \
                      fallback that does not read like one"
                .to_owned(),
        });
        return;
    }

    let mut impossible = |field: &str, message: String| {
        out.push(TariffLint {
            code: TariffLintCode::ImpossibleRestriction,
            pointer: format!("{at}/{field}"),
            message,
        });
    };

    if let (Some(min), Some(max)) = (r.min_kwh, r.max_kwh)
        && min >= max
    {
        impossible("min_kwh", format!("min_kwh {min} is at or above max_kwh {max}, which is exclusive"));
    }
    if let (Some(min), Some(max)) = (r.min_power, r.max_power)
        && min >= max
    {
        impossible("min_power", format!("min_power {min} is at or above max_power {max}"));
    }
    if let (Some(min), Some(max)) = (r.min_current, r.max_current)
        && min >= max
    {
        impossible("min_current", format!("min_current {min} is at or above max_current {max}"));
    }
    if let (Some(min), Some(max)) = (r.min_duration, r.max_duration)
        && min >= max
    {
        impossible(
            "min_duration",
            format!("min_duration {min}s is at or above max_duration {max}s, which is exclusive"),
        );
    }
    if let (Some(start), Some(end)) = (r.start_date, r.end_date)
        && end <= start
    {
        impossible("start_date", format!("valid from {start} until {end}, which is not a day"));
    }
    if let (Some(start), Some(end)) = (r.start_time, r.end_time)
        && start == end
    {
        out.push(TariffLint {
            code: TariffLintCode::WholeDayWindow,
            pointer: format!("{at}/start_time"),
            message: format!(
                "start_time and end_time are both {start}; the specification does not say what \
                 that means and this crate reads it as the whole day. Leave both out to say so"
            ),
        });
    }

    if r.reservation.is_some() {
        for (c, component) in element.price_components.iter().enumerate() {
            if !matches!(component.component_type, TariffDimensionType::Flat | TariffDimensionType::Time) {
                out.push(TariffLint {
                    code: TariffLintCode::ReservationDimension,
                    pointer: format!("/elements/{index}/price_components/{c}"),
                    message: format!(
                        "this element describes reservation costs, and a reservation can only \
                         have FLAT and TIME dimensions; {} will never be priced by it",
                        component.component_type
                    ),
                });
            }
        }
    }
}

fn is_empty_restriction(r: &TariffRestrictions) -> bool {
    r.start_time.is_none()
        && r.end_time.is_none()
        && r.start_date.is_none()
        && r.end_date.is_none()
        && r.min_kwh.is_none()
        && r.max_kwh.is_none()
        && r.min_current.is_none()
        && r.max_current.is_none()
        && r.min_power.is_none()
        && r.max_power.is_none()
        && r.min_duration.is_none()
        && r.max_duration.is_none()
        && r.day_of_week.is_empty()
        && r.reservation.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DateTime;
    use crate::v2_3_0::tariffs::{PriceComponent, TariffElement, TaxIncluded};

    fn n(s: &str) -> Number {
        s.parse().unwrap()
    }

    fn component(dimension: TariffDimensionType, price: &str, step_size: u32) -> PriceComponent {
        PriceComponent {
            component_type: dimension,
            price: n(price),
            vat: None,
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
            .last_updated("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .build()
    }

    fn codes(tariff: &Tariff) -> Vec<TariffLintCode> {
        lint(tariff).into_iter().map(|l| l.code).collect()
    }

    #[test]
    fn a_fallback_element_before_a_restricted_one_makes_the_restricted_one_dead() {
        let t = tariff(vec![
            TariffElement::builder()
                .price_components(vec![component(TariffDimensionType::Energy, "0.25", 1)])
                .build(),
            TariffElement::builder()
                .price_components(vec![component(TariffDimensionType::Energy, "0.40", 1)])
                .restrictions(TariffRestrictions { min_kwh: Some(n("10")), ..Default::default() })
                .build(),
        ]);
        assert!(codes(&t).contains(&TariffLintCode::UnreachableElement));

        // The same two elements the other way round are exactly right, and lint nothing.
        let ordered = tariff(vec![
            TariffElement::builder()
                .price_components(vec![component(TariffDimensionType::Energy, "0.40", 1)])
                .restrictions(TariffRestrictions { min_kwh: Some(n("10")), ..Default::default() })
                .build(),
            TariffElement::builder()
                .price_components(vec![component(TariffDimensionType::Energy, "0.25", 1)])
                .build(),
        ]);
        assert_eq!(lint(&ordered), vec![], "{:?}", lint(&ordered));
    }

    #[test]
    fn a_time_step_size_that_parking_will_absorb_is_reported() {
        let t = tariff(vec![
            TariffElement::builder()
                .price_components(vec![
                    component(TariffDimensionType::Time, "2.00", 300),
                    component(TariffDimensionType::ParkingTime, "1.00", 300),
                ])
                .build(),
        ]);
        assert!(codes(&t).contains(&TariffLintCode::UnusedTimeStepSize));
    }

    #[test]
    fn impossible_and_empty_restrictions_are_both_found() {
        let t = tariff(vec![
            TariffElement::builder()
                .price_components(vec![component(TariffDimensionType::Energy, "0.25", 1)])
                .restrictions(TariffRestrictions {
                    min_kwh: Some(n("50")),
                    max_kwh: Some(n("10")),
                    ..Default::default()
                })
                .build(),
            TariffElement::builder()
                .price_components(vec![component(TariffDimensionType::Time, "1.00", 1)])
                .restrictions(TariffRestrictions::default())
                .build(),
        ]);
        let found = codes(&t);
        assert!(found.contains(&TariffLintCode::ImpossibleRestriction));
        assert!(found.contains(&TariffLintCode::EmptyRestrictions));
    }

    #[test]
    fn a_reservation_element_that_prices_energy_is_reported() {
        let t = tariff(vec![
            TariffElement::builder()
                .price_components(vec![component(TariffDimensionType::Energy, "0.25", 1)])
                .restrictions(TariffRestrictions {
                    reservation: Some(crate::v2_3_0::tariffs::ReservationRestrictionType::Reservation),
                    ..Default::default()
                })
                .build(),
        ]);
        assert!(codes(&t).contains(&TariffLintCode::ReservationDimension));
    }

    #[test]
    fn a_tariff_that_only_charges_a_flat_fee_says_so() {
        let t = tariff(vec![
            TariffElement::builder()
                .price_components(vec![component(TariffDimensionType::Flat, "2.50", 0)])
                .build(),
        ]);
        assert!(codes(&t).contains(&TariffLintCode::NothingPricedByUse));
    }

    #[test]
    fn the_specifications_own_examples_are_clean() {
        // Whatever this linter reports, it must not report it about the tariffs the
        // specification itself publishes as models to copy.
        for name in [
            "tariff_1_simple_2hour",
            "tariff_8_simple_025kwh",
            "tariff_9_025kwh_start",
            "tariff_19_simple_north_american_exclusive",
        ] {
            let path = format!("fixtures/2.3.0/{name}.json");
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let tariff: Tariff = serde_json::from_str(&text).unwrap();
            assert_eq!(lint(&tariff), vec![], "{name} should lint clean");
        }
    }

    /// The specification's own € 3/hour + € 5/hour parking example carries a `step_size` its own
    /// normative sentence makes inert.
    ///
    /// > *"In the cases that `TIME` and `PARKING_TIME` Tariff Elements are both used, `step_size`
    /// > is only taken into account for the total parking duration."*
    ///
    /// `tariff_13` prices both and gives the `TIME` component a `step_size` of 60 anyway. It
    /// changes nothing in the worked example — the session charges for exactly 150 minutes — and
    /// it is the reason two implementations reading the two halves of that sentence differently
    /// can both pass the specification's own test and still disagree on a real session. Finding
    /// it in a published example is exactly what this lint is for.
    #[test]
    fn a_specification_example_carries_a_step_size_its_own_rule_makes_inert() {
        let Ok(text) = std::fs::read_to_string("fixtures/2.3.0/tariff_13_simple_3hour_5parking.json") else {
            return;
        };
        let tariff: Tariff = serde_json::from_str(&text).unwrap();
        assert_eq!(codes(&tariff), vec![TariffLintCode::UnusedTimeStepSize]);
    }

    #[test]
    fn the_north_american_inclusive_example_is_reported_as_unsplittable() {
        let Ok(text) =
            std::fs::read_to_string("fixtures/2.3.0/tariff_20_simple_north_american_inclusive.json")
        else {
            return;
        };
        let tariff: Tariff = serde_json::from_str(&text).unwrap();
        assert_eq!(codes(&tariff), vec![TariffLintCode::TaxIncludedWithoutRate]);
    }
}
