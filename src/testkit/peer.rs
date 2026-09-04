//! A complete, runnable OCPI party backed by the in-memory stores.
//!
//! Every OCPI integration needs something at the other end of the socket before the partner is
//! ready, and that something is the same dozen handler traits over a `HashMap` for everybody.
//! [`MockPeer`] is that implementation, tested here rather than in each user's repository. It is
//! what `ocpi serve-mock` runs.
//!
//! ```no_run
//! use ocpi_kit::server::OcpiRouter;
//! use ocpi_kit::testkit::{MockPeer, sample};
//! use ocpi_kit::types::Url;
//! use ocpi_kit::VersionNumber;
//!
//! # async fn serve() -> Result<(), Box<dyn std::error::Error>> {
//! let base = Url::new("http://127.0.0.1:8080")?;
//! let peer = MockPeer::cpo(base.clone());
//! peer.locations.put(sample::location("LOC1")?);
//!
//! let app = peer.mount(OcpiRouter::new(VersionNumber::V2_3_0, base, peer.token_store())).build();
//! axum::serve(tokio::net::TcpListener::bind("127.0.0.1:8080").await?, app).await?;
//! # Ok(())
//! # }
//! ```
//!
//! It is **conformant**: pagination, `date_from`/`date_to`, ownership, `Created` vs updated, the
//! `2004 Unknown Token` code, the PATCH rule. The test suite points
//! [`Conformance`](crate::client::Conformance) at it and requires a clean report.
//!
//! It is **not** a charge point. Commands, Charging Profiles and Payments are deliberately not
//! mounted: a mock that answered `ACCEPTED` and never called the `response_url` back would teach a
//! client the wrong lesson, and one that did call back would need a Charge Point to have an
//! opinion. Version discovery advertises exactly what is mounted.

use std::sync::Arc;

use crate::server::{
    CdrsReceiver, CdrsSender, Created, CredentialsHandler, Handled, InMemoryTokenStore, LocationsReceiver,
    LocationsSender, OcpiRouter, RequestContext, SessionsReceiver, SessionsSender, TariffsReceiver,
    TariffsSender, TokensReceiver, TokensSender,
};
use crate::testkit;
use crate::transport::{OcpiError, Page, PageQuery, Patch, StatusCode};
use crate::types::{PartyRef, Url};
use crate::v2_3_0::cdrs::Cdr;
use crate::v2_3_0::credentials::Credentials;
use crate::v2_3_0::locations::{Connector, Evse, Location};
use crate::v2_3_0::sessions::{ChargingPreferences, ChargingPreferencesResponse, Session};
use crate::v2_3_0::tariffs::Tariff;
use crate::v2_3_0::tokens::{AllowedType, AuthorizationInfo, LocationReferences, Token, TokenType};
use crate::v2_3_0::types::Role;

use super::stores::{InMemoryCdrs, InMemoryLocations, InMemorySessions, InMemoryTariffs, InMemoryTokens};

/// A conformant OCPI party, held in memory.
///
/// Cheap to clone: the stores are shared, so a handle handed to the router and a handle kept for
/// the test see the same objects.
#[derive(Clone, Debug)]
pub struct MockPeer(Arc<MockPeerStores>);

/// The objects a [`MockPeer`] serves, reached through its `Deref`.
///
/// Seed and inspect them directly — `peer.locations.put(location)`, `peer.cdrs.len()` — which is
/// what a test needs and what a handler trait cannot give you.
#[derive(Debug)]
pub struct MockPeerStores {
    /// The Locations this peer serves.
    pub locations: InMemoryLocations,
    /// The Sessions this peer serves.
    pub sessions: InMemorySessions,
    /// The CDRs this peer serves, and the ones a partner has POSTed to it.
    pub cdrs: InMemoryCdrs,
    /// The Tariffs this peer serves.
    pub tariffs: InMemoryTariffs,
    /// The Tokens this peer serves, and authorizes against.
    pub tokens: InMemoryTokens,
    base: Url,
    party: PartyRef,
    role: Role,
}

