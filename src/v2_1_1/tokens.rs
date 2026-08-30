//! The *Tokens* module of OCPI 2.1.1.
//!
//! The 2.1.1 Token is much smaller: no owner fields, no `contract_id` (its `auth_id` plays that
//! role), no `group_id`, no energy contract and no default charging profile. `TokenType` has two
//! values.
//!
//! Spec: 2.1.1 §mod_tokens

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_lenient_enum;
use crate::types::validate_fields;
use crate::types::{DateTime, DisplayText, Extensions, OcpiString, Validate, Validator, ViolationCode};

/// A token an EV driver uses to authorize charging, in OCPI 2.1.1.
///
/// Spec: 2.1.1 §mod_tokens_token_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Token {
    /// Identification used by the CPO system to identify this token.
    pub uid: OcpiString<36>,
    /// Type of the token.
    #[serde(rename = "type")]
    pub token_type: TokenType,
    /// Uniquely identifies the EV driver contract token within the eMSP's platform.
    ///
    /// Renamed to `contract_id` in OCPI 2.2, where `auth_id` disappeared entirely.
    pub auth_id: OcpiString<36>,
    /// Visual readable number/identification as printed on the Token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_number: Option<OcpiString<64>>,
    /// Issuing company, most of the time the name printed on the token.
    pub issuer: OcpiString<64>,
    /// Whether this Token is valid.
    pub valid: bool,
    /// What type of white-listing is allowed.
    pub whitelist: WhitelistType,
    /// Language Code ISO 639-1: the Token owner's preferred interface language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<OcpiString<2>>,
    /// Timestamp when this Token was last updated (or created).
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Token {
    /// What a CPO should do with this Token when a driver presents it.
    ///
    /// The whitelist semantics are unchanged across every OCPI version; see
    /// [`v2_3_0::tokens::Token::authorization_decision`](crate::v2_3_0::tokens::Token::authorization_decision).
    #[must_use]
    pub fn authorization_decision(&self, online: bool) -> AuthorizationDecision {
        match self.whitelist {
            WhitelistType::Always => AuthorizationDecision::AllowFromCache,
            WhitelistType::Allowed => {
                if online {
                    AuthorizationDecision::AuthorizeRealtime
                } else if self.valid {
                    AuthorizationDecision::AllowFromCache
                } else {
                    AuthorizationDecision::Deny
                }
            }
            WhitelistType::AllowedOffline => {
                if online {
                    AuthorizationDecision::AuthorizeRealtime
                } else {
                    AuthorizationDecision::AllowFromCache
                }
            }
            WhitelistType::Never => {
                if online {
                    AuthorizationDecision::AuthorizeRealtime
                } else {
                    AuthorizationDecision::Deny
                }
            }
        }
    }
}

impl Validate for Token {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self, v, uid, token_type as "type", auth_id, visual_number, issuer, whitelist,
            language, last_updated,
        );
    }
}

/// The response to a real-time authorization request, in OCPI 2.1.1.
///
/// Carries no `token` and no `authorization_reference`: both arrived in OCPI 2.2.
///
/// Spec: 2.1.1 §mod_tokens_authorizationinfo_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct AuthorizationInfo {
    /// Status of the Token, and whether charging is allowed at the optionally given location.
    pub allowed: AllowedType,
    /// The location, if it was in the request and the driver may charge there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<LocationReferences>,
    /// Additional information to display to the EV driver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<DisplayText>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for AuthorizationInfo {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, allowed, location, info);
        if self.allowed != AllowedType::Allowed && self.location.is_some() {
            v.report_at(
                "location",
                ViolationCode::Inconsistent,
                "a location is only returned when the driver is allowed to charge there",
            );
        }
    }
}

/// References to a location, its EVSEs and their connectors.
///
/// OCPI 2.2 dropped `connector_ids`: authorization is per EVSE from there on.
///
/// Spec: 2.1.1 §mod_tokens_locationreferences_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LocationReferences {
    /// Unique identifier for the location.
    pub location_id: OcpiString<39>,
    /// Unique identifiers for EVSEs within the given location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evse_uids: Vec<OcpiString<39>>,
    /// Identifies the connectors within the given EVSEs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connector_ids: Vec<OcpiString<36>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for LocationReferences {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, location_id, evse_uids, connector_ids);
        if !self.connector_ids.is_empty() && self.evse_uids.is_empty() {
            v.report_at(
                "evse_uids",
                ViolationCode::MissingConditional,
                "connectors are identified within an EVSE, so naming connectors without naming \
                 the EVSE they belong to is ambiguous",
            );
        }
    }
}

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::tokens::{AllowedType, AuthorizationDecision, WhitelistType};

ocpi_lenient_enum! {
    /// The type of a Token, in OCPI 2.1.1.
    ///
    /// Two values. `APP_USER` and `AD_HOC_USER` arrived in OCPI 2.2, `EMAID` in 2.3.0.
    ///
    /// Spec: 2.1.1 §mod_tokens_tokentype_enum
    pub enum TokenType {
        /// Other type of token.
        Other = "OTHER",
        /// RFID Token.
        Rfid = "RFID",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_2_1_1_token_type_has_two_values() {
        assert_eq!(TokenType::ALL_KNOWN.len(), 2);
        let app_user: TokenType = "APP_USER".into();
        assert!(!app_user.is_known(), "APP_USER arrived in OCPI 2.2");
        assert_eq!(serde_json::to_string(&app_user).unwrap(), "\"APP_USER\"");
    }

    #[test]
    fn a_2_1_1_token_round_trips() {
        let json = r#"{"uid":"012345678","type":"RFID","auth_id":"DE8ACC12E46L89","visual_number":"DF000-2001-8999","issuer":"TheNewMotion","valid":true,"whitelist":"ALLOWED","last_updated":"2018-12-10T17:16:15Z"}"#;
        let token: Token = serde_json::from_str(json).unwrap();
        assert!(token.validate().is_ok());
        assert_eq!(serde_json::to_string(&token).unwrap(), json);
    }

    #[test]
    fn naming_connectors_without_their_evse_is_ambiguous() {
        let refs = LocationReferences {
            location_id: OcpiString::new("LOC1").unwrap(),
            evse_uids: Vec::new(),
            connector_ids: vec![OcpiString::new("1").unwrap()],
            extensions: Extensions::new(),
        };
        assert_eq!(refs.validate().unwrap_err().as_slice()[0].pointer, "/evse_uids");
    }
}
