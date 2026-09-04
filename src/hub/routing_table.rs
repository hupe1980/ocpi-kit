//! Which platform hosts which party, and whether it is reachable.
//!
//! Locks are taken poison-tolerantly: the table is plain data, and a hub that answered every
//! request with a panic because one earlier request panicked would be a worse failure than the
//! one that started it. See [`server::auth`](crate::server::auth).

use std::collections::BTreeMap;
use std::sync::RwLock;

use crate::client::Peer;
use crate::transport::{OcpiError, StatusCode};
use crate::types::PartyRef;
use crate::v2_3_0::hub_client_info::ConnectionStatus;
use crate::v2_3_0::types::Role;
use crate::{InterfaceRole, ModuleId};

/// One platform connected to the hub, and the parties it speaks for.
#[derive(Debug)]
pub struct ConnectedPlatform {
    /// A stable identifier for the platform.
    pub platform_id: String,
    /// How to call it.
    pub peer: Peer,
    /// The parties it hosts, with the role each fills.
    pub parties: Vec<(PartyRef, Role)>,
    /// Whether messages can currently be delivered to it.
    pub status: ConnectionStatus,
}

impl ConnectedPlatform {
    /// Whether this platform hosts `party`.
    #[must_use]
    pub fn hosts(&self, party: &PartyRef) -> bool {
        self.parties.iter().any(|(p, _)| p == party)
    }

    /// The role `party` fills at this platform.
    #[must_use]
    pub fn role_of(&self, party: &PartyRef) -> Option<Role> {
        self.parties.iter().find(|(p, _)| p == party).map(|(_, r)| *r)
    }

    /// Whether messages can currently be delivered.
    #[must_use]
    pub fn is_reachable(&self) -> bool {
        self.status == ConnectionStatus::Connected
    }

    /// Whether this platform implements a module in a given interface role.
    #[must_use]
    pub fn implements(&self, module: &ModuleId, role: InterfaceRole) -> bool {
        self.peer.implements(module, role)
    }
}

/// The hub's map from party to platform.
///
/// A hub's whole job is answering "who is `NL/TNM`, and can I reach them right now?", and turning
/// a "no" into the right one of the four `4xxx` codes:
///
/// | Code | Meaning |
/// |---|---|
/// | `4001` | Unknown receiver: the `OCPI-to-*` address is unknown |
/// | `4002` | Timeout on a forwarded request |
/// | `4003` | Connection problem: the receiving party is not connected |
/// | `4000` | Anything else |
///
/// Spec: 2.3.0 §status_codes_4xxx_hub_errors
#[derive(Debug, Default)]
pub struct RoutingTable {
    platforms: RwLock<BTreeMap<String, ConnectedPlatform>>,
}

