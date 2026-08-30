//! Drives the `ocpi-kit` server with the `ocpi-kit` client over a real TCP socket.
//!
//! This is the test that catches the things unit tests cannot: that the router's paths match the
//! client's URL builders, that the pagination headers one side writes are the ones the other
//! side reads, that the authorization header round-trips, and that the specification's status
//! code rules survive contact with HTTP.

#![cfg(all(feature = "client", feature = "server", feature = "testkit"))]

use std::sync::Arc;

use ocpi_kit::client::{ClientConfig, OcpiClient, Peer};
use ocpi_kit::server::{
    AuthenticatedPeer, Created, CredentialsHandler, Handled, InMemoryTokenStore, LocationsReceiver,
    LocationsSender, OcpiRouter, RequestContext, TokensSender,
};
use ocpi_kit::testkit::{InMemoryLocations, sample, test_cpo, test_msp, test_token};
use ocpi_kit::transport::{CredentialsToken, OcpiError, Page, PageQuery, Patch, StatusCode};
use ocpi_kit::types::{PartyRef, Url, Validate};
use ocpi_kit::v2_3_0::locations::Location;
use ocpi_kit::v2_3_0::tokens::{AllowedType, AuthorizationInfo, LocationReferences, Token, TokenType};
use ocpi_kit::{InterfaceRole, ModuleId, VersionNumber};

// ---------------------------------------------------------------------------------------------
// A minimal CPO, built out of the testkit's stores.
// ---------------------------------------------------------------------------------------------

struct Cpo {
    locations: InMemoryLocations,
    base: Url,
}

impl Cpo {
    fn new(base: Url, page_size: usize) -> Self {
        Self { locations: InMemoryLocations::with_page_size(page_size), base }
    }
}

/// A shared handle to the CPO.
///
/// A newtype rather than a bare `Arc<Cpo>`, because the orphan rule forbids implementing a
/// foreign trait for `Arc<T>` from outside the crate that defines `Arc`.
#[derive(Clone)]
struct SharedCpo(Arc<Cpo>);

impl core::ops::Deref for SharedCpo {
    type Target = Cpo;
    fn deref(&self) -> &Cpo {
        &self.0
    }
}

/// A shared handle to the eMSP, for the same reason.
#[derive(Clone)]
struct SharedMsp;

impl LocationsSender for SharedCpo {
    async fn list(&self, query: PageQuery, _context: RequestContext) -> Handled<Page<Location>> {
        Ok(self.locations.page(&query, &self.base.join("locations")))
    }

    async fn location(&self, location_id: String, _context: RequestContext) -> Handled<Location> {
        self.locations
            .get(&location_id)
            .ok_or_else(|| OcpiError::NotFound(format!("no Location {location_id}")))
    }

    async fn evse(
        &self,
        location_id: String,
        evse_uid: String,
        _context: RequestContext,
    ) -> Handled<ocpi_kit::v2_3_0::locations::Evse> {
        self.locations
            .get(&location_id)
            .and_then(|l| l.evse(&evse_uid).cloned())
            .ok_or_else(|| OcpiError::NotFound(format!("no EVSE {evse_uid}")))
    }

    async fn connector(
        &self,
        location_id: String,
        evse_uid: String,
        connector_id: String,
        _context: RequestContext,
    ) -> Handled<ocpi_kit::v2_3_0::locations::Connector> {
        self.locations
            .get(&location_id)
            .and_then(|l| l.evse(&evse_uid).and_then(|e| e.connector(&connector_id).cloned()))
            .ok_or_else(|| OcpiError::NotFound(format!("no Connector {connector_id}")))
    }
}

impl LocationsReceiver for SharedCpo {
    async fn location(
        &self,
        _owner: PartyRef,
        location_id: String,
        _context: RequestContext,
    ) -> Handled<Location> {
        self.locations
            .get(&location_id)
            .ok_or_else(|| OcpiError::NotFound(format!("no Location {location_id}")))
    }

    async fn put_location(
        &self,
        _owner: PartyRef,
        location: Location,
        _context: RequestContext,
    ) -> Handled<Created> {
        Ok(Created::from(self.locations.put(location)))
    }

    async fn put_evse(
        &self,
        _owner: PartyRef,
        _location_id: String,
        _evse: ocpi_kit::v2_3_0::locations::Evse,
        _context: RequestContext,
    ) -> Handled<Created> {
        Ok(Created::No)
    }