impl core::ops::Deref for MockPeer {
    type Target = MockPeerStores;
    fn deref(&self) -> &MockPeerStores {
        &self.0
    }
}

impl MockPeer {
    /// A peer filling `role`, publishing its endpoints under `base`.
    #[must_use]
    pub fn new(base: Url, party: PartyRef, role: Role) -> Self {
        Self(Arc::new(MockPeerStores {
            base,
            party,
            role,
            locations: InMemoryLocations::new(),
            sessions: InMemorySessions::new(),
            cdrs: InMemoryCdrs::new(),
            tariffs: InMemoryTariffs::new(),
            tokens: InMemoryTokens::new(),
        }))
    }

    /// A CPO at [`test_cpo`](super::test_cpo).
    #[must_use]
    pub fn cpo(base: Url) -> Self {
        Self::new(base, super::test_cpo(), Role::Cpo)
    }

    /// An eMSP at [`test_msp`](super::test_msp).
    #[must_use]
    pub fn msp(base: Url) -> Self {
        Self::new(base, super::test_msp(), Role::Emsp)
    }

    /// Fills every store with one conformant object, so a partner's first pull is not empty.
    ///
    /// # Panics
    ///
    /// Panics if the crate's own sample objects are not constructible, which would be a bug here.
    #[must_use]
    pub fn seeded(self) -> Self {
        self.locations.put(testkit::sample::location("LOC1").expect("a valid sample Location"));
        self.sessions.put(testkit::sample::session("101").expect("a valid sample Session"));
        self.cdrs.put(testkit::sample::cdr("CDR1").expect("a valid sample CDR"));
        self.tariffs.put(testkit::sample::tariff("T1", "0.25").expect("a valid sample Tariff"));
        self.tokens.put(testkit::sample::token("012345678").expect("a valid sample Token"));
        self
    }

    /// The party this peer speaks as.
    #[must_use]
    pub fn party(&self) -> &PartyRef {
        &self.0.party
    }

    /// A token store that accepts [`test_token("c")`](super::test_token) as this peer's partner.
    ///
    /// The partner is given the *opposite* role's party, so the router's ownership check behaves
    /// the way it would in a real deployment: a partner may write under its own party and no
    /// other.
    #[must_use]
    pub fn token_store(&self) -> Arc<InMemoryTokenStore> {
        let partner = if self.role == Role::Cpo { super::test_msp() } else { super::test_cpo() };
        let store = Arc::new(InMemoryTokenStore::new());
        store.insert(super::test_token("c"), super::registered_peer("partner", vec![partner.clone()]));
        store.insert(super::test_token("a"), super::bootstrap_peer("bootstrap", vec![partner]));
        store
    }

    /// Mounts every module this peer serves onto `router`.
    ///
    /// Both interfaces of every object module, so one process can stand in for either side of a
    /// roaming relationship. That is only mountable because the router publishes the Receiver
    /// interfaces one segment deeper by default; see
    /// [`ServerConfig::receiver_path_prefix`](crate::server::ServerConfig::receiver_path_prefix).
    #[must_use]
    pub fn mount(&self, router: OcpiRouter) -> OcpiRouter {
        router
            .credentials(self.clone())
            .locations_sender(self.clone())
            .locations_receiver(self.clone())
            .sessions_sender(self.clone())
            .sessions_receiver(self.clone())
            .cdrs_sender(self.clone())
            .cdrs_receiver(self.clone())
            .tariffs_sender(self.clone())
            .tariffs_receiver(self.clone())
            .tokens_sender(self.clone())
            .tokens_receiver(self.clone())
    }

    fn endpoint(&self, module: &str) -> Url {
        self.base.join(module)
    }
}

/// `404` for an object this peer does not hold.
fn missing(what: &str, id: &str) -> OcpiError {
    OcpiError::NotFound(format!("no {what} {id}"))
}

