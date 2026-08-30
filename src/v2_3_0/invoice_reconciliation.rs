//! The *Invoice Reconciliation* module, from the OCPI 2.3.0 `payments` release branch.
//!
//! *Module Identifier: `invoicereconciliation`* — Data owner: CPO **or** eMSP.
//!
//! > *Invoice Reconciliation enables Parties that receive invoices for Charging Sessions to check
//! > the amounts of these invoices against the CDR data that they transferred via OCPI.*
//!
//! The record itself is deliberately small: an invoice identifier and the list of CDR ids that
//! invoice covers. Everything else — when to invoice, how the document is delivered, how it is
//! paid — is left to the parties.
//!
//! The reconciliation itself is a local computation, and this crate can do it:
//! [`reconcile`] adds up the CDRs a record names and compares the total to the invoice.
//!
//! Spec: 2.3.0-payments §mod_invoice_reconciliation_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    CiString, CountryCode, DateTime, Extensions, Number, PartyId, PartyRef, Validate, Validator,
    ViolationCode,
};

use super::cdrs::Cdr;

/// One invoice, and the CDRs it covers.
///
/// Spec: 2.3.0-payments §mod_invoice_reconciliation_record_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct InvoiceReconciliationRecord {
    /// ISO-3166 alpha-2 country code of the party that 'owns' this record.
    pub country_code: CountryCode,
    /// ID of the party that 'owns' this record.
    pub party_id: PartyId,
    /// Uniquely identifies this record.
    pub id: CiString<36>,
    /// An identifier for the invoice this record is about.
    pub invoice_id: CiString<255>,
    /// The CDRs invoiced by that invoice. Cardinality `+`.
    pub cdrs: Vec<CiString<36>>,
    /// When this record was issued.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl InvoiceReconciliationRecord {
    /// The party that issued this record.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }

    /// Whether this record covers a given CDR id, comparing case-insensitively.
    #[must_use]
    pub fn covers(&self, cdr_id: &str) -> bool {
        self.cdrs.iter().any(|id| id.eq_ignore_case(cdr_id))
    }
}

impl Validate for InvoiceReconciliationRecord {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, country_code, party_id, id, invoice_id, cdrs, last_updated);
        if self.cdrs.is_empty() {
            v.report_at(
                "cdrs",
                ViolationCode::EmptyRequiredList,
                "an Invoice Reconciliation Record has cardinality `+` cdrs: an invoice that \
                 covers no CDR cannot be reconciled",
            );
        }
        let mut seen: Vec<&CiString<36>> = Vec::new();
        for id in &self.cdrs {
            if seen.contains(&id) {
                v.report_at(
                    "cdrs",
                    ViolationCode::Inconsistent,
                    format!("the CDR {:?} is listed more than once", id.as_str()),
                );
            }
            seen.push(id);
        }
    }
}

/// The outcome of checking an invoice against the CDRs it claims to cover.
#[derive(Clone, Debug, PartialEq)]
pub struct Reconciliation {
    /// The record that was checked.
    pub invoice_id: String,
    /// The total of the CDRs that were found, excluding taxes.
    pub total_excl_taxes: Number,
    /// The total of the CDRs that were found, including the taxes each CDR carries.
    pub total_incl_taxes: Number,
    /// CDR ids the record names that were not among the CDRs supplied.
    ///
    /// A non-empty list means the check is incomplete, not that the invoice is wrong.
    pub missing_cdrs: Vec<String>,
    /// CDRs that were supplied but the record does not name.
    ///
    /// Not an error: an invoice covers the CDRs it lists, and *"the set of invoices referenced by
    /// an Invoice Reconciliation Record is not determined by timing, but by the list of invoice
    /// IDs"*.
    pub unlisted_cdrs: Vec<String>,
    /// The currencies encountered, in the order first seen.
    ///
    /// More than one is a problem: the totals are then not comparable to a single invoice amount.
    pub currencies: Vec<String>,
}

impl Reconciliation {
    /// Whether every CDR the record names was available to check.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.missing_cdrs.is_empty()
    }

    /// Whether the totals are meaningful: complete, and in a single currency.
    #[must_use]
    pub fn is_conclusive(&self) -> bool {
        self.is_complete() && self.currencies.len() == 1
    }

    /// The difference between an invoiced amount and the computed pre-tax total.
    ///
    /// Positive means the invoice asks for more than the CDRs add up to.
    #[must_use]
    pub fn difference_from(&self, invoiced_excl_taxes: Number) -> Number {
        invoiced_excl_taxes - self.total_excl_taxes
    }
}

