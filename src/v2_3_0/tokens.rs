//! The *Tokens* module of OCPI 2.3.0: which drivers may charge, and real-time authorization.
//!
//! *Module Identifier: `tokens`* — Data owner: eMSP.
//!
//! Spec: 2.3.0 §mod_tokens_tokens_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_open_enum;
use crate::types::validate_fields;
use crate::types::{
    CiString, ContractId, CountryCode, DateTime, DisplayText, Extensions, OcpiString, PartyId, PartyRef,
    Validate, Validator, ViolationCode,
};
use crate::{ocpi_enum, ocpi_lenient_enum};

use super::sessions::ProfileType;

/// A token an EV driver uses to authorize charging.
///
/// > *The combination of `uid` and `type` should be unique for every token within the eMSP's
/// > system.*
///
/// Spec: 2.3.0 §mod_tokens_token_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Token {
    /// ISO-3166 alpha-2 country code of the MSP that 'owns' this Token.
    pub country_code: CountryCode,
    /// ID of the eMSP that 'owns' this Token.
    pub party_id: PartyId,
    /// Unique ID by which this Token, combined with its type, can be identified.
    ///
    /// > *This is the field used by CPO system (RFID reader on the Charge Point) to identify
    /// > this token. … This field is named `uid` instead of `id` to prevent confusion with
    /// > `contract_id`.*
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
    ///
    /// > *Beware that OCPP 1.5/1.6 only support group_ids (parentId in OCPP 1.5/1.6) with a
    /// > maximum length of 20.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<CiString<36>>,
    /// Whether this Token is valid.
    pub valid: bool,
    /// What type of white-listing is allowed.
    ///
    /// > *NOTE: The eMSP is RECOMMENDED to push Tokens with type `AD_HOC_USER` or `APP_USER`
    /// > with `whitelist` set to `NEVER`. Whitelists are very useful for RFID type Tokens, but
    /// > the `AD_HOC_USER`/`APP_USER` Tokens are used to start Sessions from an App etc. so
    /// > whitelisting them has no advantages.*
    ///
    /// That is a recommendation, not a rule — the specification's own `APP_USER` example uses
    /// `ALLOWED` — so [`Validate`] does not report it. Ask
    /// [`Token::follows_whitelist_recommendation`] when you want to check it.
    pub whitelist: WhitelistType,
    /// Language Code ISO 639-1: the Token owner's preferred interface language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<OcpiString<2>>,
    /// The default Charging Preference profile type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile_type: Option<ProfileType>,
    /// The driver's own energy supplier/contract, where the Charge Point supports using it.
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

    /// Whether this Token follows the specification's advice on whitelisting.
    ///
    /// > *The eMSP is RECOMMENDED to push Tokens with type `AD_HOC_USER` or `APP_USER` with
    /// > `whitelist` set to `NEVER`.*
    ///
    /// A recommendation, not a rule: the spec's own `APP_USER` example does not follow it, so
    /// this is a query rather than a [`Validate`] violation.
    ///
    /// Spec: 2.3.0 §mod_tokens_tokentype_enum
    #[must_use]
    pub fn follows_whitelist_recommendation(&self) -> bool {
        !matches!(self.token_type, TokenType::AdHocUser | TokenType::AppUser)
            || self.whitelist == WhitelistType::Never
    }

    /// What a CPO should do with this Token when a driver presents it.
    ///
    /// This turns the `whitelist` field plus the CPO's current connectivity into the one decision
    /// a charging backend actually has to make. See [`AuthorizationDecision`].
    ///
    /// > *The validity of a Token has no influence on this. If a Token is `valid = false`, when
    /// > the `whitelist` field requires real-time authorization, the CPO SHALL do a real-time
    /// > authorization, the state of the Token might have changed.*
    ///
    /// Spec: 2.3.0 §mod_tokens_whitelisttype_enum
    #[must_use]
    pub fn authorization_decision(&self, online: bool) -> AuthorizationDecision {
        match self.whitelist {
            // "CPO shall always allow any use of this Token."
            WhitelistType::Always => AuthorizationDecision::AllowFromCache,
            // "The CPO may choose which version of authorization to use."
            WhitelistType::Allowed => {
                if online {
                    AuthorizationDecision::AuthorizeRealtime
                } else if self.valid {
                    AuthorizationDecision::AllowFromCache
                } else {
                    AuthorizationDecision::Deny
                }
            }
            // "In normal situations realtime authorization shall be used. But when the CPO cannot
            //  get a response from the eMSP … the CPO shall allow this Token to be used."
            WhitelistType::AllowedOffline => {
                if online {
                    AuthorizationDecision::AuthorizeRealtime
                } else {
                    AuthorizationDecision::AllowFromCache
                }
            }
            // "Whitelisting is forbidden, only realtime authorization is allowed."
            //
            // A `whitelist` value this version does not define lands here too: it is reported by
            // `Validate`, and until somebody acts on that report the safe reading is the
            // strictest one the enum offers — ask the eMSP, and deny when it cannot be asked.
            WhitelistType::Never | WhitelistType::Custom(_) => {
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
                "OCPP 1.5/1.6 only supports group IDs up to 20 characters; the spec advises \
                 staying within that as long as drivers may charge at such a Charge Point",
            );
        }
    }
}

