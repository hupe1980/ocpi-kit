//! One set of handlers, two OCPI versions on the wire — and a hub between them.
//!
//! Everything above `transport` in this crate speaks the canonical OCPI 2.3.0 model. Most of the
//! market speaks 2.2.1. These tests are the proof that both statements can be true at once: a
//! router published as 2.2.1 answers with 2.2.1 bytes from handlers written against 2.3.0, a
//! client whose peer is on 2.2.1 hands its caller 2.3.0 objects, and a hub carries a message
//! between two parties that disagree about the version without either of them noticing.
//!
//! They assert on **the JSON that actually crossed the socket**, not on what the code intended to
//! write. A `Tariff` is the sharpest instrument here: 2.3.0 made `tax_included` a required field,
//! so a 2.3.0 Tariff handed to a 2.2.1 peer is not merely unusual, it is undecodable — which is
//! what makes an untranslated crate unusable against most of the peers in the field.

#![cfg(all(feature = "client", feature = "server", feature = "hub", feature = "testkit"))]

use std::sync::Arc;

use ocpi_kit::client::{ClientConfig, OcpiClient, Peer};
use ocpi_kit::server::{
    AuthenticatedPeer, Created, Handled, InMemoryTokenStore, LocationsReceiver, LocationsSender, OcpiRouter,
    RequestContext, TariffsSender,
};
use ocpi_kit::testkit::{InMemoryLocations, InMemoryTariffs, sample, test_cpo, test_msp, test_token};
use ocpi_kit::transport::{CredentialsToken, OcpiError, Page, PageQuery, Patch};
use ocpi_kit::types::{PartyRef, Url};
use ocpi_kit::v2_3_0::locations::Location;
use ocpi_kit::v2_3_0::tariffs::Tariff;
use ocpi_kit::{InterfaceRole, ModuleId, VersionNumber};

// ---------------------------------------------------------------------------------------------
// A CPO whose handlers only ever see the canonical model.
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Cpo {
    locations: Arc<InMemoryLocations>,
    tariffs: Arc<InMemoryTariffs>,
    base: Url,
}

impl LocationsSender for Cpo {
    async fn list(&self, query: PageQuery, _c: RequestContext) -> Handled<Page<Location>> {
        Ok(self.locations.page(&query, &self.base.join("locations")))
    }
    async fn location(&self, id: String, _c: RequestContext) -> Handled<Location> {
        self.locations.get(&id).ok_or_else(|| OcpiError::NotFound(format!("no Location {id}")))
    }
    async fn evse(
        &self,
        id: String,
        uid: String,
        _c: RequestContext,
    ) -> Handled<ocpi_kit::v2_3_0::locations::Evse> {
        self.locations
            .get(&id)
            .and_then(|l| l.evse(&uid).cloned())
            .ok_or_else(|| OcpiError::NotFound(format!("no EVSE {uid}")))
    }
    async fn connector(
        &self,
        id: String,
        uid: String,
        conn: String,
        _c: RequestContext,
    ) -> Handled<ocpi_kit::v2_3_0::locations::Connector> {
        self.locations
            .get(&id)
            .and_then(|l| l.evse(&uid).and_then(|e| e.connector(&conn).cloned()))
            .ok_or_else(|| OcpiError::NotFound(format!("no Connector {conn}")))
    }
}

