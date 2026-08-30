//! The *Tokens* module of OCPI 2.2.1, as a delta from [`v2_3_0::tokens`](crate::v2_3_0::tokens).
//!
//! The only change 2.3.0 made is [`TokenType`]: it gained `EMAID`, for ISO 15118 Plug & Charge,
//! and became an `OpenEnum`. [`Token`] and [`AuthorizationInfo`] are redefined here only because
//! they carry that type.
//!
//! Spec: 2.2.1 §mod_tokens_tokens_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_lenient_enum;
use crate::types::validate_fields;
use crate::types::{
    CiString, ContractId, CountryCode, DateTime, DisplayText, Extensions, OcpiString, PartyId, PartyRef,
    Validate, Validator, ViolationCode,
};

use super::sessions::ProfileType;

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::tokens::{
    AllowedType, AuthorizationDecision, EnergyContract, LocationReferences, WhitelistType,
};

/// A token an EV driver uses to authorize charging, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_tokens_token_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Token {
    /// ISO-3166 alpha-2 country code of the MSP that 'owns' this Token.
    pub country_code: CountryCode,
    /// ID of the eMSP that 'owns' this Token.
    pub party_id: PartyId,
    /// Unique ID by which this Token, combined with its type, can be identified.
    pub uid: CiString<36>,
    /// Type of the token.
    #[serde(rename = "type")]
    pub token_type: TokenType,
    /// Uniquely identifies the EV driver contract token within the eMSP's platform.
    pub contract_id: ContractId,
    /// Visual readable number/identification as printed on the Token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_number: Option<OcpiString<64>>,
    /// Issuing company, most of the time the name printed on the token.
    pub issuer: OcpiString<64>,
    /// Groups a couple of tokens so a session started with one can be stopped with another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<CiString<36>>,
    /// Whether this Token is valid.
    pub valid: bool,
    /// What type of white-listing is allowed.
    pub whitelist: WhitelistType,
    /// Language Code ISO 639-1: the Token owner's preferred interface language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<OcpiString<2>>,
    /// The default Charging Preference profile type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile_type: Option<ProfileType>,
    /// The driver's own energy supplier/contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_contract: Option<EnergyContract>,
    /// Timestamp when this Token was last updated (or created).
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Token {
    /// The eMSP that owns this Token.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }

    /// What a CPO should do with this Token when a driver presents it.
    ///
    /// See [`v2_3_0::tokens::Token::authorization_decision`](crate::v2_3_0::tokens::Token::authorization_decision);
    /// the whitelist semantics are identical in both versions.
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
            self, v, country_code, party_id, uid, token_type as "type", contract_id,
            visual_number, issuer, group_id, whitelist, language, default_profile_type,
            energy_contract, last_updated,
        );
        if self.group_id.as_ref().is_some_and(|g| g.len() > 20) {
            v.report_at(
                "group_id",
                ViolationCode::Inconsistent,
                "OCPP 1.5/1.6 only supports group IDs up to 20 characters",
            );
        }
    }
}

/// The response to a real-time authorization request, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_tokens_authorizationinfo_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct AuthorizationInfo {
    /// Status of the Token, and whether charging is allowed at the optionally given location.
    pub allowed: AllowedType,
    /// The complete Token object for which this authorization was requested.
    pub token: Token,
    /// The location, if it was in the request and the driver may charge there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<LocationReferences>,
    /// Reference to the authorization, echoed later in the relevant Session and CDR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_reference: Option<CiString<36>>,
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
        validate_fields!(self, v, allowed, token, location, authorization_reference, info);
        if self.allowed != AllowedType::Allowed && self.location.is_some() {
            v.report_at(
                "location",
                ViolationCode::Inconsistent,
                "a location is only returned when the driver is allowed to charge there",
            );
        }
    }
}

ocpi_lenient_enum! {
    /// The type of a Token, in OCPI 2.2.1.
    ///
    /// OCPI 2.3.0 added `EMAID` and made the enum open. See [`ocpi_lenient_enum!`] for why an
    /// unrecognised value is still decoded here.
    ///
    /// Spec: 2.2.1 §mod_tokens_tokentype_enum
    pub enum TokenType {
        /// One-time-use Token ID generated by a server or app.
        AdHocUser = "AD_HOC_USER",
        /// Token ID generated by a server or app to identify a user of an app.
        AppUser = "APP_USER",
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
    fn emaid_arrived_in_2_3_0() {
        assert_eq!(TokenType::ALL_KNOWN.len(), 4);
        let emaid: TokenType = "EMAID".into();
        assert!(!emaid.is_known());
        assert!(emaid.validate().is_err(), "2.2.1 declares TokenType closed");
        assert_eq!(serde_json::to_string(&emaid).unwrap(), "\"EMAID\"", "but the value survives");
    }

    #[test]
    fn whitelist_semantics_are_unchanged_between_versions() {
        let t = Token::builder()
            .country_code("NL")
            .party_id("TNM")
            .uid("012345678")
            .token_type(TokenType::Rfid)
            .contract_id("NL-TNM-C12345678-X")
            .issuer("TheNewMotion")
            .valid(true)
            .whitelist(WhitelistType::Never)
            .last_updated("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .build();
        assert_eq!(t.authorization_decision(false), AuthorizationDecision::Deny);
        assert_eq!(t.authorization_decision(true), AuthorizationDecision::AuthorizeRealtime);
        assert!(t.validate().is_ok());
    }
}
