//! Who is calling: resolving a credentials token to a party, in constant time.
//!
//! # Lock poisoning
//!
//! The maps here are plain data with no invariant that a panic could leave half-applied, so every
//! lock is taken with [`PoisonError::into_inner`](std::sync::PoisonError::into_inner) rather than
//! `unwrap`. A single panicking request must not turn a server into one that answers every
//! subsequent request with a panic — which is what a poisoned `RwLock` does when nobody handles
//! it.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use crate::ModuleId;
use crate::transport::{CredentialsToken, OcpiError, TokenRole};
use crate::types::PartyRef;
use crate::{InterfaceRole, VersionNumber};

/// The party a request was authenticated as.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthenticatedPeer {
    /// A stable identifier for the platform, for logging and for looking up its state.
    pub peer_id: String,
    /// Which handshake token this is, which decides what it may address.
    pub role: TokenRole,
    /// The parties this platform speaks for, from the `roles` of its credentials.
    pub parties: Vec<PartyRef>,
    /// The OCPI version the connection was registered with.
    pub version: VersionNumber,
}

impl AuthenticatedPeer {
    /// Whether this platform speaks for `party`.
    ///
    /// This is what decides whether a client-owned-object URL is theirs to write to.
    #[must_use]
    pub fn owns(&self, party: &PartyRef) -> bool {
        self.parties.iter().any(|p| p == party)
    }

    /// Checks that this token may address `module`.
    ///
    /// > *When a server receives a request with a valid `CREDENTIALS_TOKEN_A`, on another module
    /// > than `credentials` or `versions`, the server SHALL respond with an HTTP `401 -
    /// > Unauthorized` status code.*
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::TokenAOutOfScope`], which maps to HTTP 401.
    ///
    /// Spec: 2.3.0 §transport_and_format_authorization_header
    pub fn check_scope(&self, module: &ModuleId) -> Result<(), OcpiError> {
        if self.role.may_access(module) { Ok(()) } else { Err(OcpiError::TokenAOutOfScope) }
    }

    /// Checks that this platform may write to a client-owned object under `owner`.
    ///
    /// > *When a client tries to access an object with a URL that has a different `country_code`
    /// > and/or `party_id` than one of the CredentialsRoles given during the credentials
    /// > handshake, it is allowed to respond with an HTTP `404` status code, this way blocking
    /// > client access to objects that do not belong to them.*
    ///
    /// A 404 rather than a 403 is deliberate: it does not reveal whether the object exists.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::NotFound`], which maps to HTTP 404.
    ///
    /// Spec: 2.3.0 §transport_and_format_errors
    pub fn check_ownership(&self, owner: &PartyRef) -> Result<(), OcpiError> {
        if self.owns(owner) {
            return Ok(());
        }
        Err(OcpiError::NotFound(format!("{owner} is not a party of the authenticated platform")))
    }
}

/// Resolves an incoming credentials token to the platform that holds it.
///
/// The implementation decides where registrations live — a database, a config file, an in-memory
/// map for a test. What it must do is compare tokens in **constant time**, which
/// [`CredentialsToken`]'s [`PartialEq`] does; a naive `String` comparison leaks the token one
/// byte at a time to anyone who can measure the response.
pub trait TokenStore: Send + Sync + 'static {
    /// Looks up the platform a token belongs to.
    ///
    /// Returns `None` for a token this server does not know, which the caller turns into an
    /// HTTP 401 — *"If the header is missing or the credentials token doesn't match any known
    /// party then the server SHALL respond with an HTTP `401 - Unauthorized` status code."*
    fn resolve(&self, token: &CredentialsToken) -> Option<AuthenticatedPeer>;
}

impl<T: TokenStore> TokenStore for Arc<T> {
    fn resolve(&self, token: &CredentialsToken) -> Option<AuthenticatedPeer> {
        T::resolve(self, token)
    }
}

/// A [`TokenStore`] held in memory, for tests, small deployments and getting started.
///
/// Lookup is a linear scan with a constant-time comparison per entry, which is the right
/// trade-off up to a few thousand peers; beyond that, index by a keyed hash of the token in your
/// own store rather than by the token itself.
#[derive(Debug, Default)]
pub struct InMemoryTokenStore {
    entries: RwLock<Vec<(CredentialsToken, AuthenticatedPeer)>>,
}

impl InMemoryTokenStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a token.
    pub fn insert(&self, token: CredentialsToken, peer: AuthenticatedPeer) {
        let mut entries = self.entries.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|(existing, _)| existing != &token);
        entries.push((token, peer));
    }

    /// Removes a token, as a credentials `DELETE` does.
    pub fn remove(&self, token: &CredentialsToken) {
        let mut entries = self.entries.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|(existing, _)| existing != token);
    }

    /// Replaces one platform's token, as a credentials `PUT` does.
    ///
    /// > *It is advisable to renew the credentials tokens at least once a month.*
    pub fn rotate(&self, peer_id: &str, new_token: CredentialsToken) -> bool {
        let mut entries = self.entries.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(index) = entries.iter().position(|(_, p)| p.peer_id == peer_id) else {
            return false;
        };
        let peer = entries[index].1.clone();
        entries.remove(index);
        entries.push((new_token, peer));
        true
    }

    /// How many tokens are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.read().unwrap_or_else(std::sync::PoisonError::into_inner).len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl TokenStore for InMemoryTokenStore {
    fn resolve(&self, token: &CredentialsToken) -> Option<AuthenticatedPeer> {
        let entries = self.entries.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        // `CredentialsToken: PartialEq` is a constant-time comparison.
        entries.iter().find(|(known, _)| known == token).map(|(_, peer)| peer.clone())
    }
}