impl LocationsReceiver for Cpo {
    async fn location(&self, _o: PartyRef, id: String, _c: RequestContext) -> Handled<Location> {
        self.locations.get(&id).ok_or_else(|| OcpiError::NotFound(format!("no Location {id}")))
    }
    async fn put_location(&self, _o: PartyRef, location: Location, _c: RequestContext) -> Handled<Created> {
        let created = self.locations.get(location.id.as_str()).is_none();
        self.locations.put(location);
        Ok(if created { Created::Yes } else { Created::No })
    }
    async fn put_evse(
        &self,
        _o: PartyRef,
        location_id: String,
        evse: ocpi_kit::v2_3_0::locations::Evse,
        _c: RequestContext,
    ) -> Handled<Created> {
        let mut location = self
            .locations
            .get(&location_id)
            .ok_or_else(|| OcpiError::NotFound(format!("no Location {location_id}")))?;
        let created = location.evse(evse.uid.as_str()).is_none();
        location.evses.retain(|e| e.uid != evse.uid);
        location.evses.push(evse);
        self.locations.put(location);
        Ok(if created { Created::Yes } else { Created::No })
    }
    async fn put_connector(
        &self,
        _o: PartyRef,
        _location_id: String,
        _evse_uid: String,
        _connector: ocpi_kit::v2_3_0::locations::Connector,
        _c: RequestContext,
    ) -> Handled<Created> {
        Err(OcpiError::NotFound("this test's CPO does not take connectors".to_owned()))
    }
    async fn patch(
        &self,
        _o: PartyRef,
        location_id: String,
        _evse_uid: Option<String>,
        _connector_id: Option<String>,
        patch: Patch<serde_json::Value>,
        _c: RequestContext,
    ) -> Handled<()> {
        let current = self
            .locations
            .get(&location_id)
            .ok_or_else(|| OcpiError::NotFound(format!("no Location {location_id}")))?;
        self.locations.put(patch.retype::<Location>().apply(&current)?);
        Ok(())
    }
}

impl TariffsSender for Cpo {
    async fn list(&self, query: PageQuery, _c: RequestContext) -> Handled<Page<Tariff>> {
        Ok(self.tariffs.page(&query, &self.base.join("tariffs")))
    }
}

// ---------------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------------

struct Running {
    base: Url,
    cpo: Cpo,
    _handle: tokio::task::JoinHandle<()>,
}

/// Starts a CPO publishing `version`, with 2.3.0 handlers behind it either way.
async fn start(version: VersionNumber) -> Running {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("can bind");
    let port = listener.local_addr().expect("has an address").port();
    let base = Url::new(format!("http://127.0.0.1:{port}")).expect("valid URL");

    let cpo = Cpo {
        locations: Arc::new(InMemoryLocations::new()),
        tariffs: Arc::new(InMemoryTariffs::new()),
        base: base.clone(),
    };
    let mut location = sample::location("LOC1").expect("valid sample");
    // Two fields OCPI 2.2.1 does not have, so the downgrade has something to report.
    location.help_phone = Some("+3212345678".parse().expect("a valid phone"));
    cpo.locations.put(location);
    cpo.tariffs.put(sample::tariff("T1", "0.25").expect("valid sample"));

    let tokens = Arc::new(InMemoryTokenStore::new());
    tokens.insert(
        test_token("c"),
        AuthenticatedPeer {
            peer_id: "msp".to_owned(),
            role: ocpi_kit::transport::TokenRole::C,
            parties: vec![test_msp()],
            version: version.clone(),
        },
    );

    let app = OcpiRouter::new(version, base.clone(), tokens)
        .locations_sender(cpo.clone())
        .locations_receiver(cpo.clone())
        .tariffs_sender(cpo.clone())
        .build();

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Running { base, cpo, _handle: handle }
}

fn client() -> OcpiClient {
    OcpiClient::with_config(ClientConfig::for_testing()).expect("can build a client")
}

fn peer_at(base: &Url, version: VersionNumber) -> Peer {
    Peer::builder(version, test_token("c"))
        .versions_url(base.join("versions"))
        .endpoint(ModuleId::Locations, InterfaceRole::Sender, base.join("locations"))
        .endpoint(ModuleId::Locations, InterfaceRole::Receiver, base.join("receiver").join("locations"))
        .endpoint(ModuleId::Tariffs, InterfaceRole::Sender, base.join("tariffs"))
        .party(test_cpo())
        .build()
}