/// What a CPO should do with a Token, given its whitelist setting and the current connectivity.
///
/// See [`Token::authorization_decision`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuthorizationDecision {
    /// Authorize from the locally cached Token, without contacting the eMSP.
    AllowFromCache,
    /// Perform a real-time authorization against the eMSP's Tokens Sender interface.
    AuthorizeRealtime,
    /// Refuse: whitelisting is forbidden for this Token and the eMSP cannot be reached.
    Deny,
}

/// The response to a real-time authorization request.
///
/// Spec: 2.3.0 §mod_tokens_authorizationinfo_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct AuthorizationInfo {
    /// Status of the Token, and whether charging is allowed at the optionally given location.
    pub allowed: AllowedType,
    /// The complete Token object for which this authorization was requested.
    pub token: Token,
    /// The location, if it was in the request and the driver may charge there.
    ///
    /// > *Only the EVSEs the EV driver is allowed to charge at are returned.*
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

/// References to a location and the EVSEs within it.
///
/// Spec: 2.3.0 §mod_tokens_locationreferences_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LocationReferences {
    /// Unique identifier for the location.
    pub location_id: CiString<36>,
    /// Unique identifiers for EVSEs within the given location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evse_uids: Vec<CiString<36>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for LocationReferences {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, location_id, evse_uids);
    }
}

/// A driver's own energy contract, for Charge Points that support using it.
///
/// > *NOTE: In a lot of countries it is currently not allowed/possible to use a driver's own
/// > energy supplier/contract at a Charge Point.*
///
/// Spec: 2.3.0 §mod_tokens_energy_contract
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EnergyContract {
    /// Name of the energy supplier for this token.
    pub supplier_name: OcpiString<64>,
    /// Contract ID at the energy supplier, belonging to the owner of this token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_id: Option<OcpiString<64>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for EnergyContract {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, supplier_name, contract_id);
    }
}

ocpi_enum! {
    /// The outcome of a real-time authorization.
    ///
    /// Spec: 2.3.0 §mod_tokens_allowed_enum
    pub enum AllowedType {
        /// This Token is allowed to charge (at this location).
        Allowed = "ALLOWED",
        /// This Token is blocked.
        Blocked = "BLOCKED",
        /// This Token has expired.
        Expired = "EXPIRED",
        /// The account has not enough credits to charge (at the given location).
        NoCredit = "NO_CREDIT",
        /// Token is valid, but is not allowed to charge at the given location.
        NotAllowed = "NOT_ALLOWED",
    }
}

ocpi_open_enum! {
    /// The type of a Token.
    ///
    /// Became an `OpenEnum` in OCPI 2.3.0, which also added `EMAID` for ISO 15118 Plug & Charge.
    ///
    /// > *NOTE: The eMSP is RECOMMENDED to not push Tokens with type `EMAID` at all. Exchanging
    /// > Token objects for EMAID Tokens is not necessary because the CPO already learns which
    /// > Party issued the Token from the Charging Station.*
    ///
    /// Spec: 2.3.0 §mod_tokens_tokentype_enum
    pub enum TokenType {
        /// One-time-use Token ID generated by a server or app.
        AdHocUser = "AD_HOC_USER",
        /// Token ID generated by a server or app to identify a user of an app.
        AppUser = "APP_USER",
        /// An EMAID, used when the Charging Station and vehicle speak ISO 15118.
        Emaid = "EMAID",
        /// Other type of token.
        Other = "OTHER",
        /// RFID Token.
        Rfid = "RFID",
    }
}

