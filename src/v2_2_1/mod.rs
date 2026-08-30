//! The OCPI **2.2.1** wire model, described as a delta from [`v2_3_0`](crate::v2_3_0).
//!
//! OCPI 2.2.1 is still the most widely deployed version, and most of it is byte-identical to
//! 2.3.0. Rather than transcribing the whole specification twice, this module **re-exports every
//! type whose wire format is unchanged** and defines only what actually differs. A
//! [`GeoLocation`](crate::v2_3_0::locations::GeoLocation) is therefore literally the same Rust type in both
//! versions and needs no conversion; a [`Price`] is not, and
//! [`convert`](crate::convert) knows how to carry one across.
//!
//! # What OCPI 2.3.0 changed
//!
//! | Type | 2.2.1 | 2.3.0 |
//! |---|---|---|
//! | [`types::Price`] | `{excl_vat, incl_vat?}` | `{before_taxes, taxes[]}` |
//! | [`types::Role`] | has `HUB` | `HUB` removed; hubs use `Credentials.hub_party_id` |
//! | [`credentials::Credentials`] | no `hub_party_id` | `hub_party_id` added |
//! | [`tariffs::Tariff`] | `min/max_price: Price` | `PriceLimit`, plus `tax_included`, `preauthorize_amount` |
//! | [`locations::Location`] | — | `parking_places`, `help_phone` added |
//! | [`locations::Evse`] | — | `parking`, `accepted_service_providers` added |
//! | [`locations::Connector`] | — | `capabilities` added |
//! | [`locations::ConnectorType`] | closed enum | `OpenEnum`, plus `MCS`, `SAE_J3400` |
//! | [`locations::ParkingRestriction`] | 5 values | `OpenEnum`, plus `EMPLOYEES`, `TAXIS`, `TENANTS` |
//! | [`tokens::TokenType`] | closed enum | `OpenEnum`, plus `EMAID` |
//! | *payments* | absent | new module |
//!
//! # Closed enums, decoded leniently
//!
//! OCPI 2.2.1 has **no** `OpenEnum`: by the letter of the specification an unrecognised
//! `ConnectorType` is a decode error. This crate keeps the value anyway — losing a page of
//! Locations over one unfamiliar plug is worse than the alternative, and OCPI 2.3.0
//! reclassified exactly these enums as open for the same reason — and reports it through
//! [`Validate`](crate::types::Validate). See
//! [`ocpi_lenient_enum!`](crate::ocpi_lenient_enum).
//!
//! Spec: <https://github.com/ocpi/ocpi>, `release-2.2.1-bugfixes`

pub mod cdrs;
pub mod commands;
pub mod credentials;
pub mod hub_client_info;
pub mod locations;
pub mod sessions;
pub mod tariffs;
pub mod tokens;
pub mod types;

/// The *Charging Profiles* module of OCPI 2.2.1.
///
/// Wire-identical to [`v2_3_0::charging_profiles`](crate::v2_3_0::charging_profiles).
///
/// Spec: 2.2.1 §mod_charging_profiles_module
pub mod charging_profiles {
    pub use crate::v2_3_0::charging_profiles::*;
}

/// The *Versions* module of OCPI 2.2.1.
///
/// Wire-identical to [`v2_3_0::versions`](crate::v2_3_0::versions). The `ModuleId` and
/// `VersionNumber` enums are version-neutral by design, so discovery against a 2.2.1 peer that
/// advertises a 2.3.0 module still works.
///
/// Spec: 2.2.1 §versions_module
pub mod versions {
    pub use crate::v2_3_0::versions::*;
}

pub use cdrs::Cdr;
pub use credentials::Credentials;
pub use locations::{Connector, Evse, Location};
pub use sessions::Session;
pub use tariffs::Tariff;
pub use tokens::Token;
pub use types::{Price, Role};
pub use versions::{Endpoint, Version, VersionDetails};

/// The version number this module implements.
pub const VERSION: crate::VersionNumber = crate::VersionNumber::V2_2_1;