// ---------------------------------------------------------------------------------------------
// Locations
// ---------------------------------------------------------------------------------------------

impl LocationsSender for MockPeer {
    async fn list(&self, query: PageQuery, _c: RequestContext) -> Handled<Page<Location>> {
        Ok(self.locations.page(&query, &self.endpoint("locations")))
    }

    async fn location(&self, location_id: String, _c: RequestContext) -> Handled<Location> {
        self.locations.get(&location_id).ok_or_else(|| missing("Location", &location_id))
    }

    async fn evse(&self, location_id: String, evse_uid: String, _c: RequestContext) -> Handled<Evse> {
        self.locations
            .get(&location_id)
            .and_then(|l| l.evse(&evse_uid).cloned())
            .ok_or_else(|| missing("EVSE", &evse_uid))
    }

    async fn connector(
        &self,
        location_id: String,
        evse_uid: String,
        connector_id: String,
        _c: RequestContext,
    ) -> Handled<Connector> {
        self.locations
            .get(&location_id)
            .and_then(|l| l.evse(&evse_uid).and_then(|e| e.connector(&connector_id).cloned()))
            .ok_or_else(|| missing("Connector", &connector_id))
    }
}

impl LocationsReceiver for MockPeer {
    async fn location(&self, _o: PartyRef, location_id: String, _c: RequestContext) -> Handled<Location> {
        self.locations.get(&location_id).ok_or_else(|| missing("Location", &location_id))
    }

    async fn put_location(&self, _o: PartyRef, location: Location, _c: RequestContext) -> Handled<Created> {
        Ok(Created::from(self.locations.put(location)))
    }

    async fn put_evse(
        &self,
        _o: PartyRef,
        location_id: String,
        evse: Evse,
        _c: RequestContext,
    ) -> Handled<Created> {
        let mut location =
            self.locations.get(&location_id).ok_or_else(|| missing("Location", &location_id))?;
        let created = location.evse(evse.uid.as_str()).is_none();
        location.evses.retain(|e| !e.uid.eq_ignore_case(evse.uid.as_str()));
        location.evses.push(evse);
        self.locations.put(location);
        Ok(Created::from(created))
    }

    async fn put_connector(
        &self,
        _o: PartyRef,
        location_id: String,
        evse_uid: String,
        connector: Connector,
        _c: RequestContext,
    ) -> Handled<Created> {
        let mut location =
            self.locations.get(&location_id).ok_or_else(|| missing("Location", &location_id))?;
        let evse = location
            .evses
            .iter_mut()
            .find(|e| e.uid.eq_ignore_case(&evse_uid))
            .ok_or_else(|| missing("EVSE", &evse_uid))?;
        let created = evse.connector(connector.id.as_str()).is_none();
        evse.connectors.retain(|c| !c.id.eq_ignore_case(connector.id.as_str()));
        evse.connectors.push(connector);
        self.locations.put(location);
        Ok(Created::from(created))
    }