impl RoutingTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a platform.
    pub fn upsert(&self, platform: ConnectedPlatform) {
        self.platforms
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(platform.platform_id.clone(), platform);
    }

    /// Removes a platform, as an unregistration does.
    pub fn remove(&self, platform_id: &str) -> bool {
        self.platforms
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(platform_id)
            .is_some()
    }

    /// Records a platform's connection status, which the `hubclientinfo` module publishes.
    pub fn set_status(&self, platform_id: &str, status: ConnectionStatus) -> bool {
        let mut platforms = self.platforms.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        match platforms.get_mut(platform_id) {
            Some(platform) => {
                platform.status = status;
                true
            }
            None => false,
        }
    }

    /// Runs `f` over the platform hosting `party`.
    ///
    /// # Errors
    ///
    /// Returns `4001 Unknown receiver` when no platform hosts the party, and `4003 Connection
    /// problem` when the platform is known but not connected.
    pub fn with_platform<T>(
        &self,
        party: &PartyRef,
        f: impl FnOnce(&ConnectedPlatform) -> T,
    ) -> Result<T, OcpiError> {
        let platforms = self.platforms.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let platform = platforms.values().find(|p| p.hosts(party)).ok_or_else(|| OcpiError::Remote {
            status_code: StatusCode::UNKNOWN_RECEIVER,
            status_message: Some(format!("the hub does not know {party}")),
        })?;
        if !platform.is_reachable() {
            return Err(OcpiError::Remote {
                status_code: StatusCode::CONNECTION_PROBLEM,
                status_message: Some(format!("{party} is {}", platform.status)),
            });
        }
        Ok(f(platform))
    }

    /// Whether the hub knows `party` at all, connected or not.
    #[must_use]
    pub fn knows(&self, party: &PartyRef) -> bool {
        self.platforms
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|p| p.hosts(party))
    }

    /// The platform id hosting `party`.
    #[must_use]
    pub fn platform_of(&self, party: &PartyRef) -> Option<String> {
        self.platforms
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .find(|p| p.hosts(party))
            .map(|p| p.platform_id.clone())
    }

    /// The parties a Broadcast Push from `sender` should reach for `module`.
    ///
    /// > *For simplicity, connected clients might push (POST, PUT, PATCH) information to all
    /// > connected clients with an "opposite role" … When using Broadcast Push, the Hub broadcasts
    /// > received information to all connected clients … using its own party-id and country-code
    /// > in the 'OCPI-from-' headers.*
    ///
    /// A party is a recipient when it is connected, fills a role that receives from the sender's
    /// role, implements the module's Receiver interface, and is not the sender itself.
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing_broadcast_push
    #[must_use]
    pub fn broadcast_targets(
        &self,
        sender: &PartyRef,
        sender_role: Role,
        module: &ModuleId,
    ) -> Vec<(String, PartyRef)> {
        let platforms = self.platforms.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut targets = Vec::new();
        for platform in platforms.values() {
            if !platform.is_reachable() || !platform.implements(module, InterfaceRole::Receiver) {
                continue;
            }
            for (party, role) in &platform.parties {
                if party == sender {
                    continue;
                }
                if role.receives_broadcast_from(sender_role) {
                    targets.push((platform.platform_id.clone(), party.clone()));
                }
            }
        }
        targets
    }

    /// Every party that implements a module's Sender interface, for a GET All.
    ///
    /// > *A client (Receiver) can request a GET on the Sender interface of a module implemented by
    /// > a Hub. The Hub can then combine objects from different connected parties.*
    ///
    /// Spec: 2.3.0 §transport_and_format_get_all_via_hubs
    #[must_use]
    pub fn get_all_sources(&self, requester: &PartyRef, module: &ModuleId) -> Vec<(String, PartyRef)> {
        let platforms = self.platforms.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut sources = Vec::new();
        for platform in platforms.values() {
            if !platform.is_reachable() || !platform.implements(module, InterfaceRole::Sender) {
                continue;
            }
            for (party, _) in &platform.parties {
                if party != requester {
                    sources.push((platform.platform_id.clone(), party.clone()));
                }
            }
        }
        sources
    }

    /// Every platform the hub knows, connected or not.
    #[must_use]
    pub fn platform_ids(&self) -> Vec<String> {
        self.platforms.read().unwrap_or_else(std::sync::PoisonError::into_inner).keys().cloned().collect()
    }

    /// Every party the hub knows, with its role and connection status.
    ///
    /// This is what the `hubclientinfo` module publishes.
    #[must_use]
    pub fn client_info(&self) -> Vec<(PartyRef, Role, ConnectionStatus)> {
        let platforms = self.platforms.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        platforms
            .values()
            .flat_map(|p| p.parties.iter().map(move |(party, role)| (party.clone(), *role, p.status.clone())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VersionNumber;
    use crate::transport::CredentialsToken;
    use crate::types::Url;

    fn platform(
        id: &str,
        parties: &[(&str, &str, Role)],
        status: ConnectionStatus,
        modules: &[(ModuleId, InterfaceRole)],
    ) -> ConnectedPlatform {
        let mut builder = Peer::builder(VersionNumber::V2_3_0, CredentialsToken::new("t").unwrap());
        for (module, role) in modules {
            builder = builder.endpoint(
                module.clone(),
                *role,
                Url::new(format!("https://{id}.example.com/{module}")).unwrap(),
            );
        }
        ConnectedPlatform {
            platform_id: id.to_owned(),
            peer: builder.build(),
            parties: parties
                .iter()
                .map(|(cc, pid, role)| (PartyRef::new(*cc, *pid).unwrap(), *role))
                .collect(),
            status,
        }
    }

    fn table() -> RoutingTable {
        let t = RoutingTable::new();
        t.upsert(platform(
            "cpo",
            &[("NL", "TNM", Role::Cpo)],
            ConnectionStatus::Connected,
            &[(ModuleId::Locations, InterfaceRole::Sender), (ModuleId::Tokens, InterfaceRole::Receiver)],
        ));
        t.upsert(platform(
            "msp",
            &[("DE", "ABC", Role::Emsp), ("DE", "XYZ", Role::Nsp)],
            ConnectionStatus::Connected,
            &[(ModuleId::Locations, InterfaceRole::Receiver), (ModuleId::Tokens, InterfaceRole::Sender)],
        ));
        t.upsert(platform(
            "gone",
            &[("FR", "OLD", Role::Emsp)],
            ConnectionStatus::Offline,
            &[(ModuleId::Locations, InterfaceRole::Receiver)],
        ));
        t
    }

    #[test]
    fn an_unknown_party_is_4001_and_a_disconnected_one_is_4003() {
        let t = table();
        assert!(t.with_platform(&PartyRef::new("NL", "TNM").unwrap(), |p| p.platform_id.clone()).is_ok());

        let unknown = t.with_platform(&PartyRef::new("XX", "NON").unwrap(), |_| ()).unwrap_err();
        assert_eq!(unknown.status_code(), StatusCode::UNKNOWN_RECEIVER);

        let offline = t.with_platform(&PartyRef::new("FR", "OLD").unwrap(), |_| ()).unwrap_err();
        assert_eq!(offline.status_code(), StatusCode::CONNECTION_PROBLEM);
        assert!(t.knows(&PartyRef::new("FR", "OLD").unwrap()), "known, just not reachable");
    }

    #[test]
    fn a_broadcast_from_a_cpo_reaches_the_emsp_like_roles_that_implement_the_module() {
        let t = table();
        let targets =
            t.broadcast_targets(&PartyRef::new("NL", "TNM").unwrap(), Role::Cpo, &ModuleId::Locations);
        let parties: Vec<String> = targets.iter().map(|(_, p)| p.to_string()).collect();
        assert!(parties.contains(&"DE/ABC".to_owned()), "{parties:?}");
        assert!(parties.contains(&"DE/XYZ".to_owned()), "an NSP receives Locations too");
        assert!(!parties.contains(&"FR/OLD".to_owned()), "an offline platform is skipped");
        assert!(!parties.contains(&"NL/TNM".to_owned()), "the sender is not a target");
    }

    #[test]
    fn a_broadcast_skips_platforms_that_do_not_implement_the_module() {
        let t = table();
        // The CPO platform implements Tokens/Receiver, so a broadcast of Tokens from the eMSP
        // reaches it; the other eMSP-like party does not receive from an eMSP.
        let targets =
            t.broadcast_targets(&PartyRef::new("DE", "ABC").unwrap(), Role::Emsp, &ModuleId::Tokens);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].1, PartyRef::new("NL", "TNM").unwrap());
    }

    #[test]
    fn get_all_collects_every_sender_but_the_requester() {
        let t = table();
        let sources = t.get_all_sources(&PartyRef::new("DE", "ABC").unwrap(), &ModuleId::Locations);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].1, PartyRef::new("NL", "TNM").unwrap());
    }

    #[test]
    fn status_changes_take_effect_immediately() {
        let t = table();
        assert!(t.set_status("cpo", ConnectionStatus::Offline));
        assert!(!t.set_status("nope", ConnectionStatus::Offline));
        let err = t.with_platform(&PartyRef::new("NL", "TNM").unwrap(), |_| ()).unwrap_err();
        assert_eq!(err.status_code(), StatusCode::CONNECTION_PROBLEM);
        assert_eq!(t.client_info().len(), 4);
    }
}
