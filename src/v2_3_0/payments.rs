//! The *Payments* module, new in OCPI 2.3.0: ad-hoc payment terminals and their transactions.
//!
//! *Module Identifier: `payments`* — Data owner: PTP (Payment Terminal Provider).
//!
//! The module maps payment terminals onto Locations and EVSEs, and carries the financial
//! confirmations back so a CDR can be reconciled against what was actually captured at the
//! payment service provider.
//!
//! **Spec erratum.** `payments` is missing from the `ModuleID` table in
//! §version_information_endpoint_moduleid_enum of the same release that defines this chapter;
//! see [`ModuleId::Payments`](crate::ModuleId::Payments).
//!
//! Spec: 2.3.0 §mod_payments_payments_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_enum;
use crate::types::validate_fields;
use crate::types::{
    CiString, CountryCode, Currency, DateTime, Extensions, PartyId, PartyRef, Url, Validate, Validator,
    ViolationCode,
};

use super::locations::GeoLocation;
use super::types::Price;

/// One physical payment terminal, and the charge points it serves.
///
/// > *It is designed primarily to establish a mapping between charge points (locations and/or
/// > EVSEs) and payment terminals.*
///
/// Spec: 2.3.0 §mod_payments_terminal_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Terminal {
    /// Unique ID that identifies a terminal.
    pub terminal_id: CiString<36>,
    /// Reference used to link the terminal to a CSMS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_reference: Option<CiString<36>>,
    /// Party ID, as an alternative to the customer reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party_id: Option<PartyId>,
    /// Country code, as an alternative to the customer reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country_code: Option<CountryCode>,
    /// Street/block name and house number if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<CiString<45>>,
    /// City or town.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<CiString<45>>,
    /// Postal code of the terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<CiString<10>>,
    /// State or province, only where relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<CiString<20>>,
    /// ISO 3166-1 alpha-3 code for the country of this terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<CiString<3>>,
    /// Coordinates of the terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<GeoLocation>,
    /// Base URL of the downloadable invoice.
    ///
    /// The full URL is this base plus the session's `authorization_reference`; see
    /// [`InvoiceCreator::Cpo`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_base_url: Option<Url>,
    /// Which party creates the invoice for the eDriver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_creator: Option<InvoiceCreator>,
    /// Mapping value as issued by the PTP, e.g. a serial number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<CiString<36>>,
    /// All Locations assigned to this terminal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub location_ids: Vec<CiString<36>>,
    /// All EVSEs assigned to this terminal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub evse_uids: Vec<CiString<36>>,
    /// Timestamp when this Terminal was last updated (or created).
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Terminal {
    /// The party this terminal is assigned to, when it was identified by party rather than by
    /// customer reference.
    #[must_use]
    pub fn assigned_party(&self) -> Option<PartyRef> {
        match (&self.country_code, &self.party_id) {
            (Some(country_code), Some(party_id)) => {
                Some(PartyRef { country_code: country_code.clone(), party_id: party_id.clone() })
            }
            _ => None,
        }
    }

    /// Whether the terminal has been assigned to any charge point yet.
    ///
    /// A newly created terminal has neither, which is a legitimate intermediate state: the spec's
    /// own "newly created" example has both lists empty.
    #[must_use]
    pub fn is_assigned(&self) -> bool {
        !self.location_ids.is_empty() || !self.evse_uids.is_empty()
    }

    /// The URL an eDriver can download the invoice from for a given authorization reference.
    ///
    /// > *The CPO issues the invoice and provides it via the `invoice_base_url` +
    /// > `authorization_reference`.*
    #[must_use]
    pub fn invoice_url(&self, authorization_reference: &str) -> Option<Url> {
        self.invoice_base_url.as_ref().map(|base| base.join_segment(authorization_reference))
    }
}

impl Validate for Terminal {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            terminal_id,
            customer_reference,
            party_id,
            country_code,
            address,
            city,
            postal_code,
            state,
            country,
            coordinates,
            invoice_base_url,
            invoice_creator,
            reference,
            location_ids,
            evse_uids,
            last_updated,
        );
        // "This is an alternative to the customer reference which can be used" — a lone half of
        // the pair identifies nothing.
        if self.party_id.is_some() != self.country_code.is_some() {
            v.report(
                ViolationCode::MissingConditional,
                "`party_id` and `country_code` identify a party together; set both or neither",
            );
        }
        if self.invoice_creator == Some(InvoiceCreator::Cpo) && self.invoice_base_url.is_none() {
            v.report_at(
                "invoice_base_url",
                ViolationCode::MissingConditional,
                "the CPO provides the invoice via invoice_base_url + authorization_reference",
            );
        }
    }
}