/// The raw JSON a peer would see, with no help from this crate's client.
async fn raw(url: &Url) -> serde_json::Value {
    let response = reqwest::Client::new()
        .get(url.as_str())
        .header("Authorization", CredentialsToken::new("test-token-c").unwrap().to_header_value())
        .send()
        .await
        .expect("the server answers");
    response.json().await.expect("a JSON envelope")
}

// ---------------------------------------------------------------------------------------------
// The server writes the version it publishes
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_2_2_1_router_answers_with_2_2_1_objects() {
    let server = start(VersionNumber::V2_2_1).await;
    let body = raw(&server.base.join("tariffs")).await;
    let tariff = &body["data"][0];

    // 2.3.0 made `tax_included` required; 2.2.1 has no such field, and a 2.2.1 peer decoding one
    // with `deny_unknown_fields` — or simply reading it as a Tariff — must not see it.
    assert!(tariff.get("tax_included").is_none(), "leaked a 2.3.0 field: {tariff}");
    assert!(tariff.get("elements").is_some(), "and kept everything both versions share");

    // The same server on 2.3.0 writes the field, which is what makes the assertion above mean
    // something rather than describing a sample that never had it.
    let canonical = start(VersionNumber::V2_3_0).await;
    let body = raw(&canonical.base.join("tariffs")).await;
    assert_eq!(body["data"][0]["tax_included"], serde_json::json!("NO"));
}

#[tokio::test]
async fn a_field_2_2_1_cannot_hold_is_dropped_at_the_edge_and_nowhere_else() {
    let server = start(VersionNumber::V2_2_1).await;
    let body = raw(&server.base.join("locations").join("LOC1")).await;
    assert!(body["data"].get("help_phone").is_none(), "2.2.1 has no Location.help_phone");
    // The handler's own object is untouched: the translation happens on the way out.
    assert!(server.cpo.locations.get("LOC1").expect("still there").help_phone.is_some());
}

#[tokio::test]
async fn a_router_cannot_publish_a_version_this_build_cannot_write() {
    let tokens = Arc::new(InMemoryTokenStore::new());
    let base = Url::new("https://cpo.example.com/ocpi/cpo/2.1.1").expect("valid URL");
    let built = std::panic::catch_unwind(|| {
        let _ = OcpiRouter::new(VersionNumber::V2_1_1, base, tokens).build();
    });
    assert!(built.is_err(), "a 2.1.1 router would answer with 2.3.0 objects; it must not start");
}

// ---------------------------------------------------------------------------------------------
// The client reads whatever the peer speaks
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_2_2_1_peer_is_read_as_the_canonical_model() {
    let server = start(VersionNumber::V2_2_1).await;
    let client = client();
    let peer = peer_at(&server.base, VersionNumber::V2_2_1);

    // Without the bridge this fails outright: a 2.2.1 Tariff has no `tax_included`, and 2.3.0
    // requires it.
    let mut tariffs =
        peer.tariffs(client.transport(), test_msp()).list(PageQuery::new()).expect("Sender mounted");
    let tariff: Tariff = tariffs.next().await.expect("a page").expect("one tariff");
    assert_eq!(tariff.id.as_str(), "T1");
    assert_eq!(tariff.tax_included, ocpi_kit::v2_3_0::tariffs::TaxIncluded::No);

    let location = peer.locations(client.transport(), test_msp()).location("LOC1").await.expect("found");
    assert_eq!(location.id.as_str(), "LOC1");
    assert!(location.help_phone.is_none(), "the peer could not carry it, and says so by omission");
}

