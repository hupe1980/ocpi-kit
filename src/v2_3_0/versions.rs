//! The *Versions* module of OCPI 2.3.0: the starting point of every OCPI connection.
//!
//! *Module Identifier: `versions`* — required for all implementations.
//!
//! Spec: 2.3.0 §versions_module

use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{Extensions, Url, Validate, Validator, ViolationCode};
use crate::{InterfaceRole, ModuleId, VersionNumber};

/// One supported OCPI version and where to find its details.
///
/// Spec: 2.3.0 §version_information_endpoint_version_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Version {
    /// The version number.
    pub version: VersionNumber,
    /// URL to the endpoint containing version specific information.
    pub url: Url,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Version {
    /// Creates a version entry.
    #[must_use]
    pub fn new(version: VersionNumber, url: Url) -> Self {
        Self { version, url, extensions: Extensions::new() }
    }
}

impl Validate for Version {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, version, url);
    }
}

/// The endpoints a party implements for one version.
///
/// Spec: 2.3.0 §version_information_get_details_endpoint_data
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VersionDetails {
    /// The version number these endpoints belong to.
    pub version: VersionNumber,
    /// The supported endpoints for this version. Cardinality `+`.
    pub endpoints: Vec<Endpoint>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl VersionDetails {
    /// Creates version details.
    #[must_use]
    pub fn new(version: VersionNumber, endpoints: Vec<Endpoint>) -> Self {
        Self { version, endpoints, extensions: Extensions::new() }
    }

    /// The endpoint for a module and interface role, if the peer implements it.
    ///
    /// Module identifiers are matched case-insensitively, which matters for the `Booking` module:
    /// the spec writes it in mixed case and implementations differ. See
    /// [`ModuleId::matches`].
    #[must_use]
    pub fn endpoint(&self, module: &ModuleId, role: InterfaceRole) -> Option<&Endpoint> {
        self.endpoints.iter().find(|e| e.identifier.matches(module) && e.role == role)
    }

    /// The URL of a module's endpoint for one interface role.
    #[must_use]
    pub fn url(&self, module: &ModuleId, role: InterfaceRole) -> Option<&Url> {
        self.endpoint(module, role).map(|e| &e.url)
    }

    /// The credentials endpoint.
    ///
    /// > *NOTE: for the `credentials` module, the value of the role property is not relevant as
    /// > this module is the same for all roles. It is advised to send "SENDER" as the
    /// > InterfaceRole for one's own credentials endpoint and to disregard the value of the role
    /// > property of the Endpoint object for other platforms' credentials modules.*
    ///
    /// So this ignores the role entirely, as the spec instructs.
    #[must_use]
    pub fn credentials_url(&self) -> Option<&Url> {
        self.endpoints.iter().find(|e| e.identifier.matches(&ModuleId::Credentials)).map(|e| &e.url)
    }

    /// Whether the peer implements every module in `required`, in the given role.
    ///
    /// > *In case the Sender (starting the credentials exchange process) cannot find the
    /// > endpoints it expects, it is expected NOT to send the POST request with credentials to
    /// > the Receiver.*
    ///
    /// Spec: 2.3.0 §credentials_required_endpoints_not_available
    #[must_use]
    pub fn missing(&self, required: &[(ModuleId, InterfaceRole)]) -> Vec<(ModuleId, InterfaceRole)> {
        required.iter().filter(|(m, r)| self.endpoint(m, *r).is_none()).cloned().collect()
    }
}

impl Validate for VersionDetails {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, version, endpoints);
        if self.endpoints.is_empty() {
            v.report_at(
                "endpoints",
                ViolationCode::EmptyRequiredList,
                "version details have cardinality `+` endpoints: at least one is required",
            );
        }
        // The credentials module is "Required for all implementations".
        if !self.endpoints.iter().any(|e| e.identifier.matches(&ModuleId::Credentials)) {
            v.report_at(
                "endpoints",
                ViolationCode::MissingConditional,
                "the `credentials` module is required for all implementations",
            );
        }
        let mut seen: Vec<(&ModuleId, InterfaceRole)> = Vec::new();
        for (i, e) in self.endpoints.iter().enumerate() {
            let key = (&e.identifier, e.role);
            if seen.contains(&key) {
                v.enter("endpoints");
                v.enter(&i.to_string());
                v.report(
                    ViolationCode::Inconsistent,
                    format!("{} / {} is listed more than once", e.identifier, e.role),
                );
                v.leave();
                v.leave();
            }
            seen.push(key);
            if !e.identifier.exists_in(&self.version) {
                v.enter("endpoints");
                v.enter(&i.to_string());
                v.report_at(
                    "identifier",
                    ViolationCode::Inconsistent,
                    format!("the {} module does not exist in OCPI {}", e.identifier, self.version),
                );
                v.leave();
                v.leave();
            }
        }
    }
}

