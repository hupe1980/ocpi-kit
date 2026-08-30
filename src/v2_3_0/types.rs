//! The spec-wide types whose shape is specific to OCPI 2.3.0: `Price`, `TaxAmount` and `Role`.
//!
//! These live here rather than in [`crate::types`] because they changed between versions. OCPI
//! 2.2.1's `Price` is `{excl_vat, incl_vat}` and its `Role` enum has a `HUB` member; both are in
//! [`crate::v2_2_1`].
//!
//! Spec: 2.3.0 §types_types

use serde::{Deserialize, Serialize};

use crate::ocpi_enum;
use crate::types::validate_fields;
use crate::types::{Extensions, Number, OcpiText, Validate, Validator, ViolationCode};

/// A price and the taxes applicable to it.
///
/// OCPI 2.3.0 replaced 2.2.1's `{excl_vat, incl_vat}` pair with a pre-tax amount plus an itemised
/// list of taxes. The reason is North America: a CPO there does not know the applicable tax rate
/// when it writes a Tariff, and where it does, several separately-named taxes can apply at once
/// (GST and QST in Quebec, for example). One `incl_vat` number cannot express that.
///
/// ```
/// use ocpi_kit::v2_3_0::types::Price;
///
/// let json = r#"{"before_taxes":5.00,"taxes":[{"name":"VAT","percentage":10,"amount":0.50}]}"#;
/// let price: Price = serde_json::from_str(json).unwrap();
/// assert_eq!(price.after_taxes().to_string(), "5.5");
/// ```
///
/// Spec: 2.3.0 §types_price_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Price {
    /// Price/cost excluding taxes.
    pub before_taxes: Number,
    /// All taxes applicable to this price and relevant to the receiver of the Session or CDR.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub taxes: Vec<TaxAmount>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Price {
    /// A price with no taxes attached.
    #[must_use]
    pub fn new(before_taxes: Number) -> Self {
        Self { before_taxes, taxes: Vec::new(), extensions: Extensions::new() }
    }

    /// Zero, with no taxes. *"A total_cost of 0.00 means free of charge."*
    #[must_use]
    pub fn zero() -> Self {
        Self::new(Number::ZERO)
    }

    /// The sum of all tax amounts.
    #[must_use]
    pub fn total_tax(&self) -> Number {
        self.taxes.iter().map(|t| t.amount).sum()
    }

    /// The price including every listed tax.
    ///
    /// Note that an empty `taxes` list does not mean "no tax is due": under the North American
    /// model the Tariff's `tax_included` field says `NO` and the taxes are added later by
    /// someone who knows the rate. See [`TaxIncluded`](crate::v2_3_0::tariffs::TaxIncluded).
    #[must_use]
    pub fn after_taxes(&self) -> Number {
        self.before_taxes + self.total_tax()
    }
}

impl Validate for Price {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, before_taxes, taxes);
    }
}

/// One tax applicable to a [`Price`]. New in OCPI 2.3.0.
///
/// Spec: 2.3.0 §types_tax_amount_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TaxAmount {
    /// A description of the tax.
    ///
    /// > *In countries where a tax name is required like Canada this can be something like
    /// > "QST". In countries where this is not required, this can be something more generic like
    /// > "VAT" or "General Sales Tax".*
    pub name: OcpiText,
    /// Tax Account Number of the business entity remitting these taxes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_number: Option<OcpiText>,
    /// Tax percentage. Optional, as it is not required in all countries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentage: Option<Number>,
    /// The amount of money of this tax that is due.
    pub amount: Number,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl TaxAmount {
    /// A tax line with a name, a percentage and an amount.
    ///
    /// # Errors
    ///
    /// Returns [`crate::types::InvalidString`] if `name` contains a control character.
    pub fn new(
        name: impl Into<String>,
        percentage: Option<Number>,
        amount: Number,
    ) -> Result<Self, crate::types::InvalidString> {
        Ok(Self {
            name: OcpiText::new(name)?,
            account_number: None,
            percentage,
            amount,
            extensions: Extensions::new(),
        })
    }
}

impl Validate for TaxAmount {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, name, account_number, percentage, amount);
        if self.percentage.is_some_and(Number::is_negative) {
            v.report_at("percentage", ViolationCode::OutOfRange, "a tax percentage cannot be negative");
        }
    }
}

ocpi_enum! {
    /// The role a party plays in the EV charging landscape.
    ///
    /// OCPI 2.2.1's `HUB` role was **removed** in 2.3.0: a roaming hub is now represented by
    /// `Credentials.hub_party_id` together with the parties it hosts listed in `roles`. See
    /// [`crate::v2_2_1::types::Role`] for the 2.2.1 shape.
    ///
    /// PTP (Payment Terminal Provider), introduced with the Payments module, is a *market* role
    /// in the 2.3.0 terminology and deliberately **not** a value of this enum.
    ///
    /// Spec: 2.3.0 §types_role_enum
    pub enum Role {
        /// Charge Point Operator.
        Cpo = "CPO",
        /// e-Mobility Service Provider.
        Emsp = "EMSP",
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
    /// > *For simplicity, connected clients might push information to all connected clients with
    /// > an "opposite role", for example: CPO pushing information to all eMSPs and NSPs, eMSP
    /// > pushing information to all CPOs. (The role "Other" is seen as an eMSP type of role, so
    /// > Broadcast Push from a CPO is also sent to "Other". Messages from "Other" are only sent
    /// > to CPOs and not to eMSPs though.)*
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing_broadcast_push
    #[must_use]
    pub const fn receives_broadcast_from(self, from: Self) -> bool {
        match from {
            // A CPO broadcasts to the eMSP-like roles, "Other" included.
            Self::Cpo => matches!(self, Self::Emsp | Self::Nsp | Self::Nap | Self::Other | Self::Scsp),
            // The eMSP-like roles broadcast to CPOs.
            Self::Emsp | Self::Nsp | Self::Nap | Self::Scsp | Self::Other => matches!(self, Self::Cpo),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Validate;

    #[test]
    fn price_sums_its_tax_lines() {
        let json = r#"{"before_taxes":5.00,"taxes":[{"name":"GST","percentage":5,"amount":0.25},{"name":"QST","percentage":9.975,"amount":0.50}]}"#;
        let p: Price = serde_json::from_str(json).unwrap();
        assert_eq!(p.total_tax().to_string(), "0.75");
        assert_eq!(p.after_taxes().to_string(), "5.75");
        assert!(p.validate().is_ok());
    }

    #[test]
    fn a_price_without_taxes_serialises_without_the_key() {
        let p = Price::new("1.25".parse().unwrap());
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"before_taxes":1.25}"#);
    }

    #[test]
    fn broadcast_reaches_only_the_opposite_roles() {
        assert!(Role::Emsp.receives_broadcast_from(Role::Cpo));
        assert!(Role::Other.receives_broadcast_from(Role::Cpo));
        assert!(Role::Cpo.receives_broadcast_from(Role::Emsp));
        // "Messages from Other are only sent to CPOs and not to eMSPs though."
        assert!(!Role::Emsp.receives_broadcast_from(Role::Other));
        assert!(Role::Cpo.receives_broadcast_from(Role::Other));
        assert!(!Role::Cpo.receives_broadcast_from(Role::Cpo));
    }

    #[test]
    fn the_hub_role_of_2_2_1_is_not_a_2_3_0_role() {
        assert!("HUB".parse::<Role>().is_err());
    }
}