    async fn put_connector(
        &self,
        _owner: PartyRef,
        _location_id: String,
        _evse_uid: String,
        _connector: ocpi_kit::v2_3_0::locations::Connector,
        _context: RequestContext,
    ) -> Handled<Created> {
        Ok(Created::No)
    }

    async fn patch(
        &self,
        _owner: PartyRef,
        location_id: String,
        _evse_uid: Option<String>,
        _connector_id: Option<String>,
        patch: Patch<serde_json::Value>,
        _context: RequestContext,
    ) -> Handled<()> {
        let existing = self
            .locations
            .get(&location_id)
            .ok_or_else(|| OcpiError::NotFound(format!("no Location {location_id}")))?;
        let updated: Location = patch.retype::<Location>().apply(&existing)?;
        self.locations.put(updated);
        Ok(())
    }
}

impl CredentialsHandler for SharedCpo {
    async fn get(&self, _context: RequestContext) -> Handled<ocpi_kit::v2_3_0::credentials::Credentials> {
        sample::credentials("test-token-c", self.base.join("versions").as_str())
            .map_err(|e| OcpiError::Transport(e.to_string()))
    }

    async fn post(
        &self,
        _credentials: ocpi_kit::v2_3_0::credentials::Credentials,
        _context: RequestContext,
    ) -> Handled<ocpi_kit::v2_3_0::credentials::Credentials> {
        Err(OcpiError::MethodNotAllowed("this peer is already registered".to_owned()))
    }

    async fn put(
        &self,
        _credentials: ocpi_kit::v2_3_0::credentials::Credentials,
        context: RequestContext,
    ) -> Handled<ocpi_kit::v2_3_0::credentials::Credentials> {
        self.get(context).await
    }

    async fn delete(&self, _context: RequestContext) -> Handled<()> {
        Ok(())
    }
}

impl TokensSender for SharedMsp {
    async fn list(&self, query: PageQuery, _context: RequestContext) -> Handled<Page<Token>> {
        // Honouring `date_from` matters even in a one-object stub: a Sender interface that
        // returns the same page whatever the filter says turns a partner's incremental pull into
        // a full one, and the conformance runner checks for exactly that.
        let token = sample::token("012345678").expect("valid sample");
        let matches = query.date_from.is_none_or(|from| token.last_updated >= from)
            && query.date_to.is_none_or(|to| token.last_updated < to);
        Ok(Page::single(if matches { vec![token] } else { Vec::new() }))
    }

    async fn authorize(
        &self,
        token_uid: String,
        _token_type: Option<TokenType>,
        location: Option<LocationReferences>,
        _context: RequestContext,
    ) -> Handled<AuthorizationInfo> {
        if token_uid != "012345678" {
            return Err(OcpiError::Remote {
                status_code: StatusCode::UNKNOWN_TOKEN,
                status_message: Some(format!("no Token {token_uid}")),
            });
        }
        Ok(AuthorizationInfo::builder()
            .allowed(AllowedType::Allowed)
            .token(sample::token(&token_uid).expect("valid sample"))
            .maybe_location(location)
            .build())
    }
}

// ---------------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------------

struct Running {
    base: Url,
    _handle: tokio::task::JoinHandle<()>,
}

/// Starts the server on an ephemeral port and returns its base URL.
async fn start(page_size: usize, locations: usize) -> (Running, Arc<Cpo>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("can bind");
    let port = listener.local_addr().expect("has an address").port();
    let base = Url::new(format!("http://127.0.0.1:{port}")).expect("valid URL");

    let cpo = Arc::new(Cpo::new(base.clone(), page_size));
    for i in 0..locations {
        let mut location = sample::location(&format!("LOC{i}")).expect("valid sample");
        location.last_updated = ocpi_kit::types::DateTime::from_unix_timestamp(
            1_705_312_800 + i64::try_from(i).expect("small") * 60,
        )
        .expect("in range");
        cpo.locations.put(location);
    }

    let tokens = Arc::new(InMemoryTokenStore::new());
    tokens.insert(
        test_token("c"),
        AuthenticatedPeer {
            peer_id: "msp".to_owned(),
            role: ocpi_kit::transport::TokenRole::C,
            parties: vec![test_msp()],
            version: VersionNumber::V2_3_0,
        },
    );
    tokens.insert(
        test_token("a"),
        AuthenticatedPeer {
            peer_id: "bootstrap".to_owned(),
            role: ocpi_kit::transport::TokenRole::A,
            parties: vec![test_msp()],
            version: VersionNumber::V2_3_0,
        },
    );

    let app = OcpiRouter::new(VersionNumber::V2_3_0, base.clone(), tokens)
        .credentials(SharedCpo(Arc::clone(&cpo)))
        .locations_sender(SharedCpo(Arc::clone(&cpo)))
        .locations_receiver(SharedCpo(Arc::clone(&cpo)))
        .tokens_sender(SharedMsp)
        .build();

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (Running { base, _handle: handle }, cpo)
}

