//! The credentials handshake, as a typestate.
//!
//! The registration flow is short and almost every integration gets some of it wrong. The
//! specification describes it as:
//!
//! > *The Receiver Platform must create a unique credentials token: `CREDENTIALS_TOKEN_A` that has
//! > to be used to authorize the Sender until the credentials exchange is finished. … The Sender
//! > starts the registration process, retrieves the version information and details (using
//! > `CREDENTIALS_TOKEN_A`). The Sender generates a unique credentials token
//! > `CREDENTIALS_TOKEN_B`, sends it to the Receiver in a POST request … The Receiver generates a
//! > unique credentials token `CREDENTIALS_TOKEN_C` and returns it … After the credentials
//! > exchange has finished, the Sender SHALL use `CREDENTIALS_TOKEN_C` in future OCPI requests.
//! > The `CREDENTIALS_TOKEN_A` can then be thrown away, it MAY no longer be used.*
//!
//! Each of those sentences is a state transition, so each is a type here:
//!
//! ```text
//! Registration ──discover()──▶ Discovered ──select()──▶ Selected ──register()──▶ Peer
//!  (has TOKEN_A)               (has versions)          (has endpoints)      (has TOKEN_C)
//! ```
//!
//! What that buys: `CREDENTIALS_TOKEN_A` is consumed by `register()` and never handed back, so it
//! cannot be used afterwards. `register()` exists only on `Selected`, so the POST cannot be sent
//! before the endpoints have been checked — which the specification requires:
//!
//! > *In case the Sender … cannot find the endpoints it expects, it is expected NOT to send the
//! > POST request with credentials to the Receiver.*
//!
//! Spec: 2.3.0 §credentials_registration

use http::Method;

use crate::convert::wire::ObjectKind;
use crate::transport::{CredentialsToken, OcpiError, OcpiRequest, Quirks};
use crate::types::{PartyRef, Url};
use crate::v2_3_0::credentials::Credentials;
use crate::v2_3_0::versions::{Version, VersionDetails};
use crate::{InterfaceRole, ModuleId, VersionNumber};

use super::http::Transport;
use super::peer::Peer;

/// Step 0: what was agreed out of band.
///
/// > *This credentials token along with the versions endpoint SHOULD be sent to the Sender in a
/// > secure way that is outside the scope of this protocol.*
#[derive(Debug)]
pub struct Registration {
    versions_url: Url,
    token_a: CredentialsToken,
    /// Set only when the caller overrode the profile; otherwise it follows the version once that
    /// is known.
    quirks_override: Option<Quirks>,
}

impl Registration {
    /// Starts a registration with the versions URL and `CREDENTIALS_TOKEN_A`.
    #[must_use]
    pub fn new(versions_url: Url, token_a: CredentialsToken) -> Self {
        Self { versions_url, token_a, quirks_override: None }
    }

    /// Uses a specific interoperability profile while talking to this peer.
    ///
    /// Useful when the peer is known to be a 2.1.1 implementation before its version is
    /// discovered, since those do not Base64-encode the token.
    #[must_use]
    pub fn with_quirks(mut self, quirks: Quirks) -> Self {
        self.quirks_override = Some(quirks);
        self
    }

    /// `GET {versions_url}` with `CREDENTIALS_TOKEN_A`.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError`] when the peer cannot be reached or answers with an error.
    pub async fn discover(self, transport: &Transport) -> Result<Discovered, OcpiError> {
        // The version is not known yet, so the bootstrap GET uses the caller's profile if they
        // gave one and the conformant defaults otherwise.
        let quirks = self.quirks_override.clone().unwrap_or_default();
        let request = OcpiRequest::new(Method::GET, self.versions_url.clone(), ModuleId::Versions);
        let versions: Vec<Version> = transport.send(&request, &self.token_a, &quirks).await?;
        Ok(Discovered {
            versions_url: self.versions_url,
            token_a: self.token_a,
            quirks_override: self.quirks_override,
            versions,
        })
    }
}

