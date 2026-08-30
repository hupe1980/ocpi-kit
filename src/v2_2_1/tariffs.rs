//! The *Tariffs* module of OCPI 2.2.1, as a delta from
//! [`v2_3_0::tariffs`](crate::v2_3_0::tariffs).
//!
//! OCPI 2.3.0 changed three things about the [`Tariff`] object, all of them about tax:
//!
//! * `min_price`/`max_price` became [`PriceLimit`](crate::v2_3_0::tariffs::PriceLimit), which can
//!   bound the after-tax total as well as the pre-tax one; here they are the 2.2.1
//!   [`Price`];
//! * `tax_included` was added, and is **required** there;
//! * `preauthorize_amount` was added, for the Payments module that does not exist here.
//!
//! In 2.2.1 a `PriceComponent.price` is always **excluding VAT**; in 2.3.0 that depends on the
//! Tariff's `tax_included`. Everything else about the two modules is identical.
//!
//! Spec: 2.2.1 §mod_tariffs_tariffs_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    CiString, CountryCode, Currency, DateTime, DisplayText, Extensions, PartyId, PartyRef, Url, Validate,
    Validator, ViolationCode,
};

use super::locations::EnergyMix;
use super::types::Price;

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::tariffs::{
    DayOfWeek, PriceComponent, ReservationRestrictionType, TariffDimensionType, TariffElement,
    TariffRestrictions, TariffType,
};

/// A tariff: one or more [`TariffElement`]s that price a charging session, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_tariffs_tariff_object
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tariff_alt_text: Vec<DisplayText>,
    /// URL to a web page explaining the tariff in human-readable form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tariff_alt_url: Option<Url>,
    /// A Charging Session with this tariff will cost at least this amount.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_price: Option<Price>,
    /// A Charging Session with this tariff will not cost more than this amount.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price: Option<Price>,
    /// The Tariff Elements. Cardinality `+`.
    pub elements: Vec<TariffElement>,
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
    #[must_use]
    pub fn is_active_at(&self, instant: DateTime) -> bool {
        self.start_date_time.is_none_or(|s| instant >= s) && self.end_date_time.is_none_or(|e| instant < e)
    }

    /// Whether this is the "Free of Charge" shape the spec prescribes.
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
            tariff_alt_url, min_price, max_price, elements, start_date_time, end_date_time,
            energy_mix, last_updated,
        );
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
            && max.excl_vat < min.excl_vat
        {
            v.report_at(
                "max_price",
                ViolationCode::Inconsistent,
                "max_price.excl_vat is below min_price.excl_vat",
            );
        }
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