fn client() -> OcpiClient {
    OcpiClient::with_config(ClientConfig::for_testing()).expect("can build a client")
}

fn peer_at(base: &Url, token: CredentialsToken) -> Peer {
    Peer::builder(VersionNumber::V2_3_0, token)
        .versions_url(base.join("versions"))
        .endpoint(ModuleId::Credentials, InterfaceRole::Sender, base.join("credentials"))
        .endpoint(ModuleId::Locations, InterfaceRole::Sender, base.join("locations"))
        .endpoint(
            ModuleId::Locations,
            InterfaceRole::Receiver,
            // The server publishes its Receiver interfaces one segment deeper; see
            // `ServerConfig::receiver_path_prefix`.
            base.join("receiver").join("locations"),
        )
        .endpoint(ModuleId::Tokens, InterfaceRole::Sender, base.join("tokens"))
        .party(test_cpo())
        .build()
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_paginated_pull_crawls_every_page() {
    let (server, _cpo) = start(10, 25).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));

    let mut stream = peer
        .locations(client.transport(), test_msp())
        .list(PageQuery::new())
        .expect("the peer implements the Sender interface");

    let mut ids = Vec::new();
    while let Some(location) = stream.next().await.expect("the crawl succeeds") {
        location.validate().expect("the server sends conformant objects");
        ids.push(location.id.as_str().to_owned());
    }

    assert_eq!(ids.len(), 25, "every object was yielded exactly once");
    assert_eq!(stream.pages_fetched(), 3, "25 objects at 10 per page");
    assert_eq!(stream.total_count(), Some(25));
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 25, "no duplicates across the page boundaries");
}

#[tokio::test]
async fn a_date_window_narrows_the_pull_and_the_total_count() {
    let (server, cpo) = start(100, 10).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));

    let all = cpo.locations.all();
    let from = all[3].last_updated;
    let to = all[7].last_updated;

    let objects = peer
        .locations(client.transport(), test_msp())
        .list(PageQuery::between(from, to))
        .expect("mounted")
        .collect_all()
        .await
        .expect("the pull succeeds");

    // date_from inclusive, date_to exclusive.
    assert_eq!(objects.len(), 4);
}

#[tokio::test]
async fn one_object_can_be_fetched_by_id_and_a_missing_one_is_a_404() {
    let (server, _cpo) = start(100, 3).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let locations = peer.locations(client.transport(), test_msp());

    let found = locations.location("LOC1").await.expect("LOC1 exists");
    assert_eq!(found.id.as_str(), "LOC1");
    // `CiString` compares case-insensitively, all the way through the router.
    assert_eq!(locations.location("loc1").await.expect("same object").id.as_str(), "LOC1");

    let error = locations.location("NOPE").await.expect_err("no such Location");
    assert_eq!(error.http_status(), 404);
}

#[tokio::test]
async fn nested_evse_and_connector_urls_resolve() {
    let (server, _cpo) = start(100, 1).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let locations = peer.locations(client.transport(), test_msp());

    let evse = locations.evse("LOC0", "3256").await.expect("the sample EVSE");
    assert_eq!(evse.uid.as_str(), "3256");
    let connector = locations.connector("LOC0", "3256", "1").await.expect("the sample connector");
    assert_eq!(connector.id.as_str(), "1");
}