/// Step 1: the peer's supported versions are known.
#[derive(Debug)]
pub struct Discovered {
    versions_url: Url,
    token_a: CredentialsToken,
    quirks_override: Option<Quirks>,
    versions: Vec<Version>,
}

impl Discovered {
    /// Every version the peer advertised, including ones this crate cannot speak.
    #[must_use]
    pub fn versions(&self) -> &[Version] {
        &self.versions
    }

    /// The newest version both sides can speak **and** the typed client can use.
    ///
    /// Two questions hide behind "common version", and answering the wrong one produces a peer
    /// that registers and then fails every call:
    ///
    /// * [`VersionNumber::is_supported`] asks whether this build has a *wire model* for a
    ///   version. OCPI 2.1.1 passes.
    /// * [`bridgeable`](crate::convert::wire::bridgeable) asks whether a document can be carried
    ///   between that version and the canonical model the typed module clients speak. OCPI 2.1.1
    ///   does **not** — it has no owner fields, no routing headers and no `Price`, so the
    ///   crossing is a deployment decision rather than a translation.
    ///
    /// So a bridgeable version always wins, even over a newer one that is merely modelled. When
    /// nothing bridges, the newest modelled version is still returned — the raw
    /// [`ModuleClient`](super::ModuleClient) calls work against it — and
    /// [`select_best`](Self::select_best) logs what that costs.
    #[must_use]
    pub fn best_common_version(&self) -> Option<&Version> {
        let usable = |v: &&Version| {
            v.version.is_supported()
                && crate::convert::wire::bridgeable(&v.version, &crate::CANONICAL_VERSION)
        };
        self.versions.iter().filter(usable).max_by(|a, b| a.version.cmp_by_release(&b.version)).or_else(
            || {
                self.versions
                    .iter()
                    .filter(|v| v.version.is_supported())
                    .max_by(|a, b| a.version.cmp_by_release(&b.version))
            },
        )
    }

    /// `GET` the details of the newest version both sides support.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::Remote`] with `3002 Unsupported version` when there is no version in
    /// common — which is exactly the code the specification reserves for it.
    pub async fn select_best(self, transport: &Transport) -> Result<Selected, OcpiError> {
        let chosen = self.best_common_version().map(|v| v.version.clone());
        if let Some(version) = chosen.as_ref()
            && !crate::convert::wire::bridgeable(version, &crate::CANONICAL_VERSION)
        {
            tracing::warn!(
                ocpi.peer_version = %version,
                "the only version in common is one this build cannot translate to OCPI {}; the \
                 typed module clients will refuse every call against this peer, and only the raw \
                 ModuleClient methods are usable",
                crate::CANONICAL_VERSION,
            );
        }
        let chosen = chosen.ok_or_else(|| OcpiError::Remote {
            status_code: crate::transport::StatusCode::UNSUPPORTED_VERSION,
            status_message: Some(format!(
                "peer supports {}, this build supports {}",
                self.versions.iter().map(|v| v.version.to_string()).collect::<Vec<_>>().join(", "),
                VersionNumber::supported().iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
            )),
        })?;
        self.select(transport, &chosen).await
    }

    /// `GET` the details of a specific version.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::Remote`] with `3002` when the peer does not offer that version.
    pub async fn select(self, transport: &Transport, version: &VersionNumber) -> Result<Selected, OcpiError> {
        let entry =
            self.versions.iter().find(|v| v.version == *version).ok_or_else(|| OcpiError::Remote {
                status_code: crate::transport::StatusCode::UNSUPPORTED_VERSION,
                status_message: Some(format!("peer does not offer OCPI {version}")),
            })?;
        // Now that the version is known, the profile follows it unless the caller overrode it.
        let quirks = self.quirks_override.clone().unwrap_or_else(|| Quirks::for_version(version));
        let request = OcpiRequest::new(Method::GET, entry.url.clone(), ModuleId::Versions);
        let details: VersionDetails = transport.send(&request, &self.token_a, &quirks).await?;
        Ok(Selected {
            versions_url: self.versions_url,
            token_a: self.token_a,
            quirks,
            version: version.clone(),
            details,
        })
    }
}

