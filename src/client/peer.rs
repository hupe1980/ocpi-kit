//! What this process knows about one connected platform.

use std::collections::BTreeMap;

use crate::transport::{CredentialsToken, Quirks, ReceiverEndpoint, RoutingScenario, SenderEndpoint};
use crate::types::{PartyRef, Url};
use crate::v2_3_0::versions::VersionDetails;
use crate::{InterfaceRole, ModuleId, VersionNumber};

/// A connected platform: the version agreed with it, where its endpoints are, and the token to
/// authenticate with.
///
/// A `Peer` is built by the [registration handshake](super::Registration) or, for a connection
/// that was established before this process started, from stored state with
/// [`Peer::builder`].
///
/// ```
/// use ocpi_kit::client::Peer;
/// use ocpi_kit::transport::CredentialsToken;
/// use ocpi_kit::types::{PartyRef, Url};
/// use ocpi_kit::{InterfaceRole, ModuleId, VersionNumber};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let peer = Peer::builder(VersionNumber::V2_3_0, CredentialsToken::new("token-c")?)
///     .versions_url(Url::new("https://cpo.example.com/ocpi/versions")?)
///     .endpoint(ModuleId::Locations, InterfaceRole::Sender,
///               Url::new("https://cpo.example.com/ocpi/cpo/2.3.0/locations")?)
///     .party(PartyRef::new("NL", "TNM")?)
///     .build();
///
/// assert!(peer.implements(&ModuleId::Locations, InterfaceRole::Sender));
/// assert!(!peer.implements(&ModuleId::Cdrs, InterfaceRole::Sender));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Peer {
    version: VersionNumber,
    token: CredentialsToken,
    versions_url: Option<Url>,
    endpoints: BTreeMap<(ModuleId, InterfaceRole), Url>,
    parties: Vec<PartyRef>,
    hub: Option<PartyRef>,
    quirks: Quirks,
}

impl Peer {
    /// Starts building a peer that was registered in a previous run of this process.
    #[must_use]
    pub fn builder(version: VersionNumber, token: CredentialsToken) -> PeerBuilder {
        PeerBuilder {
            quirks: Quirks::for_version(&version),
            peer: Self {
                version,
                token,
                versions_url: None,
                endpoints: BTreeMap::new(),
                parties: Vec::new(),
                hub: None,
                quirks: Quirks::default(),
            },
        }
    }

    /// The OCPI version agreed with this peer.
    #[must_use]
    pub const fn version(&self) -> &VersionNumber {
        &self.version
    }

    /// The credentials token to authenticate requests to this peer with.
    #[must_use]
    pub const fn token(&self) -> &CredentialsToken {
        &self.token
    }

    /// The peer's `/versions` endpoint.
    #[must_use]
    pub const fn versions_url(&self) -> Option<&Url> {
        self.versions_url.as_ref()
    }

    /// The parties this peer speaks for, from the `roles` of its credentials.
    #[must_use]
    pub fn parties(&self) -> &[PartyRef] {
        &self.parties
    }

    /// The hub this peer routes through, if it declared one.
    #[must_use]
    pub const fn hub(&self) -> Option<&PartyRef> {
        self.hub.as_ref()
    }

    /// The interoperability profile for this peer.
    #[must_use]
    pub const fn quirks(&self) -> &Quirks {
        &self.quirks
    }

    /// Replaces the interoperability profile.
    pub fn set_quirks(&mut self, quirks: Quirks) {
        self.quirks = quirks;
    }

    /// Replaces the credentials token, as a credentials `PUT` does.
    pub fn set_token(&mut self, token: CredentialsToken) {
        self.token = token;
    }

    /// Whether the peer implements a module in a given role.
    #[must_use]
    pub fn implements(&self, module: &ModuleId, role: InterfaceRole) -> bool {
        self.endpoint_url(module, role).is_some()
    }

    /// The peer's URL for a module and role.
    ///
    /// Module identifiers are matched case-insensitively when
    /// [`Quirks::case_insensitive_module_ids`] is on, which matters for the `Booking` module.
    #[must_use]
    pub fn endpoint_url(&self, module: &ModuleId, role: InterfaceRole) -> Option<&Url> {
        if let Some(url) = self.endpoints.get(&(module.clone(), role)) {
            return Some(url);
        }
        if self.quirks.case_insensitive_module_ids {
            return self
                .endpoints
                .iter()
                .find(|((m, r), _)| *r == role && m.matches(module))
                .map(|(_, url)| url);
        }
        None
    }

    /// The Sender-interface endpoint of a module.
    #[must_use]
    pub fn sender(&self, module: &ModuleId) -> Option<SenderEndpoint> {
        self.endpoint_url(module, InterfaceRole::Sender).cloned().map(SenderEndpoint::new)
    }

    /// The Receiver-interface endpoint of a module.
    #[must_use]
    pub fn receiver(&self, module: &ModuleId) -> Option<ReceiverEndpoint> {
        self.endpoint_url(module, InterfaceRole::Receiver).cloned().map(ReceiverEndpoint::new)
    }

    /// The credentials endpoint, ignoring the advertised role as the specification instructs.
    #[must_use]
    pub fn credentials_url(&self) -> Option<&Url> {
        self.endpoints.iter().find(|((m, _), _)| m.matches(&ModuleId::Credentials)).map(|(_, url)| url)
    }

    /// Every endpoint the peer advertised.
    pub fn endpoints(&self) -> impl Iterator<Item = (&ModuleId, InterfaceRole, &Url)> {
        self.endpoints.iter().map(|((m, r), url)| (m, *r, url))
    }