#[tokio::test]
async fn a_client_owned_put_then_patch_round_trips() {
    let (server, cpo) = start(100, 0).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let receiver = peer.locations_receiver(client.transport(), test_msp());

    let location = sample::location("LOC-NEW").expect("valid sample");
    receiver.put_location(&test_msp(), &location).await.expect("the PUT succeeds");
    assert_eq!(cpo.locations.len(), 1);

    // A PATCH must carry `last_updated`; this one does.
    let patch: Patch<Location> = Patch::from_value(serde_json::json!({
        "name": "Renamed",
        "last_updated": "2024-02-01T00:00:00Z",
    }));
    receiver.patch(&test_msp(), "LOC-NEW", None, None, &patch).await.expect("the PATCH succeeds");

    let stored = cpo.locations.get("LOC-NEW").expect("still there");
    assert_eq!(stored.name.as_deref(), Some("Renamed"));
    assert_eq!(stored.last_updated.to_string(), "2024-02-01T00:00:00Z");
}

#[tokio::test]
async fn a_failed_patch_falls_back_to_get_then_put() {
    // "In case a PATCH request fails, the client is expected to call the GET method to check the
    //  state of the object in the other party's system. If the object doesn't exist, the client
    //  should do a PUT."
    //
    // The spec describes a recovery *procedure*, not a single call, and getting it wrong means
    // either losing an update or resurrecting a deleted object. This walks it end to end.
    use ocpi_kit::transport::{PatchFallback, patch_fallback};

    let (server, cpo) = start(10, 0).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let locations = peer.locations_receiver(client.transport(), test_msp());
    let location = sample::location("LOC-NEW").expect("valid sample");

    // The object does not exist yet, so the PATCH fails.
    let patch = Patch::<Location>::from_value(serde_json::json!({
        "name": "Renamed",
        "last_updated": "2024-02-01T09:00:00Z",
    }));
    let failure = locations
        .patch(&test_msp(), "LOC-NEW", None, None, &patch)
        .await
        .expect_err("there is nothing to patch");

    // Step one: what does the fallback say to do?
    assert_eq!(
        patch_fallback(&failure),
        PatchFallback::PutWholeObject,
        "a 404 means the object is absent, so the whole object has to be sent",
    );

    // Step two: do it, and the object now exists with the whole state, not just the patched field.
    locations.put_location(&test_msp(), &location).await.expect("the PUT creates it");
    assert!(cpo.locations.get("LOC-NEW").is_some());

    // And now the same PATCH succeeds, because there is something to merge into.
    locations.patch(&test_msp(), "LOC-NEW", None, None, &patch).await.expect("the retry succeeds");
    let stored = cpo.locations.get("LOC-NEW").expect("stored");
    assert_eq!(stored.name.as_ref().map(ocpi_kit::types::OcpiString::as_str), Some("Renamed"));
    assert_eq!(stored.id.as_str(), "LOC-NEW", "the rest of the object is untouched");

    // A failure that is *not* a 404 means the object may well exist; reconcile before writing.
    assert_eq!(
        patch_fallback(&OcpiError::Transport("connection reset".into())),
        PatchFallback::GetThenReconcile,
        "blindly PUTting after an ambiguous failure would clobber concurrent updates",
    );
}

#[tokio::test]
async fn a_patch_without_last_updated_never_reaches_the_handler() {
    let (server, cpo) = start(100, 0).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let receiver = peer.locations_receiver(client.transport(), test_msp());
    receiver
        .put_location(&test_msp(), &sample::location("LOC-NEW").expect("valid"))
        .await
        .expect("the PUT succeeds");

    let patch: Patch<Location> = Patch::from_value(serde_json::json!({ "name": "Renamed" }));
    let error = receiver
        .patch(&test_msp(), "LOC-NEW", None, None, &patch)
        .await
        .expect_err("the spec's own example of a 2001");
    assert_eq!(error.status_code(), StatusCode::INVALID_PARAMETERS);
    assert_eq!(cpo.locations.get("LOC-NEW").expect("unchanged").name.as_deref(), Some("Gent Zuid"),);
}

#[tokio::test]
async fn writing_under_another_partys_id_is_refused_with_a_404() {
    let (server, cpo) = start(100, 0).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let receiver = peer.locations_receiver(client.transport(), test_msp());

    // The authenticated platform speaks for DE/ABC; this writes under NL/TNM.
    let error = receiver
        .put_location(&test_cpo(), &sample::location("LOC-NEW").expect("valid"))
        .await
        .expect_err("not this platform's party");
    assert_eq!(error.http_status(), 404, "a 404 does not reveal whether the object exists");
    assert!(cpo.locations.is_empty(), "nothing was written");
}

