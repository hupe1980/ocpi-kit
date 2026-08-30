//! The OCPI **2.1.1** wire model: the legacy version, and still in the field.
//!
//! OCPI 2.1.1 predates almost everything the later versions are built on. It has:
//!
//! * **no owner fields on objects** — `country_code` and `party_id` came with 2.2, so in 2.1.1
//!   the owner is known only from the URL and the credentials handshake;
//! * **no message routing** — the four `OCPI-*` headers do not exist, so a connection is
//!   strictly peer-to-peer;
//! * **a flat `Credentials` object** — one party, one role, no `roles` list;
//! * **no `Price`** — a cost is a bare `number` excluding VAT, and a `PriceComponent` has no
//!   `vat` field at all;
//! * **no `CommandResult`** — the asynchronous callback reuses `CommandResponse`, whose
//!   `CommandResponseType` therefore includes `TIMEOUT`;
//! * **seven modules** — no Hub Client Info, no Charging Profiles, no Payments;
//! * **`start_datetime`/`end_datetime`** on a Session, without the second underscore every later
//!   version uses.
//!
//! Everything here is decoded leniently: 2.1.1 declares every enum closed, and a peer still
//! running it in 2026 will have plugs, token types and dimensions that the 2015 list does not
//! contain. See [`ocpi_lenient_enum!`](crate::ocpi_lenient_enum).
//!
//! # Talking to a 2.1.1 peer
//!
//! [`Quirks::for_version`](crate::transport::Quirks::for_version) sets the two flags such a peer
//! needs: it does not Base64-encode the `Authorization` token, and it has no routing headers.
//!
//! Spec: <https://github.com/ocpi/ocpi>, `release-2.1.1-bugfixes`

pub mod cdrs;
pub mod commands;
pub mod credentials;
pub mod locations;
pub mod sessions;
pub mod tariffs;
pub mod tokens;

/// The *Versions* module of OCPI 2.1.1.
///
/// Wire-identical to the later versions except that `Endpoint` has no `role` field:
///
/// > *NOTE: OCPI 2.2 introduced the role field in the version details. Older versions of OCPI do
/// > not support this.*
///
/// This crate models `Endpoint.role` as required and defaults it to `SENDER` when a 2.1.1 peer
/// omits it, which is what the specification advises for the credentials module and the only
/// sensible reading for a version with no interface roles at all.
///
/// Spec: 2.1.1 §version_information_endpoint
pub mod versions {
    use serde::{Deserialize, Serialize};

    use crate::types::validate_fields;
    use crate::types::{Extensions, Url, Validate, Validator};
    use crate::{InterfaceRole, ModuleId, VersionNumber};

    pub use crate::v2_3_0::versions::Version;

    /// The endpoints a 2.1.1 party implements for one version.
    ///
    /// Spec: 2.1.1 §version_information_endpoint
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    pub struct VersionDetails {
        /// The version number these endpoints belong to.
        pub version: VersionNumber,
        /// The supported endpoints for this version.
        pub endpoints: Vec<Endpoint>,
        /// Undocumented JSON fields, preserved verbatim.
        #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
        pub extensions: Extensions,
    }

    impl VersionDetails {
        /// The URL of a module's endpoint.
        ///
        /// There are no interface roles in 2.1.1, so a module has at most one endpoint.
        #[must_use]
        pub fn url(&self, module: &ModuleId) -> Option<&Url> {
            self.endpoints.iter().find(|e| e.identifier.matches(module)).map(|e| &e.url)
        }
    }

    impl Validate for VersionDetails {
        fn validate_in(&self, v: &mut Validator) {
            validate_fields!(self, v, version, endpoints);
        }
    }

    /// One module endpoint of a 2.1.1 party.
    ///
    /// Spec: 2.1.1 §version_information_endpoint
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    pub struct Endpoint {
        /// Endpoint identifier.
        pub identifier: ModuleId,
        /// URL to the endpoint.
        pub url: Url,
        /// Undocumented JSON fields, preserved verbatim.
        #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
        pub extensions: Extensions,
    }

    impl Endpoint {
        /// Creates an endpoint entry.
        #[must_use]
        pub fn new(identifier: ModuleId, url: Url) -> Self {
            Self { identifier, url, extensions: Extensions::new() }
        }

        /// The interface role this endpoint would have in OCPI 2.2 and later.
        ///
        /// Always `SENDER`, since 2.1.1 has no roles and the specification advises sending
        /// `SENDER` where one is required.
        #[must_use]
        pub const fn assumed_role(&self) -> InterfaceRole {
            InterfaceRole::Sender
        }
    }

    impl Validate for Endpoint {
        fn validate_in(&self, v: &mut Validator) {
            validate_fields!(self, v, identifier, url);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_2_1_1_endpoint_has_no_role_field() {
            let json = r#"{"version":"2.1.1","endpoints":[{"identifier":"credentials","url":"https://example.com/ocpi/2.1.1/credentials"}]}"#;
            let details: VersionDetails = serde_json::from_str(json).unwrap();
            assert_eq!(details.endpoints[0].assumed_role(), InterfaceRole::Sender);
            assert!(details.url(&ModuleId::Credentials).is_some());
            assert_eq!(serde_json::to_string(&details).unwrap(), json);
        }
    }
}

pub use cdrs::Cdr;
pub use credentials::Credentials;
pub use locations::{Connector, Evse, Location};
pub use sessions::Session;
pub use tariffs::Tariff;
pub use tokens::Token;
pub use versions::{Endpoint, Version, VersionDetails};

/// The version number this module implements.
pub const VERSION: crate::VersionNumber = crate::VersionNumber::V2_1_1;