#[tokio::test]
async fn a_push_to_a_2_2_1_peer_is_written_in_2_2_1() {
    let server = start(VersionNumber::V2_2_1).await;
    let client = client();
    let peer = peer_at(&server.base, VersionNumber::V2_2_1);

    let mut location = sample::location("LOC2").expect("valid sample");
    location.help_phone = Some("+3299999999".parse().expect("a valid phone"));
    peer.locations_receiver(client.transport(), test_msp())
        // The URL's owner must be the authenticated peer's own party; a platform writing under
        // someone else's gets a 404, which is what the specification permits.
        .put_location(&test_msp(), &location)
        .await
        .expect("the peer accepts it");

    // It arrived, and the field 2.2.1 cannot express did not survive the crossing — which is the
    // honest outcome, and is why the client logs the loss rather than pretending.
    let stored = server.cpo.locations.get("LOC2").expect("the CPO stored it");
    assert_eq!(stored.id.as_str(), "LOC2");
    assert!(stored.help_phone.is_none());
}

#[tokio::test]
async fn a_patch_that_both_versions_read_the_same_way_goes_through() {
    let server = start(VersionNumber::V2_2_1).await;
    let client = client();
    let peer = peer_at(&server.base, VersionNumber::V2_2_1);

    let patch: Patch<Location> = Patch::from_value(serde_json::json!({
        "name": "Gent Noord",
        "last_updated": "2024-01-15T10:00:00Z",
    }));
    peer.locations_receiver(client.transport(), test_msp())
        .patch(&test_msp(), "LOC1", None, None, &patch)
        .await
        .expect("`name` means the same thing in both versions");
    assert_eq!(server.cpo.locations.get("LOC1").expect("still there").name.as_deref(), Some("Gent Noord"),);
}

#[tokio::test]
async fn a_patch_the_versions_disagree_about_is_refused_with_the_way_out() {
    let server = start(VersionNumber::V2_2_1).await;
    let client = client();
    let peer = peer_at(&server.base, VersionNumber::V2_2_1);

    // A merge patch is not an object, so it cannot be decoded, converted and re-encoded; and
    // `help_phone` is precisely a field the two versions do not share.
    let patch: Patch<Location> = Patch::from_value(serde_json::json!({
        "help_phone": "+3212345678",
        "last_updated": "2024-01-15T10:00:00Z",
    }));
    let error = peer
        .locations_receiver(client.transport(), test_msp())
        .patch(&test_msp(), "LOC1", None, None, &patch)
        .await
        .expect_err("this cannot be translated");
    assert!(matches!(error, OcpiError::Unsupported(_)), "{error}");
    assert!(error.to_string().contains("PUT"), "and it names the recovery: {error}");
}