#[tokio::test]
async fn an_object_id_that_disagrees_with_the_url_is_a_2001() {
    let (server, _cpo) = start(100, 0).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));

    // Build the URL for one id but send an object with another.
    let endpoint = ocpi_kit::transport::ReceiverEndpoint::new(server.base.join("receiver").join("locations"));
    let url = endpoint.location(&test_msp(), "WRONG-ID", None, None);
    let module = peer.module(client.transport(), ModuleId::Locations, test_msp());
    let error =
        module.put(url, &sample::location("LOC-NEW").expect("valid")).await.expect_err("the ids disagree");
    assert_eq!(error.status_code(), StatusCode::INVALID_PARAMETERS);
}

#[tokio::test]
async fn a_bootstrap_token_cannot_reach_a_functional_module() {
    let (server, _cpo) = start(100, 1).await;
    let client = client();
    // `test_token("a")` is registered as CREDENTIALS_TOKEN_A.
    let peer = peer_at(&server.base, test_token("a"));

    let error = peer
        .locations(client.transport(), test_msp())
        .location("LOC0")
        .await
        .expect_err("TOKEN_A is scoped to credentials and versions");
    assert_eq!(error.http_status(), 401);

    // The same token works on the credentials module.
    let module = peer.module(client.transport(), ModuleId::Credentials, test_msp());
    let credentials: ocpi_kit::v2_3_0::credentials::Credentials =
        module.get(server.base.join("credentials")).await.expect("in scope");
    assert_eq!(credentials.roles.len(), 1);
}

#[tokio::test]
async fn an_unknown_token_is_a_401() {
    let (server, _cpo) = start(100, 1).await;
    let client = client();
    let peer = peer_at(&server.base, CredentialsToken::new("not-a-registered-token").unwrap());

    let error =
        peer.locations(client.transport(), test_msp()).location("LOC0").await.expect_err("unknown token");
    assert_eq!(error.http_status(), 401);
}

#[tokio::test]
async fn a_credentials_post_when_already_registered_is_a_405() {
    let (server, _cpo) = start(100, 0).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let module = peer.module(client.transport(), ModuleId::Credentials, test_msp());

    let credentials =
        sample::credentials("token-b", "https://msp.example.com/ocpi/versions").expect("valid sample");
    let error = module
        .post::<_, serde_json::Value>(server.base.join("credentials"), &credentials)
        .await
        .expect_err("already registered");
    assert_eq!(error.http_status(), 405);
}

#[tokio::test]
async fn a_real_time_authorization_round_trips_and_an_unknown_token_is_2004() {
    let (server, _cpo) = start(100, 0).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let tokens = peer.tokens(client.transport(), test_cpo());

    let info =
        tokens.authorize("012345678", Some(TokenType::AppUser), None).await.expect("the driver may charge");
    assert_eq!(info.allowed, AllowedType::Allowed);
    info.validate().expect("the eMSP sends a conformant object");

    let error = tokens.authorize("nope", None, None).await.expect_err("unknown token");
    assert_eq!(error.status_code(), StatusCode::UNKNOWN_TOKEN);
    assert_eq!(error.http_status(), 200, "an error that reached the OCPI layer is a 200");
}

#[tokio::test]
async fn the_versions_endpoints_describe_exactly_what_was_mounted() {
    let (server, _cpo) = start(100, 0).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let module = peer.module(client.transport(), ModuleId::Versions, test_msp());

    let versions: Vec<ocpi_kit::v2_3_0::versions::Version> =
        module.get(server.base.join("versions")).await.expect("the versions endpoint");
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].version, VersionNumber::V2_3_0);

    let details: ocpi_kit::v2_3_0::versions::VersionDetails =
        module.get(server.base.clone()).await.expect("the version details endpoint");
    details.validate().expect("generated details are conformant");

    // Exactly the four interfaces the harness mounted, and nothing else.
    assert!(details.endpoint(&ModuleId::Credentials, InterfaceRole::Sender).is_some());
    assert!(details.endpoint(&ModuleId::Locations, InterfaceRole::Sender).is_some());
    assert!(details.endpoint(&ModuleId::Locations, InterfaceRole::Receiver).is_some());
    assert!(details.endpoint(&ModuleId::Tokens, InterfaceRole::Sender).is_some());
    assert!(details.endpoint(&ModuleId::Cdrs, InterfaceRole::Sender).is_none());
    assert_eq!(details.endpoints.len(), 4);
}