/// Step 2: the peer's endpoints for the chosen version are known, and can be checked before
/// anything is sent.
#[derive(Debug)]
pub struct Selected {
    versions_url: Url,
    token_a: CredentialsToken,
    quirks: Quirks,
    version: VersionNumber,
    details: VersionDetails,
}

impl Selected {
    /// The version that was selected.
    #[must_use]
    pub const fn version(&self) -> &VersionNumber {
        &self.version
    }

    /// The endpoints the peer advertised for it.
    #[must_use]
    pub const fn details(&self) -> &VersionDetails {
        &self.details
    }

    /// Checks that the peer implements everything this party needs.
    ///
    /// > *In case the Sender (starting the credentials exchange process) cannot find the endpoints
    /// > it expects, it is expected NOT to send the POST request with credentials to the Receiver.
    /// > Log a message/notify the administrator.*
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::Remote`] with `3003 No matching endpoints` naming what is missing.
    /// Call this before [`Selected::register`]; nothing has been sent to the peer yet, so
    /// stopping here is exactly what the specification asks for.
    pub fn require(&self, required: &[(ModuleId, InterfaceRole)]) -> Result<(), OcpiError> {
        let missing = self.details.missing(required);
        if missing.is_empty() {
            return Ok(());
        }
        Err(OcpiError::Remote {
            status_code: crate::transport::StatusCode::NO_MATCHING_ENDPOINTS,
            status_message: Some(format!(
                "peer does not implement {}",
                missing.iter().map(|(m, r)| format!("{m}/{r}")).collect::<Vec<_>>().join(", ")
            )),
        })
    }

    /// `POST {credentials}` with `CREDENTIALS_TOKEN_B`, receiving `CREDENTIALS_TOKEN_C`.
    ///
    /// Consumes `self`, so `CREDENTIALS_TOKEN_A` is gone afterwards: *"it MAY no longer be
    /// used."*
    ///
    /// `credentials` must carry **`CREDENTIALS_TOKEN_B`** — the token the peer will use to call
    /// *this* party — and the versions URL of *this* party. The token in the response is
    /// `CREDENTIALS_TOKEN_C`, which is what the returned [`Peer`] authenticates with.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::MethodNotAllowed`] when the peer says this party is already
    /// registered — *"This method MUST return a HTTP status code 405: method not allowed if the
    /// client has already been registered before"* — and [`OcpiError::Remote`] with `3001` when
    /// the peer could not call back.
    pub async fn register(self, transport: &Transport, credentials: &Credentials) -> Result<Peer, OcpiError> {
        super::http::check_outgoing(credentials, transport.config())?;
        let url = self.details.credentials_url().cloned().ok_or_else(|| OcpiError::Remote {
            status_code: crate::transport::StatusCode::NO_MATCHING_ENDPOINTS,
            status_message: Some(
                "peer advertised no credentials endpoint, which every implementation must have".to_owned(),
            ),
        })?;
        let request = self.credentials_request(Method::POST, url, credentials)?;
        let theirs = self.their_credentials(transport, &request, &self.token_a).await?;
        Ok(peer_from(self.version, self.quirks, self.versions_url, &self.details, &theirs))
    }

    /// `PUT {credentials}`, for a peer this party is already registered with.
    ///
    /// > *A `PUT` will switch to the version that contains this credentials endpoint if it's
    /// > different from the current version. The server must fetch the client's endpoints again,
    /// > even if the version has not changed.*
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::MethodNotAllowed`] when the peer says this party is **not** registered
    /// — the mirror image of [`Selected::register`].
    pub async fn update(
        self,
        transport: &Transport,
        current_token: &CredentialsToken,
        credentials: &Credentials,
    ) -> Result<Peer, OcpiError> {
        super::http::check_outgoing(credentials, transport.config())?;
        let url = self.details.credentials_url().cloned().ok_or_else(|| OcpiError::Remote {
            status_code: crate::transport::StatusCode::NO_MATCHING_ENDPOINTS,
            status_message: Some("peer advertised no credentials endpoint".to_owned()),
        })?;
        let request = self.credentials_request(Method::PUT, url, credentials)?;
        let theirs = self.their_credentials(transport, &request, current_token).await?;
        Ok(peer_from(self.version, self.quirks, self.versions_url, &self.details, &theirs))
    }

    /// A credentials request whose body is written in the version that was negotiated.
    ///
    /// The handshake is the one exchange that happens *before* there is a [`Peer`] to ask, so the
    /// version comes from [`Discovered::select`] instead. It matters: a 2.2.1 `Credentials` has no
    /// `hub_party_id`, and its `Role` enum still has `HUB`, which the 2.3.0 one does not — so
    /// registering with a 2.2.1 hub without translating fails to decode the answer.
    fn credentials_request(
        &self,
        method: Method,
        url: Url,
        credentials: &Credentials,
    ) -> Result<OcpiRequest, OcpiError> {
        let request = OcpiRequest::new(method, url, ModuleId::Credentials);
        match self.bridge_out(credentials)? {
            Some(value) => request.with_body(&value),
            None => request.with_body(credentials),
        }
    }

    fn bridge_out(&self, credentials: &Credentials) -> Result<Option<serde_json::Value>, OcpiError> {
        if self.version == crate::CANONICAL_VERSION {
            return Ok(None);
        }
        let value = serde_json::to_value(credentials)
            .map_err(|e| OcpiError::Decode { path: "/".to_owned(), message: e.to_string() })?;
        let converted = ObjectKind::Credentials
            .bridge(&crate::CANONICAL_VERSION, &self.version, value)
            .map_err(|e| OcpiError::Unsupported(e.to_string()))?;
        if let Some(note) = converted.lossy.to_status_message() {
            tracing::warn!(ocpi.peer_version = %self.version, "{note}");
        }
        Ok(Some(converted.value))
    }

    async fn their_credentials(
        &self,
        transport: &Transport,
        request: &OcpiRequest,
        token: &CredentialsToken,
    ) -> Result<Credentials, OcpiError> {
        if self.version == crate::CANONICAL_VERSION {
            return transport.send(request, token, &self.quirks).await;
        }
        let value: serde_json::Value = transport.send(request, token, &self.quirks).await?;
        let converted = ObjectKind::Credentials
            .bridge(&self.version, &crate::CANONICAL_VERSION, value)
            .map_err(|e| OcpiError::Unsupported(e.to_string()))?;
        serde_json::from_value(converted.value)
            .map_err(|e| OcpiError::Decode { path: "/".to_owned(), message: e.to_string() })
    }
}

fn peer_from(
    version: VersionNumber,
    quirks: Quirks,
    versions_url: Url,
    details: &VersionDetails,
    theirs: &Credentials,
) -> Peer {
    let mut builder = Peer::builder(version, CredentialsToken::new_lenient(theirs.token.as_str()))
        .versions_url(versions_url)
        .endpoints_from(details)
        .quirks(quirks);
    for party in theirs.parties() {
        builder = builder.party(party);
    }
    if let Some(hub) = theirs.hub_party() {
        builder = builder.hub(hub);
    }
    builder.build()
}

/// Where a connection with one peer stands.
///
/// The transitions between these are the ones the specification defines; storing this enum is how
/// a process remembers a registration across restarts.
///
/// Spec: 2.3.0 §credentials_use_cases
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PeerState {
    /// `CREDENTIALS_TOKEN_A` has been exchanged out of band; nothing else has happened.
    ///
    /// The token may only be used on the `credentials` and `versions` modules.
    Bootstrapped {
        /// The peer's versions endpoint.
        versions_url: Url,
        /// `CREDENTIALS_TOKEN_A`.
        token_a: CredentialsToken,
    },
    /// The handshake completed.
    Registered {
        /// The version in use.
        version: VersionNumber,
        /// The token this party uses to call the peer (`CREDENTIALS_TOKEN_C` for the Sender).
        our_token_for_them: CredentialsToken,
        /// The token the peer uses to call this party (`CREDENTIALS_TOKEN_B` for the Sender).
        their_token_for_us: CredentialsToken,
        /// The parties the peer speaks for.
        parties: Vec<PartyRef>,
    },
    /// A `DELETE` on the credentials module ended the connection.
    ///
    /// > *Both parties must end any automated communication.*
    Unregistered,
}

impl PeerState {
    /// Whether requests to functional modules are allowed in this state.
    ///
    /// Only a registered peer may be called for anything other than `credentials` and `versions`.
    #[must_use]
    pub const fn may_use_functional_modules(&self) -> bool {
        matches!(self, Self::Registered { .. })
    }

    /// Whether a credentials `POST` is the right method in this state.
    ///
    /// > *This method MUST return a HTTP status code 405: method not allowed if the client has
    /// > already been registered before.*
    #[must_use]
    pub const fn accepts_credentials_post(&self) -> bool {
        matches!(self, Self::Bootstrapped { .. } | Self::Unregistered)
    }

    /// Whether a credentials `PUT` or `DELETE` is the right method in this state.
    ///
    /// > *This method MUST return a HTTP status code 405: method not allowed if the client has not
    /// > been registered yet.*
    #[must_use]
    pub const fn accepts_credentials_put_or_delete(&self) -> bool {
        matches!(self, Self::Registered { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(v: &str) -> CredentialsToken {
        CredentialsToken::new(v).unwrap()
    }

    #[test]
    fn credentials_methods_are_gated_on_the_registration_state() {
        let bootstrapped = PeerState::Bootstrapped {
            versions_url: Url::new("https://e.com/versions").unwrap(),
            token_a: token("A"),
        };
        assert!(bootstrapped.accepts_credentials_post());
        assert!(!bootstrapped.accepts_credentials_put_or_delete(), "405 until registered");
        assert!(!bootstrapped.may_use_functional_modules());

        let registered = PeerState::Registered {
            version: VersionNumber::V2_3_0,
            our_token_for_them: token("C"),
            their_token_for_us: token("B"),
            parties: vec![PartyRef::new("NL", "TNM").unwrap()],
        };
        assert!(!registered.accepts_credentials_post(), "405 once registered");
        assert!(registered.accepts_credentials_put_or_delete());
        assert!(registered.may_use_functional_modules());

        assert!(PeerState::Unregistered.accepts_credentials_post());
        assert!(!PeerState::Unregistered.may_use_functional_modules());
    }

    #[test]
    fn the_newest_common_version_is_selected() {
        let versions = vec![
            Version::new(VersionNumber::V2_1_1, Url::new("https://e.com/2.1.1").unwrap()),
            Version::new(VersionNumber::V2_2_1, Url::new("https://e.com/2.2.1").unwrap()),
            Version::new("3.0".into(), Url::new("https://e.com/3.0").unwrap()),
        ];
        let discovered = Discovered {
            versions_url: Url::new("https://e.com/versions").unwrap(),
            token_a: token("A"),
            quirks_override: None,
            versions,
        };
        // 3.0 is advertised but this crate cannot speak it, so 2.2.1 wins.
        assert_eq!(discovered.best_common_version().map(|v| v.version.clone()), Some(VersionNumber::V2_2_1));
    }

    #[test]
    fn missing_required_endpoints_stop_the_handshake_before_anything_is_sent() {
        use crate::v2_3_0::versions::Endpoint;
        let details = VersionDetails::new(
            VersionNumber::V2_3_0,
            vec![Endpoint::new(
                ModuleId::Credentials,
                InterfaceRole::Sender,
                Url::new("https://e.com/credentials").unwrap(),
            )],
        );
        let selected = Selected {
            versions_url: Url::new("https://e.com/versions").unwrap(),
            token_a: token("A"),
            quirks: Quirks::default(),
            version: VersionNumber::V2_3_0,
            details,
        };
        assert!(selected.require(&[(ModuleId::Credentials, InterfaceRole::Sender)]).is_ok());
        let err = selected.require(&[(ModuleId::Cdrs, InterfaceRole::Receiver)]).unwrap_err();
        assert_eq!(err.status_code(), crate::transport::StatusCode::NO_MATCHING_ENDPOINTS);
        assert!(err.to_string().contains("cdrs"), "{err}");
    }
}
