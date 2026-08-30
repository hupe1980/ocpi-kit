//! The *Credentials* module of OCPI 2.2.1, as a delta from
//! [`v2_3_0::credentials`](crate::v2_3_0::credentials).
//!
//! Two differences, both about hubs:
//!
//! * there is no `hub_party_id` — a hub identifies itself with the `HUB`
//!   [`Role`] instead, which 2.3.0 removed;
//! * a hub's `roles` list does **not** include the parties reachable through it, which is what
//!   2.3.0's note about Roaming Hubs changed.
//!
//! Spec: 2.2.1 §credentials_credentials_endpoint

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    CountryCode, Extensions, OcpiString, PartyId, PartyRef, Url, Validate, Validator, ViolationCode,
};

use super::locations::BusinessDetails;
use super::types::Role;

/// The credentials one platform gives another, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §credentials_credentials_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Credentials {
    /// The credentials token for the other party to authenticate in your system.
    pub token: OcpiString<64>,
    /// The URL to your API versions endpoint.
    pub url: Url,
    /// The roles this party provides. Cardinality `+`.
    pub roles: Vec<CredentialsRole>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Credentials {
    /// Every party this platform speaks for.
    pub fn parties(&self) -> impl Iterator<Item = PartyRef> + '_ {
        self.roles.iter().map(CredentialsRole::party)
    }

    /// Whether this platform hosts the given party.
    #[must_use]
    pub fn hosts(&self, party: &PartyRef) -> bool {
        self.roles.iter().any(|r| &r.party() == party)
    }

    /// Whether this platform declares itself a hub.
    ///
    /// In 2.2.1 that is the `HUB` role. OCPI 2.3.0 removed the role and uses
    /// `Credentials.hub_party_id` instead; see
    /// [`v2_3_0::credentials::Credentials::is_routing_platform`](crate::v2_3_0::credentials::Credentials::is_routing_platform).
    #[must_use]
    pub fn is_hub(&self) -> bool {
        self.roles.iter().any(|r| r.role == Role::Hub)
    }

    /// The party that fills the `HUB` role, if any.
    #[must_use]
    pub fn hub_party(&self) -> Option<PartyRef> {
        self.roles.iter().find(|r| r.role == Role::Hub).map(CredentialsRole::party)
    }
}

impl Validate for Credentials {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, token, url, roles);
        if self.roles.is_empty() {
            v.report_at(
                "roles",
                ViolationCode::EmptyRequiredList,
                "Credentials has cardinality `+` roles: at least one is required",
            );
        }
        let mut seen: Vec<(Role, PartyRef)> = Vec::new();
        for (i, role) in self.roles.iter().enumerate() {
            let key = (role.role, role.party());
            if seen.contains(&key) {
                v.enter("roles");
                v.enter(&i.to_string());
                v.report(
                    ViolationCode::Inconsistent,
                    format!(
                        "the combination {} / {} appears more than once; every role needs a \
                         unique combination of role, party_id and country_code",
                        key.0, key.1
                    ),
                );
                v.leave();
                v.leave();
            }
            seen.push(key);
        }
        if let Some(bad) = self.token.as_str().chars().find(|c| !matches!(c, '!'..='~')) {
            v.report_at(
                "token",
                ViolationCode::IllegalCharacter,
                format!("a credentials token may only contain U+0021..U+007E; found U+{:04X}", bad as u32),
            );
        }
    }
}

/// One role a platform provides, with the party that fills it, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §credentials_credentials_role_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct CredentialsRole {
    /// Type of role.
    pub role: Role,
    /// Details of this party.
    pub business_details: BusinessDetails,
    /// CPO, eMSP (or other role) ID of this party.
    pub party_id: PartyId,
    /// ISO-3166 alpha-2 country code of the country this party is operating in.
    pub country_code: CountryCode,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl CredentialsRole {
    /// The party filling this role.
    #[must_use]
    pub fn party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }
}

impl Validate for CredentialsRole {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, role, business_details, party_id, country_code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hub_is_a_role_here_not_a_field() {
        let c = Credentials::builder()
            .token("ebf3b399-779f-4497-9b9d-ac6ad3cc44d2")
            .url(Url::new("https://hub.example.com/ocpi/versions").unwrap())
            .roles(vec![
                CredentialsRole::builder()
                    .role(Role::Hub)
                    .business_details(BusinessDetails::builder().name("Example Hub").build())
                    .party_id("HUB")
                    .country_code("NL")
                    .build(),
            ])
            .build();
        assert!(c.is_hub());
        assert_eq!(c.hub_party(), Some(PartyRef::new("NL", "HUB").unwrap()));
        assert!(c.validate().is_ok());
        // No `hub_party_id` field exists in 2.2.1, so it would land in extensions.
        assert!(!serde_json::to_string(&c).unwrap().contains("hub_party_id"));
    }
}