// ---------------------------------------------------------------------------------------------
// A hub between two parties that disagree about the version
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_hub_carries_a_message_between_two_versions_and_says_what_it_cost() {
    use ocpi_kit::hub::{ConnectedPlatform, Forwardable, Forwarder, RoutingTable};
    use ocpi_kit::transport::{RequestIds, RoutingHeaders};
    use ocpi_kit::v2_3_0::hub_client_info::ConnectionStatus;
    use ocpi_kit::v2_3_0::types::Role;

    let downstream = start(VersionNumber::V2_2_1).await;
    let client = client();
    let table = RoutingTable::new();
    table.upsert(ConnectedPlatform {
        platform_id: "cpo".to_owned(),
        parties: vec![(test_cpo(), Role::Cpo)],
        status: ConnectionStatus::Connected,
        peer: peer_at(&downstream.base, VersionNumber::V2_2_1),
    });

    let hub_party = PartyRef::new("NL", "HUB").expect("a valid party");
    let forwarder = Forwarder::new(client.transport(), &table, hub_party.clone());

    // A 2.3.0 eMSP asks the hub for a 2.2.1 CPO's Location.
    let request = Forwardable {
        method: http::Method::GET,
        module: ModuleId::Locations,
        interface: InterfaceRole::Sender,
        path: "LOC1".to_owned(),
        query: None,
        routing: RoutingHeaders { to: Some(test_cpo()), from: test_msp() },
        ids: RequestIds::generate(),
        body: None,
        version: VersionNumber::V2_3_0,
    };
    let relayed = forwarder.relay(&request, &test_cpo(), RoutingHeaders::new(test_msp(), test_cpo())).await;
    let response = relayed.outcome.expect("the CPO answered");
    let data = response.data.expect("with a Location");

    // What the eMSP receives is a 2.3.0 Location, although nothing on the wire below the hub was.
    let location: Location = serde_json::from_value(data).expect("a canonical Location");
    assert_eq!(location.id.as_str(), "LOC1");

    // And the whole exchange the other way is refused rather than mistranslated.
    let mut unbridgeable = request;
    unbridgeable.version = VersionNumber::V2_1_1;
    unbridgeable.body = Some(br#"{"id":"LOC1"}"#.to_vec());
    unbridgeable.method = http::Method::PUT;
    unbridgeable.interface = InterfaceRole::Receiver;
    unbridgeable.path = "NL/TNM/LOC1".to_owned();
    let relayed =
        forwarder.relay(&unbridgeable, &test_cpo(), RoutingHeaders::new(test_msp(), test_cpo())).await;
    let error = relayed.outcome.expect_err("2.1.1 to 2.2.1 has no conversions");
    assert!(matches!(error, OcpiError::NotRoutable(_)), "{error}");
    assert!(error.to_string().contains("RelayVerbatim"), "and it names the other choice: {error}");

    // A hub that is deliberately a pipe can say so, and the same message goes out untranslated.
    let pipe = Forwarder::new(client.transport(), &table, hub_party)
        .on_unbridgeable(ocpi_kit::hub::Unbridgeable::RelayVerbatim);
    let relayed = pipe.relay(&unbridgeable, &test_cpo(), RoutingHeaders::new(test_msp(), test_cpo())).await;
    // It reached the CPO and was answered on OCPI's terms rather than refused by the hub.
    assert!(relayed.outcome.is_ok(), "{:?}", relayed.outcome.err());
}

#[tokio::test]
async fn a_hub_reports_what_a_downgrade_cost_in_the_status_message() {
    use ocpi_kit::hub::{ConnectedPlatform, Forwardable, Forwarder, RoutingTable};
    use ocpi_kit::transport::{RequestIds, RoutingHeaders};
    use ocpi_kit::v2_3_0::hub_client_info::ConnectionStatus;
    use ocpi_kit::v2_3_0::types::Role;

    // This time the CPO is canonical and the *requester* is on 2.2.1, so the response has to be
    // carried backwards — and the Location the CPO holds has a `help_phone` that cannot come.
    let downstream = start(VersionNumber::V2_3_0).await;
    let client = client();
    let table = RoutingTable::new();
    table.upsert(ConnectedPlatform {
        platform_id: "cpo".to_owned(),
        parties: vec![(test_cpo(), Role::Cpo)],
        status: ConnectionStatus::Connected,
        peer: peer_at(&downstream.base, VersionNumber::V2_3_0),
    });
    let forwarder =
        Forwarder::new(client.transport(), &table, PartyRef::new("NL", "HUB").expect("valid party"));

    let request = Forwardable {
        method: http::Method::GET,
        module: ModuleId::Locations,
        interface: InterfaceRole::Sender,
        path: "LOC1".to_owned(),
        query: None,
        routing: RoutingHeaders { to: Some(test_cpo()), from: test_msp() },
        ids: RequestIds::generate(),
        body: None,
        version: VersionNumber::V2_2_1,
    };
    let relayed = forwarder.relay(&request, &test_cpo(), RoutingHeaders::new(test_msp(), test_cpo())).await;
    let response = relayed.outcome.expect("the CPO answered");

    let message = response.status_message.expect("the hub says what the crossing cost");
    assert!(message.contains("help_phone"), "{message}");
    assert!(
        response.data.expect("still a Location").get("help_phone").is_none(),
        "and the field really is gone, rather than merely reported",
    );
}
