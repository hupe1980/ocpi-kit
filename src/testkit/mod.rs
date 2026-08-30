//! Building blocks for testing an OCPI integration: sample objects and in-memory stores.
//!
//! Everything here is deliberately dependency-light — no HTTP mocking framework, no test runner —
//! so it can be used from a unit test, an integration test, a fuzz target or a demo binary
//! without dragging a stack along.
//!
//! ```
//! use ocpi_kit::testkit::{sample, InMemoryLocations};
//! use ocpi_kit::types::Validate;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let location = sample::location("LOC1")?;
//! location.validate()?;
//!
//! let store = InMemoryLocations::new();
//! store.put(location.clone());
//! assert_eq!(store.get("loc1").unwrap().id, location.id);
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "server")]
mod peer;
pub mod sample;
pub mod stores;

#[cfg(feature = "server")]
#[cfg_attr(docsrs, doc(cfg(feature = "server")))]
pub use peer::{MockPeer, MockPeerStores};
pub use stores::{InMemoryCdrs, InMemoryLocations, InMemorySessions, InMemoryTariffs, InMemoryTokens};

#[cfg(feature = "server")]
use crate::VersionNumber;
use crate::transport::CredentialsToken;
use crate::types::PartyRef;

/// A credentials token with a fixed, obviously-fake value.
///
/// Using a recognisable constant in tests makes an accidental leak into a log or a fixture
/// obvious, and makes failures reproducible.
#[must_use]
pub fn test_token(label: &str) -> CredentialsToken {
    CredentialsToken::new(format!("test-token-{label}"))
        .unwrap_or_else(|e| panic!("test token label {label:?} is not usable: {e}"))
}

/// The party a test CPO speaks as.
///
/// `NL/TNM` is the pair the specification's own examples use.
#[must_use]
pub fn test_cpo() -> PartyRef {
    PartyRef::new("NL", "TNM").expect("NL/TNM is a valid party reference")
}

/// The party a test eMSP speaks as.
#[must_use]
pub fn test_msp() -> PartyRef {
    PartyRef::new("DE", "ABC").expect("DE/ABC is a valid party reference")
}

/// The party a test hub speaks as.
#[must_use]
pub fn test_hub() -> PartyRef {
    PartyRef::new("NL", "HUB").expect("NL/HUB is a valid party reference")
}

/// An [`AuthenticatedPeer`](crate::server::AuthenticatedPeer) for a fully registered platform.
#[cfg(feature = "server")]
#[must_use]
pub fn registered_peer(peer_id: &str, parties: Vec<PartyRef>) -> crate::server::AuthenticatedPeer {
    crate::server::AuthenticatedPeer {
        peer_id: peer_id.to_owned(),
        role: crate::transport::TokenRole::C,
        parties,
        version: VersionNumber::V2_3_0,
    }
}

/// An [`AuthenticatedPeer`](crate::server::AuthenticatedPeer) that still holds
/// `CREDENTIALS_TOKEN_A`, for testing the scope rule.
///
/// Takes the parties for the same reason [`registered_peer`] does: the ownership check compares
/// the URL's owner against them, so a helper that guessed would make every ownership test a test
/// of the guess.
#[cfg(feature = "server")]
#[must_use]
pub fn bootstrap_peer(peer_id: &str, parties: Vec<PartyRef>) -> crate::server::AuthenticatedPeer {
    crate::server::AuthenticatedPeer {
        peer_id: peer_id.to_owned(),
        role: crate::transport::TokenRole::A,
        parties,
        version: VersionNumber::V2_3_0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_parties_are_the_ones_the_spec_examples_use() {
        assert_eq!(test_cpo().to_string(), "NL/TNM");
        assert_eq!(test_msp().to_string(), "DE/ABC");
        assert!(test_token("a").expose_secret().starts_with("test-token-"));
    }
}
