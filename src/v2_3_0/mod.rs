//! The OCPI **2.3.0** wire model: the canonical model of this crate.
//!
//! Every other version in `ocpi-kit` is described as a delta from this one — [`crate::v2_2_1`]
//! re-exports the types that are byte-identical and defines only what changed, and
//! [`crate::convert`] carries objects between them with an explicit account of what was lost.
//!
//! # What 2.3.0 added
//!
//! | Area | Change |
//! |---|---|
//! | [`payments`] | new module: payment terminals and financial advice confirmations |
//! | [`types::Price`] | `{before_taxes, taxes[]}` replaces 2.2.1's `{excl_vat, incl_vat}` |
//! | [`tariffs::TaxIncluded`] | new required `Tariff.tax_included`, for North American tax rules |
//! | [`tariffs::PriceLimit`] | `min_price`/`max_price` gained an `after_taxes` bound |
//! | [`locations::Parking`] | new object, for EU AFIR reporting to National Access Points |
//! | [`locations::ConnectorType`] | became an `OpenEnum`, and gained `MCS` |
//! | [`locations::ConnectorCapability`] | new: ISO 15118-2 / -20 Plug & Charge |
//! | [`tokens::TokenType`] | became an `OpenEnum`, and gained `EMAID` |
//! | [`types::Role`] | `HUB` **removed**; a hub is now `Credentials.hub_party_id` |
//! | [`credentials::Credentials`] | gained `hub_party_id` |
//! | [`locations::Location`] | gained `help_phone` |
//!
//! # Conventions used throughout
//!
//! * Every object carries an `extensions` field: undocumented JSON is preserved, never dropped.
//! * A field the spec marks optional (`?`) is an `Option`; a list (`*` or `+`) is a `Vec`, and
//!   the `+` cardinality is checked by [`Validate`](crate::types::Validate) rather than by the
//!   type system, so a peer's under-filled object still decodes.
//! * A field named `type` on the wire is spelled out in Rust — `token_type`, `image_type`,
//!   `tariff_type` — and carries `#[serde(rename = "type")]`.
//! * Objects that are more than a handful of fields have a compile-time-checked builder:
//!   `Location::builder().country_code("NL")…build()`.
//!
//! Spec: <https://github.com/ocpi/ocpi>, `2.3.0/release/core`

pub mod cdrs;
pub mod charging_profiles;
pub mod commands;
pub mod credentials;
pub mod hub_client_info;
pub mod invoice_reconciliation;
pub mod locations;
pub mod sessions;
pub mod tariffs;
pub mod tokens;
pub mod types;
pub mod versions;

#[cfg(feature = "bookings")]
#[cfg_attr(docsrs, doc(cfg(feature = "bookings")))]
pub mod bookings;

#[cfg(feature = "payments")]
#[cfg_attr(docsrs, doc(cfg(feature = "payments")))]
pub mod payments;

pub use cdrs::Cdr;
pub use credentials::Credentials;
pub use invoice_reconciliation::{InvoiceReconciliationRecord, Reconciliation, reconcile};
pub use locations::{Connector, Evse, Location};

#[cfg(feature = "bookings")]
pub use bookings::{Booking, BookingLocation, Calendar};

#[cfg(feature = "payments")]
pub use payments::{FinancialAdviceConfirmation, Terminal};
pub use sessions::Session;
pub use tariffs::Tariff;
pub use tokens::Token;
pub use types::{Price, Role, TaxAmount};
pub use versions::{Endpoint, Version, VersionDetails};

/// The version number this module implements.
pub const VERSION: crate::VersionNumber = crate::VersionNumber::V2_3_0;
