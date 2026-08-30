//! The *Credentials* module of OCPI 2.3.0: the registration handshake.
//!
//! *Module Identifier: `credentials`* — required for all implementations.
//!
//! This module is symmetric: every platform both calls it and answers it. The token exchange it
//! performs — `CREDENTIALS_TOKEN_A` out of band, `B` in the POST, `C` in the response — is
//! modelled as a typestate in [`Registration`](crate::client::Registration), which makes the classic mistakes
//! (using `TOKEN_A` after registration, POSTing twice) unrepresentable.
//!
//! Spec: 2.3.0 §credentials_credentials_endpoint

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{
    CiString, CountryCode, Extensions, OcpiString, PartyId, PartyRef, Url, Validate, Validator, ViolationCode,
};

use super::locations::BusinessDetails;
use super::types::Role;

/// The credentials one platform gives another.
///
/// Spec: 2.3.0 §credentials_credentials_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Credentials {
    /// The credentials token for the other party to authenticate in your system.
    ///
    /// > *It should only contain printable non-whitespace ASCII characters, that is, characters
    /// > with Unicode code points from the range of U+0021 up to and including U+007E.*
    ///
    /// This is the token in cleartext — the value that goes, Base64-encoded, into the peer's
    /// `Authorization` header. [`CredentialsToken`](crate::transport::CredentialsToken) is the
    /// type to hold it in once it leaves this object: it redacts itself in `Debug`, compares in
    /// constant time and is zeroised on drop.
    pub token: OcpiString<64>,
    /// The URL to your API versions endpoint.
    pub url: Url,
    /// The Hub party of this platform, as a five-character `<country><party>` string.
    ///
    /// > *A Platform that supports Hub functionality with the Message routing headers SHALL give
    /// > the country code and party ID of the Hub in the `hub_party_id` field.*
    ///
    /// New in OCPI 2.3.0. Use [`PartyRef::from_hub_party_id`] to split it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_party_id: Option<CiString<5>>,
    /// The roles this platform provides. Cardinality `+`.
    ///
    /// > *NOTE: In OCPI 2.3.0, unlike in OCPI 2.2 or 2.2.1, Roaming Hubs' platforms are expected
    /// > to include the parties that are reachable through the Roaming Hub in the list in
    /// > `roles`.*
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

    /// The hub this platform routes through, if it declared one.
    #[must_use]
    pub fn hub_party(&self) -> Option<PartyRef> {
        self.hub_party_id.as_ref().and_then(|id| PartyRef::from_hub_party_id(id).ok())
    }

    /// Whether this platform advertises itself as a routing platform.
    ///
    /// A hub is recognised by the presence of `hub_party_id`, not by a role: OCPI 2.3.0 removed
    /// 2.2.1's `HUB` role value.
    #[must_use]
    pub fn is_routing_platform(&self) -> bool {
        self.hub_party_id.is_some()
    }
}

impl Validate for Credentials {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, token, url, hub_party_id, roles);

        if self.roles.is_empty() {
            v.report_at(
                "roles",
                ViolationCode::EmptyRequiredList,
                "Credentials has cardinality `+` roles: at least one is required",
            );
        }

        // "Every role needs a unique combination of: role, party_id and country_code."
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

        // "It should only contain printable non-whitespace ASCII characters, U+0021..U+007E."
        if let Some(bad) = self.token.as_str().chars().find(|c| !matches!(c, '!'..='~')) {
            v.report_at(
                "token",
                ViolationCode::IllegalCharacter,
                format!("a credentials token may only contain U+0021..U+007E; found U+{:04X}", bad as u32),
            );
        }
        if self.token.is_empty() {
            v.report_at("token", ViolationCode::IllegalCharacter, "a credentials token cannot be empty");
        }

        if let Some(hub) = self.hub_party_id.as_ref()
            && hub.len() != 5
        {
            v.report_at(
                "hub_party_id",
                ViolationCode::Inconsistent,
                "must be exactly five characters: a two-letter country code followed by a \
                     three-character party ID",
            );
        }
    }
}

/// One role a platform provides, with the party that fills it.
///
/// > *A platform can have the same role more than once, each with its own unique `party_id` and
/// > `country_code`, for example when a CPO provides 'white-label' services for 'virtual' CPOs.*
///
/// Spec: 2.3.0 §credentials_credentials_role_class
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

    fn role(role: Role, country: &str, party: &str) -> CredentialsRole {
        CredentialsRole::builder()
            .role(role)
            .business_details(BusinessDetails::builder().name("Example Operations").build())
            .party_id(party)
            .country_code(country)
            .build()
    }

    fn credentials(roles: Vec<CredentialsRole>) -> Credentials {
        Credentials::builder()
            .token("ebf3b399-779f-4497-9b9d-ac6ad3cc44d2")
            .url(Url::new("https://example.com/ocpi/versions").unwrap())
            .roles(roles)
            .build()
    }

    #[test]
    fn role_combinations_must_be_unique() {
        let ok = credentials(vec![role(Role::Cpo, "NL", "TNM"), role(Role::Emsp, "NL", "TNM")]);
        assert!(ok.validate().is_ok(), "the same party in two roles is allowed");

        let dup = credentials(vec![role(Role::Cpo, "NL", "TNM"), role(Role::Cpo, "nl", "tnm")]);
        let err = dup.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].pointer, "/roles/1", "party ids compare case-insensitively");
    }

    #[test]
    fn white_label_platforms_may_repeat_a_role() {
        let c = credentials(vec![
            role(Role::Cpo, "NL", "TNM"),
            role(Role::Cpo, "NL", "ABC"),
            role(Role::Cpo, "DE", "TNM"),
        ]);
        assert!(c.validate().is_ok());
        assert_eq!(c.parties().count(), 3);
        assert!(c.hosts(&PartyRef::new("de", "tnm").unwrap()));
    }

    #[test]
    fn the_token_charset_is_narrower_than_cistring() {
        let mut c = credentials(vec![role(Role::Cpo, "NL", "TNM")]);
        c.token = OcpiString::new("has a space").unwrap();
        let err = c.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].code, ViolationCode::IllegalCharacter);
    }

    #[test]
    fn a_hub_is_recognised_by_hub_party_id_not_by_a_role() {
        let mut c = credentials(vec![role(Role::Cpo, "NL", "TNM")]);
        assert!(!c.is_routing_platform());
        c.hub_party_id = Some(CiString::new("NLHUB").unwrap());
        assert!(c.is_routing_platform());
        assert_eq!(c.hub_party(), Some(PartyRef::new("NL", "HUB").unwrap()));
        assert!(c.validate().is_ok());
    }

    #[test]
    fn round_trips_the_spec_example() {
        let json = r#"{"token":"ebf3b399-779f-4497-9b9d-ac6ad3cc44d2","url":"https://example.com/ocpi/versions","roles":[{"role":"CPO","business_details":{"name":"Example Operator"},"party_id":"EXA","country_code":"NL"}]}"#;
        let c: Credentials = serde_json::from_str(json).unwrap();
        assert_eq!(c.roles[0].role, Role::Cpo);
        assert_eq!(serde_json::to_string(&c).unwrap(), json);
    }
}