#[tokio::test]
async fn the_registration_handshake_discovers_and_checks_endpoints() {
    let (server, _cpo) = start(100, 0).await;
    let client = client();

    let selected = ocpi_kit::client::Registration::new(server.base.join("versions"), test_token("a"))
        .discover(client.transport())
        .await
        .expect("discovery succeeds")
        .select_best(client.transport())
        .await
        .expect("2.3.0 is common");

    assert_eq!(selected.version(), &VersionNumber::V2_3_0);
    selected.require(&[(ModuleId::Locations, InterfaceRole::Sender)]).expect("the CPO serves Locations");

    // A module the peer does not implement stops the handshake before anything is POSTed.
    let error = selected
        .require(&[(ModuleId::Cdrs, InterfaceRole::Receiver)])
        .expect_err("the CPO does not serve CDRs");
    assert_eq!(error.status_code(), StatusCode::NO_MATCHING_ENDPOINTS);
}

#[tokio::test]
async fn the_client_refuses_to_send_a_non_conformant_object() {
    let (server, cpo) = start(100, 0).await;
    let client = client();
    let peer = peer_at(&server.base, test_token("c"));
    let receiver = peer.locations_receiver(client.transport(), test_msp());

    let mut location = sample::location("LOC-NEW").expect("valid sample");
    // A Location may not list `publish_allowed_to` while `publish` is true.
    location.publish_allowed_to = vec![ocpi_kit::v2_3_0::locations::PublishTokenType {
        group_id: Some(ocpi_kit::types::CiString::new("G1").expect("valid")),
        ..Default::default()
    }];

    let error = receiver
        .put_location(&test_msp(), &location)
        .await
        .expect_err("validated before it leaves the process");
    assert!(matches!(error, OcpiError::Invalid(_)), "{error}");
    assert!(cpo.locations.is_empty(), "nothing reached the peer");
}

#[tokio::test]
async fn every_response_echoes_the_request_and_correlation_ids() {
    let (server, _cpo) = start(100, 1).await;
    let http = reqwest::Client::new();
    let ids = ocpi_kit::transport::RequestIds::generate();

    let response = http
        .get(server.base.join("locations").as_str())
        .header("Authorization", test_token("c").to_header_value())
        .header("X-Request-ID", &ids.request_id)
        .header("X-Correlation-ID", &ids.correlation_id)
        .send()
        .await
        .expect("the request succeeds");

    assert_eq!(response.headers().get("x-request-id").unwrap(), ids.request_id.as_str());
    assert_eq!(response.headers().get("x-correlation-id").unwrap(), ids.correlation_id.as_str());
    assert_eq!(response.headers().get("x-limit").unwrap(), "100");
    assert_eq!(response.headers().get("x-total-count").unwrap(), "1");
}

#[tokio::test]
async fn a_limit_above_the_servers_maximum_never_reaches_a_handler() {
    // "X-Limit: The maximum number of objects that the server can return." The header is a
    // promise; a cap that is only advertised is not a cap, and a peer asking for a hundred
    // thousand objects is how a list endpoint becomes a denial of service.
    let (server, cpo) = start(1000, 40).await;
    let http = reqwest::Client::new();

    let response = http
        .get(server.base.join("locations").with_query("limit=100000").as_str())
        .header("Authorization", test_token("c").to_header_value())
        .send()
        .await
        .expect("the request succeeds");
    assert_eq!(response.headers().get("x-limit").unwrap(), "100", "the advertised maximum");

    let body: serde_json::Value = response.json().await.expect("an envelope");
    let returned = body["data"].as_array().expect("a list").len();
    assert!(returned <= 100, "the handler was asked for at most the maximum, got {returned}");
    assert_eq!(cpo.locations.len(), 40, "and the store really does hold more than one page");
}