    async fn patch(
        &self,
        _o: PartyRef,
        location_id: String,
        evse_uid: Option<String>,
        connector_id: Option<String>,
        patch: Patch<serde_json::Value>,
        _c: RequestContext,
    ) -> Handled<()> {
        let mut location =
            self.locations.get(&location_id).ok_or_else(|| missing("Location", &location_id))?;
        match (evse_uid, connector_id) {
            (None, _) => location = patch.retype::<Location>().apply(&location)?,
            (Some(uid), None) => {
                let evse = location
                    .evses
                    .iter_mut()
                    .find(|e| e.uid.eq_ignore_case(&uid))
                    .ok_or_else(|| missing("EVSE", &uid))?;
                *evse = patch.retype::<Evse>().apply(evse)?;
            }
            (Some(uid), Some(id)) => {
                let evse = location
                    .evses
                    .iter_mut()
                    .find(|e| e.uid.eq_ignore_case(&uid))
                    .ok_or_else(|| missing("EVSE", &uid))?;
                let connector = evse
                    .connectors
                    .iter_mut()
                    .find(|c| c.id.eq_ignore_case(&id))
                    .ok_or_else(|| missing("Connector", &id))?;
                *connector = patch.retype::<Connector>().apply(connector)?;
            }
        }
        self.locations.put(location);
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------------------------

impl SessionsSender for MockPeer {
    async fn list(&self, query: PageQuery, _c: RequestContext) -> Handled<Page<Session>> {
        Ok(self.sessions.page(&query, &self.endpoint("sessions")))
    }

    async fn set_charging_preferences(
        &self,
        session_id: String,
        _preferences: ChargingPreferences,
        _c: RequestContext,
    ) -> Handled<ChargingPreferencesResponse> {
        // "If a PUT with ChargingPreferences is received for an EVSE that does not have the
        //  capability CHARGING_PREFERENCES_CAPABLE, the receiver should respond with an HTTP
        //  status of 404 and an OCPI status code of 2001." The sample EVSE does not have it, so
        //  this mock answers the honest thing rather than pretending to accept a preference it
        //  has nowhere to apply.
        self.sessions.get(&session_id).ok_or_else(|| missing("Session", &session_id))?;
        Ok(ChargingPreferencesResponse::NotPossible)
    }
}

impl SessionsReceiver for MockPeer {
    async fn session(&self, _o: PartyRef, session_id: String, _c: RequestContext) -> Handled<Session> {
        self.sessions.get(&session_id).ok_or_else(|| missing("Session", &session_id))
    }

    async fn put_session(&self, _o: PartyRef, session: Session, _c: RequestContext) -> Handled<Created> {
        Ok(Created::from(self.sessions.put(session)))
    }

    async fn patch_session(
        &self,
        _o: PartyRef,
        session_id: String,
        patch: Patch<Session>,
        _c: RequestContext,
    ) -> Handled<()> {
        let current = self.sessions.get(&session_id).ok_or_else(|| missing("Session", &session_id))?;
        self.sessions.put(patch.apply(&current)?);
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// CDRs
// ---------------------------------------------------------------------------------------------

impl CdrsSender for MockPeer {
    async fn list(&self, query: PageQuery, _c: RequestContext) -> Handled<Page<Cdr>> {
        Ok(self.cdrs.page(&query, &self.endpoint("cdrs")))
    }
}

impl CdrsReceiver for MockPeer {
    async fn cdr(&self, cdr_id: String, _c: RequestContext) -> Handled<Cdr> {
        self.cdrs.get(&cdr_id).ok_or_else(|| missing("CDR", &cdr_id))
    }

    async fn post_cdr(&self, cdr: Cdr, _c: RequestContext) -> Handled<Url> {
        // "The eMSP returns the URL to the just created CDR object in the Location header field."
        // A CDR is immutable, so re-POSTing one is the peer's mistake, not ours to overwrite.
        if self.cdrs.get(cdr.id.as_str()).is_some() {
            return Err(OcpiError::Remote {
                status_code: StatusCode::INVALID_PARAMETERS,
                status_message: Some(format!("CDR {} has already been received", cdr.id)),
            });
        }
        let url = self.endpoint("cdrs").join_segment(cdr.id.as_str());
        self.cdrs.put(cdr);
        Ok(url)
    }
}

// ---------------------------------------------------------------------------------------------
// Tariffs
// ---------------------------------------------------------------------------------------------

impl TariffsSender for MockPeer {
    async fn list(&self, query: PageQuery, _c: RequestContext) -> Handled<Page<Tariff>> {
        Ok(self.tariffs.page(&query, &self.endpoint("tariffs")))
    }
}

impl TariffsReceiver for MockPeer {
    async fn tariff(&self, _o: PartyRef, tariff_id: String, _c: RequestContext) -> Handled<Tariff> {
        self.tariffs.get(&tariff_id).ok_or_else(|| missing("Tariff", &tariff_id))
    }

    async fn put_tariff(&self, _o: PartyRef, tariff: Tariff, _c: RequestContext) -> Handled<Created> {
        Ok(Created::from(self.tariffs.put(tariff)))
    }

    async fn delete_tariff(&self, _o: PartyRef, tariff_id: String, _c: RequestContext) -> Handled<()> {
        if self.tariffs.remove(&tariff_id) { Ok(()) } else { Err(missing("Tariff", &tariff_id)) }
    }
}

// ---------------------------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------------------------

impl TokensSender for MockPeer {
    async fn list(&self, query: PageQuery, _c: RequestContext) -> Handled<Page<Token>> {
        Ok(self.tokens.page(&query, &self.endpoint("tokens")))
    }

    async fn authorize(
        &self,
        token_uid: String,
        _token_type: Option<TokenType>,
        location: Option<LocationReferences>,
        _c: RequestContext,
    ) -> Handled<AuthorizationInfo> {
        let token = self.tokens.get(&token_uid).ok_or(OcpiError::Remote {
            status_code: StatusCode::UNKNOWN_TOKEN,
            status_message: Some(format!("no Token {token_uid}")),
        })?;
        let allowed = if token.valid { AllowedType::Allowed } else { AllowedType::Blocked };
        // "Only the EVSEs the EV driver is allowed to charge at are returned" — and a location is
        // only returned at all when the answer is ALLOWED, which `AuthorizationInfo::validate`
        // enforces.
        let location = location.filter(|_| allowed == AllowedType::Allowed);
        Ok(AuthorizationInfo::builder().allowed(allowed).token(token).maybe_location(location).build())
    }
}

impl TokensReceiver for MockPeer {
    async fn token(
        &self,
        _o: PartyRef,
        token_uid: String,
        _token_type: Option<TokenType>,
        _c: RequestContext,
    ) -> Handled<Token> {
        self.tokens.get(&token_uid).ok_or_else(|| missing("Token", &token_uid))
    }

    async fn put_token(&self, _o: PartyRef, token: Token, _c: RequestContext) -> Handled<Created> {
        Ok(Created::from(self.tokens.put(token)))
    }

    async fn patch_token(
        &self,
        _o: PartyRef,
        token_uid: String,
        _token_type: Option<TokenType>,
        patch: Patch<Token>,
        _c: RequestContext,
    ) -> Handled<()> {
        let current = self.tokens.get(&token_uid).ok_or_else(|| missing("Token", &token_uid))?;
        self.tokens.put(patch.apply(&current)?);
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------------------------

impl CredentialsHandler for MockPeer {
    async fn get(&self, _c: RequestContext) -> Handled<Credentials> {
        Ok(self.credentials())
    }

    async fn post(&self, _credentials: Credentials, _c: RequestContext) -> Handled<Credentials> {
        // A real implementation fetches the client's versions and version details with the token
        // it was just given, and answers `3001` if that fails. This one has nothing to fetch
        // from and no state to keep, so it answers with its own credentials and says so here
        // rather than pretending the handshake completed.
        Ok(self.credentials())
    }

    async fn put(&self, _credentials: Credentials, _c: RequestContext) -> Handled<Credentials> {
        Ok(self.credentials())
    }

    async fn delete(&self, _c: RequestContext) -> Handled<()> {
        Ok(())
    }
}

impl MockPeer {
    /// This peer's own credentials object, as the credentials endpoint returns it.
    #[must_use]
    pub fn credentials(&self) -> Credentials {
        use crate::v2_3_0::credentials::CredentialsRole;
        use crate::v2_3_0::locations::BusinessDetails;
        Credentials::builder()
            .token(super::test_token("c").to_credentials_field())
            .url(self.base.join("versions"))
            .roles(vec![
                CredentialsRole::builder()
                    .role(self.role)
                    .business_details(BusinessDetails::builder().name("ocpi-kit mock peer").build())
                    .party_id(self.party.party_id.clone())
                    .country_code(self.party.country_code.clone())
                    .build(),
            ])
            .build()
    }
}