ocpi_lenient_enum! {
    /// When authorization of a Token by the CPO is allowed without asking the eMSP.
    ///
    /// Spec: 2.3.0 §mod_tokens_whitelisttype_enum
    pub enum WhitelistType {
        /// Token always has to be whitelisted; real-time authorization is not possible.
        Always = "ALWAYS",
        /// Whitelisting is allowed and so is real-time authorization; the CPO chooses.
        Allowed = "ALLOWED",
        /// Real-time authorization normally, whitelist only when the eMSP cannot be reached.
        AllowedOffline = "ALLOWED_OFFLINE",
        /// Whitelisting is forbidden; only real-time authorization is allowed.
        Never = "NEVER",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(whitelist: WhitelistType, valid: bool) -> Token {
        Token::builder()
            .country_code("NL")
            .party_id("TNM")
            .uid("012345678")
            .token_type(TokenType::Rfid)
            .contract_id("NL-TNM-C12345678-X")
            .issuer("TheNewMotion")
            .valid(valid)
            .whitelist(whitelist)
            .last_updated("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .build()
    }

    #[test]
    fn whitelist_semantics_become_one_decision() {
        use AuthorizationDecision::{AllowFromCache, AuthorizeRealtime, Deny};
        // ALWAYS: "CPO shall always allow any use of this Token", online or not, valid or not.
        for online in [true, false] {
            assert_eq!(token(WhitelistType::Always, false).authorization_decision(online), AllowFromCache);
        }
        // NEVER: only real-time; offline means no charging.
        assert_eq!(token(WhitelistType::Never, true).authorization_decision(true), AuthorizeRealtime);
        assert_eq!(token(WhitelistType::Never, true).authorization_decision(false), Deny);
        // ALLOWED_OFFLINE: real-time when possible, cache when not.
        assert_eq!(token(WhitelistType::AllowedOffline, false).authorization_decision(false), AllowFromCache);
        // ALLOWED: the CPO chooses; offline it falls back to the cached validity.
        assert_eq!(token(WhitelistType::Allowed, false).authorization_decision(false), Deny);
        assert_eq!(token(WhitelistType::Allowed, true).authorization_decision(false), AllowFromCache);
    }

    #[test]
    fn the_whitelist_recommendation_is_a_query_not_a_violation() {
        let mut t = token(WhitelistType::Allowed, true);
        t.token_type = TokenType::AppUser;
        assert!(!t.follows_whitelist_recommendation());
        // The spec's own APP_USER example uses ALLOWED, so this must not be a violation.
        assert!(t.validate().is_ok());
        t.whitelist = WhitelistType::Never;
        assert!(t.follows_whitelist_recommendation());
        assert!(token(WhitelistType::Always, true).follows_whitelist_recommendation());
    }

    #[test]
    fn long_group_ids_are_flagged_for_ocpp_compatibility() {
        let mut t = token(WhitelistType::Allowed, true);
        t.group_id = Some(CiString::new("G".repeat(21)).unwrap());
        assert!(t.validate().unwrap_err().as_slice().iter().any(|x| x.pointer == "/group_id"));
        t.group_id = Some(CiString::new("G".repeat(20)).unwrap());
        assert!(t.validate().is_ok());
    }

    #[test]
    fn round_trips_with_the_spec_field_names() {
        let json = r#"{"country_code":"NL","party_id":"TNM","uid":"012345678","type":"RFID","contract_id":"NL-TNM-C12345678-X","issuer":"TheNewMotion","valid":true,"whitelist":"ALWAYS","last_updated":"2018-12-10T17:16:15Z"}"#;
        let t: Token = serde_json::from_str(json).unwrap();
        assert_eq!(t.token_type, TokenType::Rfid);
        assert_eq!(serde_json::to_string(&t).unwrap(), json);
    }
}