/// What was actually captured at the payment service provider for one ad-hoc session.
///
/// > *It correlates payment transactions with charging sessions by using the
/// > `authorization_reference` obtained from the Commands.StartSession, Session, and CDR.*
///
/// Spec: 2.3.0 §mod_payments_financial_advice_confirmation_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct FinancialAdviceConfirmation {
    /// Unique ID that identifies a financial advice confirmation.
    pub id: CiString<36>,
    /// Reference to the authorization given by the PTP in `Commands.StartSession`.
    pub authorization_reference: CiString<36>,
    /// The real amount that was captured at the PSP. A consumer price, with VAT.
    pub total_costs: Price,
    /// ISO-4217 code of the currency.
    pub currency: Currency,
    /// Invoice-relevant data from the direct payment. Cardinality `+`.
    pub eft_data: Vec<CiString<255>>,
    /// Code identifying the financial advice status.
    pub capture_status_code: CaptureStatusCode,
    /// Message about any error in the financial advice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_status_message: Option<CiString<255>>,
    /// Timestamp when this confirmation was last updated (or created).
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl FinancialAdviceConfirmation {
    /// Whether the full amount was captured.
    #[must_use]
    pub fn is_fully_captured(&self) -> bool {
        self.capture_status_code == CaptureStatusCode::Success
    }
}

impl Validate for FinancialAdviceConfirmation {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            id,
            authorization_reference,
            total_costs,
            currency,
            eft_data,
            capture_status_code,
            capture_status_message,
            last_updated,
        );
        if self.eft_data.is_empty() {
            v.report_at(
                "eft_data",
                ViolationCode::EmptyRequiredList,
                "eft_data has cardinality `+`: it is mandatory on invoices, so at least one \
                 entry is required",
            );
        }
        if self.capture_status_code != CaptureStatusCode::Success && self.capture_status_message.is_none() {
            v.report_at(
                "capture_status_message",
                ViolationCode::MissingConditional,
                "a non-successful capture should say what went wrong",
            );
        }
    }
}

ocpi_enum! {
    /// Which party issues the invoice for an ad-hoc session.
    ///
    /// Spec: 2.3.0 §mod_payments_invoice_creator_enum
    pub enum InvoiceCreator {
        /// The CPO issues the invoice, via `invoice_base_url` + `authorization_reference`.
        Cpo = "CPO",
        /// The PTP issues the invoice and shows it to the eDriver at the payment terminal.
        Ptp = "PTP",
    }
}

ocpi_enum! {
    /// The outcome of the payment capture following a transaction.
    ///
    /// Spec: 2.3.0 §mod_payments_capture_status_code_enum
    pub enum CaptureStatusCode {
        /// Completed successfully; funds were secured.
        Success = "SUCCESS",
        /// Only part of the amount was approved, or conditions were altered during processing.
        PartialSuccess = "PARTIAL_SUCCESS",
        /// The capture attempt was unsuccessful.
        Failed = "FAILED",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal() -> Terminal {
        Terminal::builder()
            .terminal_id("TERM0001")
            .last_updated("2024-03-15T10:00:00Z".parse::<DateTime>().unwrap())
            .build()
    }

    #[test]
    fn a_newly_created_terminal_is_valid_but_unassigned() {
        let t = terminal();
        assert!(!t.is_assigned());
        assert!(t.validate().is_ok());
    }

    #[test]
    fn party_identification_needs_both_halves() {
        let mut t = terminal();
        t.party_id = Some(PartyId::new("TNM").unwrap());
        assert_eq!(t.validate().unwrap_err().as_slice()[0].code, ViolationCode::MissingConditional);
        t.country_code = Some(CountryCode::new("NL").unwrap());
        assert!(t.validate().is_ok());
        assert_eq!(t.assigned_party(), Some(PartyRef::new("NL", "TNM").unwrap()));
    }

    #[test]
    fn a_cpo_invoice_creator_needs_a_base_url() {
        let mut t = terminal();
        t.invoice_creator = Some(InvoiceCreator::Cpo);
        assert_eq!(t.validate().unwrap_err().as_slice()[0].pointer, "/invoice_base_url");
        t.invoice_base_url = Some(Url::new("https://cpo.example.com/invoices").unwrap());
        assert!(t.validate().is_ok());
        assert_eq!(t.invoice_url("AUTH123").unwrap().as_str(), "https://cpo.example.com/invoices/AUTH123");
    }

    #[test]
    fn a_failed_capture_must_explain_itself() {
        let mut fac = FinancialAdviceConfirmation::builder()
            .id("FAC1")
            .authorization_reference("AUTH123")
            .total_costs(Price::new("12.50".parse().unwrap()))
            .currency("EUR")
            .eft_data(vec![CiString::new("DEBIT 12.50 EUR").unwrap()])
            .capture_status_code(CaptureStatusCode::Failed)
            .last_updated("2024-03-15T10:00:00Z".parse::<DateTime>().unwrap())
            .build();
        assert!(!fac.is_fully_captured());
        assert_eq!(fac.validate().unwrap_err().as_slice()[0].pointer, "/capture_status_message");
        fac.capture_status_message = Some(CiString::new("insufficient funds").unwrap());
        assert!(fac.validate().is_ok());
    }

    #[test]
    fn eft_data_is_mandatory() {
        let fac = FinancialAdviceConfirmation::builder()
            .id("FAC1")
            .authorization_reference("AUTH123")
            .total_costs(Price::new("12.50".parse().unwrap()))
            .currency("EUR")
            .eft_data(vec![])
            .capture_status_code(CaptureStatusCode::Success)
            .last_updated("2024-03-15T10:00:00Z".parse::<DateTime>().unwrap())
            .build();
        assert_eq!(fac.validate().unwrap_err().as_slice()[0].code, ViolationCode::EmptyRequiredList);
    }
}
