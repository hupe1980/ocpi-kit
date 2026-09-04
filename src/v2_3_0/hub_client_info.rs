//! The *Hub Client Info* module of OCPI 2.3.0: which parties a hub has connected.
//!
//! *Module Identifier: `hubclientinfo`* — Data owner: Hub.
//!
//! A configuration module, so its requests are **never** routed and carry no `OCPI-to-*` or
//! `OCPI-from-*` headers.
//!
//! Spec: 2.3.0 §mod_hub_client_info_module

use serde::{Deserialize, Serialize};

use crate::ocpi_lenient_enum;
use crate::types::validate_fields;
use crate::types::{CountryCode, DateTime, Extensions, PartyId, PartyRef, Validate, Validator};

use super::types::Role;

/// The connection status of one party at a hub.
///
/// Spec: 2.3.0 §mod_hub_client_info_hub_client_info_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ClientInfo {
    /// CPO or eMSP ID of this party, as used in the credentials exchange.
    pub party_id: PartyId,
    /// Country code of the country this party is operating in.
    pub country_code: CountryCode,
    /// The role of the connected party.
    pub role: Role,
    /// Status of the connection to the party.
    pub status: ConnectionStatus,
    /// Timestamp when this ClientInfo object was last updated.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl ClientInfo {
    /// Creates a client info entry.
    #[must_use]
    pub fn new(party: PartyRef, role: Role, status: ConnectionStatus, last_updated: DateTime) -> Self {
        Self {
            party_id: party.party_id,
            country_code: party.country_code,
            role,
            status,
            last_updated,
            extensions: Extensions::new(),
        }
    }

    /// The party this entry is about.
    #[must_use]
    pub fn party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }

    /// Whether messages can currently be delivered to this party.
    #[must_use]
    pub fn is_reachable(&self) -> bool {
        self.status == ConnectionStatus::Connected
    }
}

impl Validate for ClientInfo {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, party_id, country_code, role, status, last_updated);
    }
}

ocpi_lenient_enum! {
    /// The state of a hub's connection to one party.
    ///
    /// Spec: 2.3.0 §mod_hub_client_info_hub_connection_type_enum
    pub enum ConnectionStatus {
        /// Party is connected.
        Connected = "CONNECTED",
        /// Party is currently not connected.
        Offline = "OFFLINE",
        /// Connection to this party is planned, but has never been connected.
        Planned = "PLANNED",
        /// Party is no longer active and will never connect again.
        Suspended = "SUSPENDED",
    }
}

impl ConnectionStatus {
    /// Whether a still-alive check should be attempted against this party.
    ///
    /// A `PLANNED` party has never connected and a `SUSPENDED` one never will, so polling either
    /// is wasted work.
    #[must_use]
    pub fn should_poll(&self) -> bool {
        matches!(self, Self::Connected | Self::Offline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_connected_parties_are_reachable() {
        let info = ClientInfo::new(
            PartyRef::new("NL", "TNM").unwrap(),
            Role::Cpo,
            ConnectionStatus::Offline,
            "2019-06-24T12:39:09Z".parse().unwrap(),
        );
        assert!(!info.is_reachable());
        assert!(info.status.should_poll(), "an offline party may come back");
        assert!(!ConnectionStatus::Suspended.should_poll());
        assert!(!ConnectionStatus::Planned.should_poll());
    }

    #[test]
    fn round_trips_the_spec_shape() {
        let json = r#"{"party_id":"TNM","country_code":"NL","role":"CPO","status":"CONNECTED","last_updated":"2019-06-24T12:39:09Z"}"#;
        let info: ClientInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.party(), PartyRef::new("NL", "TNM").unwrap());
        assert_eq!(serde_json::to_string(&info).unwrap(), json);
    }
}
