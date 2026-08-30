//! The *Tariffs* module of OCPI 2.1.1.
//!
//! A 2.1.1 Tariff has no owner fields, no `type`, no price limits and no tax handling: a
//! [`PriceComponent`] here is *"price per unit (excluding VAT)"* with no `vat` field at all, so
//! VAT is simply not expressible. OCPI 2.2 added the per-component `vat`; 2.3.0 added the
//! Tariff-wide `tax_included`.
//!
//! Spec: 2.1.1 §mod_tariffs

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    Currency, DateTime, DisplayText, Extensions, LocalDate, LocalTime, Number, OcpiString, Url, Validate,
    Validator, ViolationCode,
};

use super::locations::EnergyMix;

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::tariffs::{DayOfWeek, TariffDimensionType};

/// A tariff, in OCPI 2.1.1.
///
/// Spec: 2.1.1 §mod_tariffs_tariff_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Tariff {
    /// Uniquely identifies the tariff within the CPO's platform.
    pub id: OcpiString<36>,
    /// Currency of this tariff, ISO 4217 code.
    pub currency: Currency,
    /// Multi-language alternative tariff info text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tariff_alt_text: Vec<DisplayText>,
    /// Alternative URL to tariff info.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tariff_alt_url: Option<Url>,
    /// The Tariff Elements. Cardinality `+`.
    pub elements: Vec<TariffElement>,
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
    /// Whether this is the "Free of Charge" shape: one unrestricted element with a single zero
    /// `FLAT` component.
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
            self,
            v,
            id,
            currency,
            tariff_alt_text,
            tariff_alt_url,
            elements,
            energy_mix,
            last_updated,
        );
        if self.elements.is_empty() {
            v.report_at(
                "elements",
                ViolationCode::EmptyRequiredList,
                "a Tariff has cardinality `+` elements: at least one is required",
            );
        }
    }
}

/// A group of [`PriceComponent`]s that share a set of restrictions.
///
/// Spec: 2.1.1 §mod_tariffs_tariffelement_class
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

impl Validate for TariffElement {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, price_components, restrictions);
        if self.price_components.is_empty() {
            v.report_at(
                "price_components",
                ViolationCode::EmptyRequiredList,
                "a TariffElement has cardinality `+` price_components",
            );
        }
    }
}

/// How consumption of one dimension translates into money owed, in OCPI 2.1.1.
///
/// **There is no `vat` field.** The price is always excluding VAT, and OCPI 2.1.1 provides no
/// way to say what the VAT is; that arrived in OCPI 2.2.
///
/// Spec: 2.1.1 §mod_tariffs_pricecomponent_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PriceComponent {
    /// The dimension being priced.
    #[serde(rename = "type")]
    pub component_type: TariffDimensionType,
    /// Price per unit, excluding VAT.
    pub price: Number,
    /// Minimum amount to be billed: the dimension is billed in blocks of this size.
    pub step_size: u32,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl PriceComponent {
    /// A price component with a `step_size` of 1.
    #[must_use]
    pub fn new(component_type: TariffDimensionType, price: Number) -> Self {
        Self { component_type, price, step_size: 1, extensions: Extensions::new() }
    }
}

impl Validate for PriceComponent {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, component_type as "type", price);
        if self.step_size == 0 && self.component_type.step_size_unit().is_some() {
            v.report_at(
                "step_size",
                ViolationCode::OutOfRange,
                "a step_size of 0 would bill nothing for a dimension that has a unit",
            );
        }
    }
}

/// When a [`TariffElement`] is active, in OCPI 2.1.1.
///
/// The current-based restrictions (`min_current`, `max_current`) and the reservation restriction
/// arrived in OCPI 2.2.
///
/// Spec: 2.1.1 §mod_tariffs_tariffrestrictions_class
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct TariffRestrictions {
    /// Start time of day, valid from this time of the day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<LocalTime>,
    /// End time of day, valid until this time of the day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<LocalTime>,
    /// Start date, valid from this day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<LocalDate>,
    /// End date, valid until this day, excluding this day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<LocalDate>,
    /// Minimum used energy in kWh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_kwh: Option<Number>,
    /// Maximum used energy in kWh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_kwh: Option<Number>,
    /// Minimum power in kW.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_power: Option<Number>,
    /// Maximum power in kW.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_power: Option<Number>,
    /// Minimum duration in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration: Option<u64>,
    /// Maximum duration in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<u64>,
    /// Which days of the week this tariff is valid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub day_of_week: Vec<DayOfWeek>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
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
            min_power,
            max_power,
            day_of_week,
        );
        for (lo_name, lo, hi_name, hi) in [
            ("min_kwh", self.min_kwh, "max_kwh", self.max_kwh),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_2_1_1_price_component_cannot_express_vat() {
        let json = r#"{"type":"ENERGY","price":0.25,"step_size":1}"#;
        let component: PriceComponent = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&component).unwrap(), json);
        // A peer that sends a 2.2-style `vat` keeps it in extensions rather than losing it.
        let with_vat: PriceComponent =
            serde_json::from_str(r#"{"type":"ENERGY","price":0.25,"step_size":1,"vat":10}"#).unwrap();
        assert_eq!(with_vat.extensions.get::<u32>("vat").unwrap(), Some(10));
    }

    #[test]
    fn a_free_of_charge_tariff_has_the_usual_shape() {
        let tariff = Tariff::builder()
            .id("15")
            .currency("EUR")
            .elements(vec![
                TariffElement::builder()
                    .price_components(vec![PriceComponent::new(TariffDimensionType::Flat, Number::ZERO)])
                    .build(),
            ])
            .last_updated("2015-06-29T20:39:09Z".parse::<DateTime>().unwrap())
            .build();
        assert!(tariff.is_free_of_charge());
        assert!(tariff.validate().is_ok());
    }
}