    /// Replaces the endpoint map from freshly fetched version details.
    ///
    /// A credentials `PUT` requires this: *"The server must fetch the client's endpoints again,
    /// even if the version has not changed."*
    pub fn update_endpoints(&mut self, details: &VersionDetails) {
        self.version = details.version.clone();
        self.endpoints =
            details.endpoints.iter().map(|e| ((e.identifier.clone(), e.role), e.url.clone())).collect();
    }

    /// The routing scenario for a direct request to `party`, or an open request when the
    /// destination is unknown.
    #[must_use]
    pub fn routing_for(&self, party: Option<&PartyRef>) -> RoutingScenario {
        match (party, self.hub.as_ref()) {
            (None, Some(_)) => RoutingScenario::OpenRoutingRequest,
            _ => RoutingScenario::Direct,
        }
    }

    /// The party to address a functional request to, when the caller did not name one.
    ///
    /// A peer that speaks for exactly one party needs no explicit `to`.
    #[must_use]
    pub fn default_party(&self) -> Option<&PartyRef> {
        match self.parties.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }
}

/// Builds a [`Peer`] from stored registration state.
#[derive(Debug)]
pub struct PeerBuilder {
    peer: Peer,
    quirks: Quirks,
}

impl PeerBuilder {
    /// Sets the peer's `/versions` endpoint.
    #[must_use]
    pub fn versions_url(mut self, url: Url) -> Self {
        self.peer.versions_url = Some(url);
        self
    }

    /// Adds one endpoint.
    #[must_use]
    pub fn endpoint(mut self, module: ModuleId, role: InterfaceRole, url: Url) -> Self {
        self.peer.endpoints.insert((module, role), url);
        self
    }

    /// Adds every endpoint from a version details document.
    #[must_use]
    pub fn endpoints_from(mut self, details: &VersionDetails) -> Self {
        self.peer.update_endpoints(details);
        self
    }

    /// Adds a party this peer speaks for.
    #[must_use]
    pub fn party(mut self, party: PartyRef) -> Self {
        self.peer.parties.push(party);
        self
    }

    /// Names the hub this peer routes through.
    #[must_use]
    pub fn hub(mut self, hub: PartyRef) -> Self {
        self.peer.hub = Some(hub);
        self
    }

    /// Overrides the interoperability profile, which otherwise follows the version.
    #[must_use]
    pub fn quirks(mut self, quirks: Quirks) -> Self {
        self.quirks = quirks;
        self
    }

    /// Finishes the peer.
    #[must_use]
    pub fn build(mut self) -> Peer {
        self.peer.quirks = self.quirks;
        self.peer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(path: &str) -> Url {
        Url::new(format!("https://cpo.example.com/ocpi/{path}")).unwrap()
    }

    fn peer() -> Peer {
        Peer::builder(VersionNumber::V2_3_0, CredentialsToken::new("token-c").unwrap())
            .versions_url(url("versions"))
            .endpoint(ModuleId::Credentials, InterfaceRole::Receiver, url("cpo/2.3.0/credentials"))
            .endpoint(ModuleId::Locations, InterfaceRole::Sender, url("cpo/2.3.0/locations"))
            .party(PartyRef::new("NL", "TNM").unwrap())
            .build()
    }

    #[test]
    fn endpoints_are_looked_up_by_module_and_role() {
        let p = peer();
        assert!(p.implements(&ModuleId::Locations, InterfaceRole::Sender));
        assert!(!p.implements(&ModuleId::Locations, InterfaceRole::Receiver));
        assert_eq!(p.sender(&ModuleId::Locations).unwrap().base(), &url("cpo/2.3.0/locations"));
        assert!(p.receiver(&ModuleId::Locations).is_none());
    }

    #[test]
    fn the_credentials_endpoint_ignores_the_advertised_role() {
        // The spec: "disregard the value of the role property of the Endpoint object for other
        // platforms' credentials modules".
        assert_eq!(peer().credentials_url(), Some(&url("cpo/2.3.0/credentials")));
    }

    #[test]
    fn module_ids_match_case_insensitively_by_default() {
        let p = Peer::builder(VersionNumber::V2_3_0, CredentialsToken::new("t").unwrap())
            .endpoint(ModuleId::Custom("bookings".into()), InterfaceRole::Sender, url("bookings"))
            .build();
        // The spec writes the identifier `Booking`; this peer wrote `bookings`.
        assert!(p.implements(&ModuleId::Booking, InterfaceRole::Sender));

        let strict = Peer::builder(VersionNumber::V2_3_0, CredentialsToken::new("t").unwrap())
            .endpoint(ModuleId::Custom("bookings".into()), InterfaceRole::Sender, url("bookings"))
            .quirks(Quirks::strict())
            .build();
        assert!(!strict.implements(&ModuleId::Booking, InterfaceRole::Sender));
    }

    #[test]
    fn quirks_follow_the_version_unless_overridden() {
        let legacy = Peer::builder(VersionNumber::V2_1_1, CredentialsToken::new("t").unwrap()).build();
        assert!(legacy.quirks().send_unencoded_token, "2.1.1 peers do not Base64 the token");
        assert!(legacy.quirks().omit_routing_headers);
        assert!(!peer().quirks().send_unencoded_token);
    }

    #[test]
    fn a_single_party_peer_needs_no_explicit_destination() {
        assert_eq!(peer().default_party(), Some(&PartyRef::new("NL", "TNM").unwrap()));
        let platform = Peer::builder(VersionNumber::V2_3_0, CredentialsToken::new("t").unwrap())
            .party(PartyRef::new("NL", "AAA").unwrap())
            .party(PartyRef::new("NL", "BBB").unwrap())
            .build();
        assert_eq!(platform.default_party(), None, "a multi-party platform must be addressed");
    }
}