/// Adds up the CDRs an Invoice Reconciliation Record names.
///
/// `cdrs` may contain more CDRs than the record covers; only the ones it names are counted, and
/// the rest are reported as [`Reconciliation::unlisted_cdrs`].
///
/// ```
/// # use ocpi_kit::v2_3_0::invoice_reconciliation::{reconcile, InvoiceReconciliationRecord};
/// # use ocpi_kit::v2_3_0::cdrs::Cdr;
/// # fn check(record: &InvoiceReconciliationRecord, cdrs: &[Cdr]) {
/// let result = reconcile(record, cdrs);
/// if !result.is_conclusive() {
///     eprintln!("cannot check invoice {}: {:?}", result.invoice_id, result.missing_cdrs);
/// }
/// println!("the CDRs add up to {}", result.total_incl_taxes);
/// # }
/// ```
///
/// Spec: 2.3.0-payments §mod_invoice_reconciliation_flow_and_lifecycle
#[must_use]
pub fn reconcile(record: &InvoiceReconciliationRecord, cdrs: &[Cdr]) -> Reconciliation {
    let mut total_excl = Number::ZERO;
    let mut total_incl = Number::ZERO;
    let mut currencies: Vec<String> = Vec::new();
    let mut found: Vec<String> = Vec::new();

    for cdr in cdrs {
        if !record.covers(cdr.id.as_str()) {
            continue;
        }
        found.push(cdr.id.as_str().to_owned());
        // A credit CDR corrects an earlier one, so it counts against the invoice.
        let sign = if cdr.is_credit() { -Number::ONE } else { Number::ONE };
        total_excl = total_excl + cdr.total_cost.before_taxes * sign;
        total_incl = total_incl + cdr.total_cost.after_taxes() * sign;
        let currency = cdr.currency.as_str().to_owned();
        if !currencies.contains(&currency) {
            currencies.push(currency);
        }
    }

    let missing_cdrs = record
        .cdrs
        .iter()
        .filter(|id| !found.iter().any(|f| f.eq_ignore_ascii_case(id.as_str())))
        .map(|id| id.as_str().to_owned())
        .collect();
    let unlisted_cdrs = cdrs
        .iter()
        .filter(|cdr| !record.covers(cdr.id.as_str()))
        .map(|cdr| cdr.id.as_str().to_owned())
        .collect();

    Reconciliation {
        invoice_id: record.invoice_id.as_str().to_owned(),
        total_excl_taxes: total_excl,
        total_incl_taxes: total_incl,
        missing_cdrs,
        unlisted_cdrs,
        currencies,
    }
}

#[cfg(all(test, feature = "testkit"))]
mod tests {
    use super::*;
    use crate::types::Extensions;
    use crate::v2_3_0::types::Price;

    fn cdr(id: &str, cost: &str, credit: bool) -> Cdr {
        let mut cdr = crate::testkit::sample::cdr(id).unwrap();
        cdr.total_cost = Price {
            before_taxes: cost.parse().unwrap(),
            taxes: vec![
                super::super::types::TaxAmount::new(
                    "VAT",
                    Some(Number::from(10u32)),
                    (cost.parse::<Number>().unwrap() / Number::from(10u32)).round_dp(2),
                )
                .unwrap(),
            ],
            extensions: Extensions::new(),
        };
        cdr.credit = credit.then_some(true);
        if credit {
            cdr.credit_reference_id = Some(CiString::new("CDR1").unwrap());
        }
        cdr
    }

    fn record(cdr_ids: &[&str]) -> InvoiceReconciliationRecord {
        InvoiceReconciliationRecord::builder()
            .country_code("NL")
            .party_id("TNM")
            .id("IRR1")
            .invoice_id("INV-2024-03")
            .cdrs(cdr_ids.iter().map(|id| CiString::new(*id).unwrap()).collect::<Vec<_>>())
            .last_updated("2024-04-01T00:00:00Z".parse::<DateTime>().unwrap())
            .build()
    }

    #[test]
    fn the_named_cdrs_are_added_up_and_the_rest_ignored() {
        let record = record(&["CDR1", "CDR2"]);
        let cdrs = vec![cdr("CDR1", "10.00", false), cdr("CDR2", "5.00", false), cdr("CDR3", "99.00", false)];
        let result = reconcile(&record, &cdrs);
        assert_eq!(result.total_excl_taxes.to_string(), "15.00");
        assert_eq!(result.total_incl_taxes.to_string(), "16.50");
        assert_eq!(result.unlisted_cdrs, vec!["CDR3".to_owned()]);
        assert!(result.is_conclusive());
        assert_eq!(result.difference_from("15.00".parse().unwrap()), Number::ZERO);
    }

    #[test]
    fn a_credit_cdr_counts_against_the_invoice() {
        let record = record(&["CDR1", "CDR2"]);
        let cdrs = vec![cdr("CDR1", "10.00", false), cdr("CDR2", "4.00", true)];
        let result = reconcile(&record, &cdrs);
        assert_eq!(result.total_excl_taxes.to_string(), "6.00");
    }

    #[test]
    fn a_missing_cdr_makes_the_check_inconclusive() {
        let record = record(&["CDR1", "CDR2"]);
        let result = reconcile(&record, &[cdr("CDR1", "10.00", false)]);
        assert_eq!(result.missing_cdrs, vec!["CDR2".to_owned()]);
        assert!(!result.is_complete() && !result.is_conclusive());
    }

    #[test]
    fn several_currencies_make_the_totals_meaningless() {
        let record = record(&["CDR1", "CDR2"]);
        let mut second = cdr("CDR2", "5.00", false);
        second.currency = crate::types::Currency::new("CHF").unwrap();
        let result = reconcile(&record, &[cdr("CDR1", "10.00", false), second]);
        assert_eq!(result.currencies.len(), 2);
        assert!(result.is_complete());
        assert!(!result.is_conclusive(), "two currencies cannot be compared to one amount");
    }

    #[test]
    fn a_record_that_covers_no_cdr_is_reported() {
        let empty = InvoiceReconciliationRecord { cdrs: Vec::new(), ..record(&["CDR1"]) };
        assert_eq!(empty.validate().unwrap_err().as_slice()[0].code, ViolationCode::EmptyRequiredList);
        let duplicated = record(&["CDR1", "cdr1"]);
        assert!(duplicated.validate().is_err(), "ids compare case-insensitively");
    }

    #[test]
    fn round_trips_through_json() {
        let json = r#"{"country_code":"NL","party_id":"TNM","id":"IRR1","invoice_id":"INV-2024-03","cdrs":["CDR1","CDR2"],"last_updated":"2024-04-01T00:00:00Z"}"#;
        let record: InvoiceReconciliationRecord = serde_json::from_str(json).unwrap();
        assert!(record.covers("cdr1"));
        assert_eq!(serde_json::to_string(&record).unwrap(), json);
    }
}
