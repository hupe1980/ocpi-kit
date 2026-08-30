//! The *Credentials* module of OCPI 2.1.1.
//!
//! The 2.1.1 credentials object is **flat**: one party, one role — implied by which interface the
//! connection is on rather than stated. OCPI 2.2 replaced this with the `roles` list, which is
//! what made platforms hosting several parties expressible at all.
//!
//! Spec: 2.1.1 §credentials

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{Extensions, OcpiString, PartyRef, Url, Validate, Validator, ViolationCode};

use super::locations::BusinessDetails;

/// The credentials one party gives another, in OCPI 2.1.1.
///
/// Spec: 2.1.1 §credentials_credentials_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Credentials {
    /// The token for the other party to authenticate in your system.
    pub token: OcpiString<64>,
    /// The URL to your API versions endpoint.
    pub url: Url,
    /// Details of this party.
    pub business_details: BusinessDetails,
    /// CPO or eMSP ID of this party.
    pub party_id: OcpiString<3>,
    /// Country code of the country this party is operating in.
    pub country_code: OcpiString<2>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Credentials {
    /// The single party these credentials speak for.
    ///
    /// # Errors
    ///
    /// Returns [`crate::types::InvalidString`] if the two fields are not a usable party
    /// reference, which can happen for a value that came off the wire.
    pub fn party(&self) -> Result<PartyRef, crate::types::InvalidString> {
        PartyRef::new(self.country_code.as_str(), self.party_id.as_str())
    }
}

impl Validate for Credentials {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, token, url, business_details, party_id, country_code);
        if let Some(bad) = self.token.as_str().chars().find(|c| !matches!(c, '!'..='~')) {
            v.report_at(
                "token",
                ViolationCode::IllegalCharacter,
                format!("a credentials token may only contain U+0021..U+007E; found U+{:04X}", bad as u32),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_2_1_1_credentials_object_is_flat() {
        let json = r#"{"token":"ebf3b399-779f-4497-9b9d-ac6ad3cc44d2","url":"https://example.com/ocpi/cpo/","business_details":{"name":"Example Operator"},"party_id":"EXA","country_code":"NL"}"#;
        let credentials: Credentials = serde_json::from_str(json).unwrap();
        assert!(credentials.validate().is_ok());
        assert_eq!(credentials.party().unwrap().to_string(), "NL/EXA");
        assert_eq!(serde_json::to_string(&credentials).unwrap(), json);
        // There is no `roles` list here; a 2.2-style body keeps it in extensions.
        assert!(!json.contains("roles"));
    }
}