/// One module endpoint of a party, for one interface role.
///
/// Spec: 2.3.0 §version_information_endpoint_endpoint_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Endpoint {
    /// Endpoint identifier.
    pub identifier: ModuleId,
    /// Interface role this endpoint implements.
    pub role: InterfaceRole,
    /// URL to the endpoint.
    pub url: Url,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Endpoint {
    /// Creates an endpoint entry.
    #[must_use]
    pub fn new(identifier: ModuleId, role: InterfaceRole, url: Url) -> Self {
        Self { identifier, role, url, extensions: Extensions::new() }
    }
}

impl Validate for Endpoint {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, identifier, role, url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(path: &str) -> Url {
        Url::new(format!("https://example.com/ocpi/cpo/2.3.0/{path}")).unwrap()
    }

    fn details(endpoints: Vec<Endpoint>) -> VersionDetails {
        VersionDetails::new(VersionNumber::V2_3_0, endpoints)
    }

    #[test]
    fn endpoint_lookup_ignores_the_role_for_credentials() {
        let d = details(vec![
            Endpoint::new(ModuleId::Credentials, InterfaceRole::Receiver, url("credentials")),
            Endpoint::new(ModuleId::Locations, InterfaceRole::Sender, url("locations")),
        ]);
        // The spec says to disregard the role on the credentials endpoint.
        assert_eq!(d.credentials_url(), Some(&url("credentials")));
        assert_eq!(d.url(&ModuleId::Locations, InterfaceRole::Sender), Some(&url("locations")));
        assert_eq!(d.url(&ModuleId::Locations, InterfaceRole::Receiver), None);
    }

    #[test]
    fn missing_required_endpoints_are_listed_for_the_handshake() {
        let d =
            details(vec![Endpoint::new(ModuleId::Credentials, InterfaceRole::Sender, url("credentials"))]);
        let missing = d.missing(&[
            (ModuleId::Credentials, InterfaceRole::Sender),
            (ModuleId::Cdrs, InterfaceRole::Receiver),
        ]);
        assert_eq!(missing, vec![(ModuleId::Cdrs, InterfaceRole::Receiver)]);
    }

    #[test]
    fn a_module_that_does_not_exist_in_the_version_is_reported() {
        let d = VersionDetails::new(
            VersionNumber::V2_1_1,
            vec![
                Endpoint::new(ModuleId::Credentials, InterfaceRole::Sender, url("credentials")),
                Endpoint::new(ModuleId::ChargingProfiles, InterfaceRole::Receiver, url("cp")),
            ],
        );
        let err = d.validate().unwrap_err();
        assert!(err.as_slice().iter().any(|x| x.pointer == "/endpoints/1/identifier"), "{err}");
    }

    #[test]
    fn credentials_is_required_in_version_details() {
        let d = details(vec![Endpoint::new(ModuleId::Locations, InterfaceRole::Sender, url("locations"))]);
        assert!(
            d.validate().unwrap_err().as_slice().iter().any(|x| x.code == ViolationCode::MissingConditional)
        );
    }

    #[test]
    fn unknown_modules_and_versions_survive_discovery() {
        let json = r#"{"version":"3.0","endpoints":[{"identifier":"credentials","role":"SENDER","url":"https://example.com/ocpi/3.0/credentials"},{"identifier":"nltnm-tokens","role":"RECEIVER","url":"https://example.com/ocpi/3.0/x"}]}"#;
        let d: VersionDetails = serde_json::from_str(json).unwrap();
        assert!(!d.version.is_known());
        assert!(!d.endpoints[1].identifier.is_known());
        assert_eq!(serde_json::to_string(&d).unwrap(), json);
    }
}