/// Which modules and interfaces this server implements, for generating version details.
#[derive(Clone, Debug, Default)]
pub struct MountedModules {
    modules: Vec<(ModuleId, InterfaceRole)>,
}

impl MountedModules {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that a module and interface is served.
    pub fn add(&mut self, module: ModuleId, role: InterfaceRole) {
        if !self.modules.iter().any(|(m, r)| m == &module && *r == role) {
            self.modules.push((module, role));
        }
    }

    /// Everything mounted, in the order it was mounted.
    #[must_use]
    pub fn all(&self) -> &[(ModuleId, InterfaceRole)] {
        &self.modules
    }

    /// Whether a module and interface is served.
    #[must_use]
    pub fn contains(&self, module: &ModuleId, role: InterfaceRole) -> bool {
        self.modules.iter().any(|(m, r)| m.matches(module) && *r == role)
    }
}

/// A registry of the peers this server has registered, keyed by peer id.
///
/// Only the parts the server needs at request time; the full registration state belongs in the
/// integrator's own storage.
#[derive(Debug, Default)]
pub struct PeerRegistry {
    peers: RwLock<HashMap<String, AuthenticatedPeer>>,
}

impl PeerRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records or replaces a peer.
    pub fn upsert(&self, peer: AuthenticatedPeer) {
        self.peers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(peer.peer_id.clone(), peer);
    }

    /// Looks a peer up by id.
    #[must_use]
    pub fn get(&self, peer_id: &str) -> Option<AuthenticatedPeer> {
        self.peers.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(peer_id).cloned()
    }

    /// Forgets a peer, as a credentials `DELETE` does.
    pub fn remove(&self, peer_id: &str) -> Option<AuthenticatedPeer> {
        self.peers.write().unwrap_or_else(std::sync::PoisonError::into_inner).remove(peer_id)
    }

    /// Every registered peer.
    #[must_use]
    pub fn all(&self) -> Vec<AuthenticatedPeer> {
        self.peers.read().unwrap_or_else(std::sync::PoisonError::into_inner).values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_while_holding_the_lock_does_not_break_the_store() {
        // One request panicking must not turn a server into one that answers every subsequent
        // request with a panic, which is what an `unwrap` on a poisoned lock does.
        let store = std::sync::Arc::new(InMemoryTokenStore::new());
        let token = CredentialsToken::new("token-c").unwrap();
        store.insert(token.clone(), peer("platform-1", TokenRole::C));

        let poisoner = std::sync::Arc::clone(&store);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.entries.write().unwrap();
            panic!("a handler panicked while holding the lock");
        })
        .join();

        assert!(store.resolve(&token).is_some(), "the store still answers");
        store.insert(CredentialsToken::new("token-c2").unwrap(), peer("platform-2", TokenRole::C));
        assert_eq!(store.len(), 2);
    }

    fn peer(id: &str, role: TokenRole) -> AuthenticatedPeer {
        AuthenticatedPeer {
            peer_id: id.to_owned(),
            role,
            parties: vec![PartyRef::new("NL", "TNM").unwrap()],
            version: VersionNumber::V2_3_0,
        }
    }

    #[test]
    fn token_a_may_only_reach_credentials_and_versions() {
        let bootstrap = peer("p1", TokenRole::A);
        assert!(bootstrap.check_scope(&ModuleId::Credentials).is_ok());
        assert!(bootstrap.check_scope(&ModuleId::Versions).is_ok());
        let err = bootstrap.check_scope(&ModuleId::Locations).unwrap_err();
        assert_eq!(err.http_status(), 401);

        assert!(peer("p1", TokenRole::C).check_scope(&ModuleId::Locations).is_ok());
    }

    #[test]
    fn writing_to_another_partys_object_is_a_404_not_a_403() {
        let p = peer("p1", TokenRole::C);
        assert!(p.check_ownership(&PartyRef::new("nl", "tnm").unwrap()).is_ok());
        let err = p.check_ownership(&PartyRef::new("DE", "ABC").unwrap()).unwrap_err();
        assert_eq!(err.http_status(), 404, "a 404 does not reveal whether the object exists");
    }

    #[test]
    fn the_in_memory_store_resolves_rotates_and_forgets() {
        let store = InMemoryTokenStore::new();
        let token = CredentialsToken::new("token-c").unwrap();
        store.insert(token.clone(), peer("p1", TokenRole::C));
        assert_eq!(store.resolve(&token).unwrap().peer_id, "p1");
        assert!(store.resolve(&CredentialsToken::new("other").unwrap()).is_none());

        let rotated = CredentialsToken::new("token-c2").unwrap();
        assert!(store.rotate("p1", rotated.clone()));
        assert!(store.resolve(&token).is_none(), "the old token stops working");
        assert_eq!(store.resolve(&rotated).unwrap().peer_id, "p1");
        assert_eq!(store.len(), 1);

        store.remove(&rotated);
        assert!(store.is_empty());
    }

    #[test]
    fn mounted_modules_match_the_booking_identifier_either_way() {
        let mut mounted = MountedModules::new();
        mounted.add(ModuleId::Booking, InterfaceRole::Sender);
        mounted.add(ModuleId::Booking, InterfaceRole::Sender);
        assert_eq!(mounted.all().len(), 1, "mounting twice is idempotent");
        assert!(mounted.contains(&ModuleId::Custom("bookings".into()), InterfaceRole::Sender));
        assert!(!mounted.contains(&ModuleId::Booking, InterfaceRole::Receiver));
    }
}