#[tokio::test]
async fn a_body_that_is_not_json_is_a_400_and_a_wrong_object_is_a_2001() {
    let (server, _cpo) = start(100, 0).await;
    let http = reqwest::Client::new();
    let url = server.base.join("receiver").join("locations").join("DE").join("ABC").join("LOC1");

    let malformed = http
        .put(url.as_str())
        .header("Authorization", test_token("c").to_header_value())
        .header("Content-Type", "application/json")
        .body("{not json")
        .send()
        .await
        .expect("the request completes");
    assert_eq!(malformed.status(), 400, "the transport layer never reached OCPI");

    let wrong_shape = http
        .put(url.as_str())
        .header("Authorization", test_token("c").to_header_value())
        .header("Content-Type", "application/json")
        .body(r#"{"id":"LOC1"}"#)
        .send()
        .await
        .expect("the request completes");
    assert_eq!(wrong_shape.status(), 200, "an HTTP error status MUST NOT be returned here");
    let body: serde_json::Value = wrong_shape.json().await.expect("an OCPI envelope");
    assert_eq!(body["status_code"], 2001);
    // The path to the offending value is in the message, which is what makes this actionable.
    assert!(body["status_message"].as_str().unwrap().contains("country_code"), "{}", body["status_message"]);
}

#[tokio::test]
async fn a_missing_authorization_header_is_a_401_with_an_ocpi_envelope() {
    let (server, _cpo) = start(100, 0).await;
    let response = reqwest::Client::new()
        .get(server.base.join("locations").as_str())
        .send()
        .await
        .expect("the request completes");
    assert_eq!(response.status(), 401);
    let body: serde_json::Value = response.json().await.expect("still an OCPI envelope");
    assert_eq!(body["status_code"], 2000);
}

// ---------------------------------------------------------------------------------------------
// The conformance runner, pointed at this crate's own server.
//
// This is the test that keeps both honest: every check the runner makes is a rule the router is
// supposed to follow, so a failure means one of the two is wrong and the report says which.
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_conformance_runner_finds_nothing_wrong_with_our_own_server() {
    use ocpi_kit::client::{Conformance, Outcome};

    let (server, _cpo) = start(10, 25).await;
    let report =
        Conformance::new(server.base.join("versions"), test_token("c")).run(client().transport()).await;

    assert!(!report.has_failures(), "the router should satisfy every check the runner makes:\n{report}");
    assert!(report.count(Outcome::Pass) > 10, "the run should be substantial:\n{report}");
    assert_eq!(report.version.as_ref(), Some(&VersionNumber::V2_3_0));

    // The modules the test server does not mount must be skipped, not failed.
    assert!(
        report.checks.iter().any(|c| c.outcome == Outcome::Skipped),
        "sessions, cdrs and tariffs are not mounted here:\n{report}"
    );

    // The two checks that need a *second* request must have actually run rather than skipped:
    // a check that quietly does nothing is a check that passes for the wrong reason.
    for id in ["module.offset", "module.date_from"] {
        let check =
            report.checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id} never ran:\n{report}"));
        assert_eq!(check.outcome, Outcome::Pass, "{id}: {}", check.detail);
    }
}

#[tokio::test]
async fn the_conformance_runner_reports_an_unreachable_peer_rather_than_erroring() {
    use ocpi_kit::client::Conformance;

    // Nothing is listening on this port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("can bind");
    let port = listener.local_addr().expect("has an address").port();
    drop(listener);

    let report = Conformance::new(
        Url::new(format!("http://127.0.0.1:{port}/versions")).expect("valid URL"),
        test_token("c"),
    )
    .run(client().transport())
    .await;

    assert!(report.has_failures(), "a peer that cannot be reached is a finding");
    assert_eq!(report.failures().next().map(|c| c.id), Some("versions.get"));
}

#[tokio::test]
async fn the_conformance_runner_catches_an_unauthenticated_read() {
    use ocpi_kit::client::{Conformance, Outcome};

    let (server, _cpo) = start(10, 3).await;
    let report =
        Conformance::new(server.base.join("versions"), test_token("c")).run(client().transport()).await;

    // Both auth probes must have run and both must have been refused.
    let auth: Vec<&ocpi_kit::client::Check> =
        report.checks.iter().filter(|c| c.id.starts_with("auth.")).collect();
    assert_eq!(auth.len(), 2, "both auth probes should run:\n{report}");
    for check in auth {
        assert_eq!(check.outcome, Outcome::Pass, "{}: {}", check.id, check.detail);
    }
}
