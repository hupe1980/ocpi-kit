//! The *Hub Client Info* module of OCPI 2.2.1, as a delta from
//! [`v2_3_0::hub_client_info`](crate::v2_3_0::hub_client_info).
//!
//! Only [`ClientInfo`] is redefined, because its `role` is the 2.2.1
//! [`Role`], which still has `HUB`.
//!
//! **Spec erratum.** The Sender GET endpoint structure in
//! §mod_hub_client_info of both 2.2.1 and 2.3.0 is written as `{locations_endpoint_url}?…`,
//! copy-pasted from the Locations module, and the Receiver PUT example uses the path
//! `/ocpi/cpo/2.0/clientinfo/…` — the wrong version and the wrong module identifier. The
//! endpoint URL to use is the one discovered for `hubclientinfo`; examples are not normative.
//!
//! Spec: 2.2.1 §mod_hub_client_info_module

use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{CountryCode, DateTime, Extensions, PartyId, PartyRef, Validate, Validator};

use super::types::Role;

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::hub_client_info::ConnectionStatus;

/// The connection status of one party at a hub, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_hub_client_info_hub_client_info_object
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
