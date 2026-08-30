//! The OCPI 2.2.1 spec-wide types that differ from 2.3.0: `Price` and `Role`.

use serde::{Deserialize, Serialize};

use crate::ocpi_enum;
use crate::types::validate_fields;
use crate::types::{Extensions, Number, Validate, Validator, ViolationCode};

/// A price excluding and, optionally, including VAT.
///
/// This is the OCPI 2.2.1 shape. OCPI 2.3.0 replaced it with
/// [`v2_3_0::types::Price`](crate::v2_3_0::types::Price), `{before_taxes, taxes[]}`, because one
/// `incl_vat` number cannot express a North American tax system where several separately-named
/// taxes apply and the CPO may not know the rate when it writes the Tariff.
///
/// > *`excl_vat`: Price/Cost excluding VAT. `incl_vat`: Price/Cost including VAT.*
///
/// Spec: 2.2.1 §types_price_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Price {
    /// Price/cost excluding VAT.
    pub excl_vat: Number,
    /// Price/cost including VAT.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incl_vat: Option<Number>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Price {
    /// A price with no VAT figure attached.
    #[must_use]
    pub fn new(excl_vat: Number) -> Self {
        Self { excl_vat, incl_vat: None, extensions: Extensions::new() }
    }

    /// A price with both figures.
    #[must_use]
    pub fn with_vat(excl_vat: Number, incl_vat: Number) -> Self {
        Self { excl_vat, incl_vat: Some(incl_vat), extensions: Extensions::new() }
    }

    /// Zero, with no VAT figure.
    #[must_use]
    pub fn zero() -> Self {
        Self::new(Number::ZERO)
    }

    /// The VAT amount, when both figures are present.
    #[must_use]
    pub fn vat_amount(&self) -> Option<Number> {
        self.incl_vat.map(|incl| incl - self.excl_vat)
    }
}

impl Validate for Price {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, excl_vat, incl_vat);
        if self.incl_vat.is_some_and(|incl| incl < self.excl_vat) {
            v.report_at(
                "incl_vat",
                ViolationCode::Inconsistent,
                "the amount including VAT cannot be lower than the amount excluding it",
            );
        }
    }
}

ocpi_enum! {
    /// The role a party plays in the EV charging landscape, in OCPI 2.2.1.
    ///
    /// The difference from [`v2_3_0::types::Role`](crate::v2_3_0::types::Role) is `HUB`, which
    /// 2.3.0 **removed**: a roaming hub is represented there by `Credentials.hub_party_id`
    /// together with the parties it hosts listed in `roles`.
    ///
    /// Spec: 2.2.1 §types_role_enum
    pub enum Role {
        /// Charge Point Operator.
        Cpo = "CPO",
        /// e-Mobility Service Provider.
        Emsp = "EMSP",
        /// Hub role. **Removed in OCPI 2.3.0.**
        Hub = "HUB",
        /// National Access Point: a national database of all Location information of a country.
        Nap = "NAP",
        /// Navigation Service Provider: like an eMSP, probably only interested in Locations.
        Nsp = "NSP",
        /// Other role.
        Other = "OTHER",
        /// Smart Charging Service Provider.
        Scsp = "SCSP",
    }
}

impl Role {
    /// Whether a Broadcast Push from a party with role `from` reaches a party with this role.
    ///
    /// See [`v2_3_0::types::Role::receives_broadcast_from`](crate::v2_3_0::types::Role::receives_broadcast_from);
    /// a `HUB` is the party doing the fan-out, not a recipient of one.
    ///
    /// Spec: 2.2.1 §transport_and_format_message_routing_broadcast_push
    #[must_use]
    pub const fn receives_broadcast_from(self, from: Self) -> bool {
        match (from, self) {
            (Self::Hub, _) | (_, Self::Hub) => false,
            (Self::Cpo, r) => {
                matches!(r, Self::Emsp | Self::Nsp | Self::Nap | Self::Other | Self::Scsp)
            }
            (_, r) => matches!(r, Self::Cpo),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vat_is_the_difference_between_the_two_figures() {
        let p = Price::with_vat("5.00".parse().unwrap(), "5.50".parse().unwrap());
        assert_eq!(p.vat_amount().unwrap().to_string(), "0.50");
        assert!(p.validate().is_ok());
        assert_eq!(Price::new(Number::ONE).vat_amount(), None);
    }

    #[test]
    fn incl_vat_below_excl_vat_is_reported() {
        let p = Price::with_vat("5.00".parse().unwrap(), "4.00".parse().unwrap());
        assert_eq!(p.validate().unwrap_err().as_slice()[0].pointer, "/incl_vat");
    }

    #[test]
    fn serialises_in_the_2_2_1_shape() {
        let p = Price::with_vat("2.50".parse().unwrap(), "2.75".parse().unwrap());
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"excl_vat":2.5,"incl_vat":2.75}"#);
        assert_eq!(serde_json::to_string(&Price::new(Number::ZERO)).unwrap(), r#"{"excl_vat":0}"#);
    }

    #[test]
    fn the_hub_role_exists_here_and_only_here() {
        assert_eq!("HUB".parse::<Role>().unwrap(), Role::Hub);
        assert!("HUB".parse::<crate::v2_3_0::types::Role>().is_err());
        assert!(!Role::Cpo.receives_broadcast_from(Role::Hub));
        assert!(Role::Emsp.receives_broadcast_from(Role::Cpo));
    }
}
