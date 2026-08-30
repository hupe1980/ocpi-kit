//! The hub, driven end to end: a real hub relaying to real downstream servers over real sockets.
//!
//! The hub is the part of OCPI where the specification's five routing tables become code, and
//! where getting `OCPI-to-`/`OCPI-from-` wrong is invisible until a partner complains. So these
//! tests assert on the **headers the downstream party actually received**, not on what the
//! forwarder intended to send. The same goes for the `X-Request-ID`/`X-Correlation-ID` rule,
//! which is one sentence in the spec and a data-loss bug in practice.

#![cfg(all(feature = "hub", feature = "testkit"))]

use std::sync::Arc;
use std::sync::Mutex;

use http::Method;
use ocpi_kit::client::{ClientConfig, OcpiClient, Peer};
use ocpi_kit::hub::{
    AggregatePolicy, BodyOwnerRouter, ConnectedPlatform, Forwardable, Forwarder, OpenRouter, RoutingTable,
    aggregate,
};
use ocpi_kit::testkit::test_token;
use ocpi_kit::transport::{
    OcpiError, RequestIds, RoutingHeaders, RoutingScenario, StatusCode, headers as hdr,
};
use ocpi_kit::types::{PartyRef, Url};
use ocpi_kit::v2_3_0::hub_client_info::ConnectionStatus;
use ocpi_kit::v2_3_0::types::Role;
use ocpi_kit::{InterfaceRole, ModuleId, VersionNumber};

/// What one downstream party saw.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    method: String,
    path: String,
    query: Option<String>,
    to: Option<String>,
    from: Option<String>,
    request_id: String,
    correlation_id: String,
    body: String,
}

/// A downstream party that records every request and answers `1000`.
struct Downstream {
    base: Url,
    seen: Arc<Mutex<Vec<Seen>>>,
    _handle: tokio::task::JoinHandle<()>,
}

impl Downstream {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("seen lock").clone()
    }
}

async fn downstream() -> Downstream {
    use axum::extract::{RawQuery, State};
    use axum::routing::any;

    let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
    let app = axum::Router::new()
        .route(
            "/{*rest}",
            any(
                async move |State(seen): State<Arc<Mutex<Vec<Seen>>>>,
                            method: Method,
                            uri: http::Uri,
                            RawQuery(query): RawQuery,
                            headers: http::HeaderMap,
                            body: axum::body::Bytes| {
                    let header = |name: &http::HeaderName| {
                        headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
                    };
                    let party = |country: &http::HeaderName, id: &http::HeaderName| {
                        Some(format!("{}/{}", header(country)?, header(id)?))
                    };
                    seen.lock().expect("seen lock").push(Seen {
                        method: method.to_string(),
                        path: uri.path().to_owned(),
                        query,
                        to: party(&hdr::OCPI_TO_COUNTRY_CODE, &hdr::OCPI_TO_PARTY_ID),
                        from: party(&hdr::OCPI_FROM_COUNTRY_CODE, &hdr::OCPI_FROM_PARTY_ID),
                        request_id: header(&hdr::X_REQUEST_ID).unwrap_or_default(),
                        correlation_id: header(&hdr::X_CORRELATION_ID).unwrap_or_default(),
                        body: String::from_utf8_lossy(&body).into_owned(),
                    });
                    axum::Json(serde_json::json!({
                        "status_code": 1000,
                        "timestamp": "2024-01-15T10:00:00Z",
                    }))
                },
            ),
        )
        .with_state(Arc::clone(&seen));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("can bind");
    let port = listener.local_addr().expect("has an address").port();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Downstream {
        base: Url::new(format!("http://127.0.0.1:{port}")).expect("valid URL"),
        seen,
        _handle: handle,
    }
}

fn hub_party() -> PartyRef {
    PartyRef::new("NL", "HUB").expect("a valid party")
}
fn cpo() -> PartyRef {
    PartyRef::new("NL", "TNM").expect("a valid party")
}
fn msp() -> PartyRef {
    PartyRef::new("DE", "ABC").expect("a valid party")
}
fn other_msp() -> PartyRef {
    PartyRef::new("FR", "XYZ").expect("a valid party")
}

fn platform(id: &str, party: &PartyRef, role: Role, base: &Url) -> ConnectedPlatform {
    ConnectedPlatform {
        platform_id: id.to_owned(),
        parties: vec![(party.clone(), role)],
        status: ConnectionStatus::Connected,
        peer: Peer::builder(VersionNumber::V2_3_0, test_token("c"))
            .versions_url(base.join("versions"))
            .endpoint(ModuleId::Locations, InterfaceRole::Sender, base.join("locations"))
            .endpoint(ModuleId::Locations, InterfaceRole::Receiver, base.join("locations"))
            .party(party.clone())
            .build(),
    }
}

fn forwardable(method: Method, to: Option<PartyRef>, interface: InterfaceRole) -> Forwardable {
    Forwardable {
        method,
        module: ModuleId::Locations,
        interface,
        path: String::new(),
        query: None,
        routing: RoutingHeaders { to, from: cpo() },
        ids: RequestIds::generate(),
        body: None,
        version: VersionNumber::V2_3_0,
    }
}

fn client() -> OcpiClient {
    OcpiClient::with_config(ClientConfig::for_testing()).expect("can build a client")
}

// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_relayed_request_keeps_the_correlation_id_and_gets_a_fresh_request_id() {
    // "the request to this party SHALL contain a new unique value in the X-Request-ID HTTP
    // header, not a copy … the request SHALL contain the same X-Correlation-ID HTTP header."
    let party = downstream().await;
    let table = RoutingTable::new();
    table.upsert(platform("msp", &msp(), Role::Emsp, &party.base));

    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let mut request = forwardable(Method::PUT, Some(msp()), InterfaceRole::Receiver);
    request.path = "NL/TNM/LOC1".to_owned();
    request.body = Some(br#"{"id":"LOC1"}"#.to_vec());
    let original = request.ids.clone();

    let relayed = forwarder.relay(&request, &msp(), RoutingHeaders::new(cpo(), msp())).await;
    assert!(relayed.is_success(), "{:?}", relayed.outcome);

    let seen = party.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].correlation_id, original.correlation_id, "the correlation id is carried through");
    assert_ne!(seen[0].request_id, original.request_id, "the request id is new for this hop");
    assert!(!seen[0].request_id.is_empty());
    assert_eq!(seen[0].path, "/locations/NL/TNM/LOC1", "the client-owned path is preserved verbatim");
    assert_eq!(seen[0].body, r#"{"id":"LOC1"}"#, "the body is relayed untouched");
}

#[tokio::test]
async fn a_broadcast_push_reaches_every_opposite_role_speaking_in_the_hubs_name() {
    // "Broadcast request | Hub to receiving platform | Receiving-party | Hub"
    let a = downstream().await;
    let b = downstream().await;
    let sender = downstream().await;
    let table = RoutingTable::new();
    table.upsert(platform("msp-a", &msp(), Role::Emsp, &a.base));
    table.upsert(platform("msp-b", &other_msp(), Role::Emsp, &b.base));
    // The CPO that sent the push is not itself a target: a broadcast goes to opposite roles.
    table.upsert(platform("cpo", &cpo(), Role::Cpo, &sender.base));

    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let mut request = forwardable(Method::PUT, Some(hub_party()), InterfaceRole::Receiver);
    request.path = "NL/TNM/LOC1".to_owned();
    request.body = Some(br#"{"id":"LOC1"}"#.to_vec());

    let results = forwarder.broadcast(&request, Role::Cpo).await;
    assert_eq!(results.len(), 2, "both eMSPs, and not the CPO that sent it");
    assert_eq!(aggregate(&results, AggregatePolicy::FirstErrorWins), StatusCode::SUCCESS);
    assert!(sender.seen().is_empty(), "a broadcast is not echoed back to its sender");

    for (party, expected_to) in [(&a, "DE/ABC"), (&b, "FR/XYZ")] {
        let seen = party.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].to.as_deref(), Some(expected_to), "TO is the receiving party");
        assert_eq!(seen[0].from.as_deref(), Some("NL/HUB"), "FROM is the hub, not the original sender");
        assert_eq!(
            seen[0].path, "/locations/NL/TNM/LOC1",
            "for client-owned objects the URL keeps the original party, not the hub's",
        );
    }
    // Every hop gets its own request id; the correlation id is one exchange.
    let (first, second) = (&a.seen()[0], &b.seen()[0]);
    assert_ne!(first.request_id, second.request_id);
    assert_eq!(first.correlation_id, second.correlation_id);
}

#[tokio::test]
async fn an_open_routing_request_speaks_in_the_requesting_partys_name() {
    // "Open request | Hub to receiving platform | Receiving-party | Requesting-party"
    let party = downstream().await;
    let table = RoutingTable::new();
    table.upsert(platform("msp", &msp(), Role::Emsp, &party.base));

    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let mut request = forwardable(Method::PUT, None, InterfaceRole::Receiver);
    request.path = "DE/ABC/LOC1".to_owned();
    request.body = Some(br#"{"country_code":"DE","party_id":"ABC","id":"LOC1"}"#.to_vec());

    assert_eq!(request.scenario(&hub_party()).expect("a scenario"), RoutingScenario::OpenRoutingRequest);
    let relayed = forwarder.open_route(&request, &BodyOwnerRouter).await.expect("a destination");
    assert!(relayed.is_success(), "{:?}", relayed.outcome);

    let seen = party.seen();
    assert_eq!(seen[0].to.as_deref(), Some("DE/ABC"));
    assert_eq!(seen[0].from.as_deref(), Some("NL/TNM"), "the hub does not substitute itself here");
}

#[tokio::test]
async fn an_open_routing_request_the_hub_cannot_place_is_a_4001() {
    struct Undecided;
    impl OpenRouter for Undecided {
        fn destination(&self, _request: &Forwardable) -> Option<PartyRef> {
            None
        }
    }

    let table = RoutingTable::new();
    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let request = forwardable(Method::PUT, None, InterfaceRole::Receiver);
    let error = forwarder.open_route(&request, &Undecided).await.expect_err("no destination");
    assert_eq!(error.status_code(), StatusCode::UNKNOWN_RECEIVER);
}

#[tokio::test]
async fn a_get_all_asks_every_source_in_the_requesters_name_and_merges_the_answers() {
    let a = downstream().await;
    let b = downstream().await;
    let table = RoutingTable::new();
    table.upsert(platform("cpo-a", &cpo(), Role::Cpo, &a.base));
    table.upsert(platform("cpo-b", &other_msp(), Role::Cpo, &b.base));

    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let mut request = Forwardable {
        query: Some("limit=10".to_owned()),
        ..forwardable(Method::GET, Some(hub_party()), InterfaceRole::Sender)
    };
    request.routing.from = msp();

    assert!(matches!(
        request.scenario(&hub_party()).expect("a scenario"),
        RoutingScenario::GetAllViaHub { .. }
    ));

    let results = forwarder.get_all(&request).await;
    assert_eq!(results.len(), 2, "every connected CPO is a source");
    assert!(results.iter().all(ocpi_kit::hub::Relayed::is_success));

    for party in [&a, &b] {
        let seen = party.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "GET");
        assert_eq!(seen[0].query.as_deref(), Some("limit=10"), "the query survives the hop");
        assert_eq!(
            seen[0].from.as_deref(),
            Some("DE/ABC"),
            "the answering party can see who actually asked, and authorise accordingly",
        );
    }
    assert_eq!(a.seen()[0].to.as_deref(), Some("NL/TNM"));
    assert_eq!(b.seen()[0].to.as_deref(), Some("FR/XYZ"));
}

#[tokio::test]
async fn a_request_for_a_party_the_hub_does_not_know_is_a_4001() {
    let table = RoutingTable::new();
    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let request = forwardable(Method::PUT, Some(msp()), InterfaceRole::Receiver);
    let relayed = forwarder.relay(&request, &msp(), RoutingHeaders::new(cpo(), msp())).await;
    let error = relayed.outcome.expect_err("nobody to relay to");
    assert_eq!(error.status_code(), StatusCode::UNKNOWN_RECEIVER);
}

#[tokio::test]
async fn a_party_that_does_not_implement_the_interface_is_a_4001_not_a_dropped_message() {
    let party = downstream().await;
    let table = RoutingTable::new();
    // Connected, but only for the Sender interface.
    table.upsert(ConnectedPlatform {
        platform_id: "msp".to_owned(),
        parties: vec![(msp(), Role::Emsp)],
        status: ConnectionStatus::Connected,
        peer: Peer::builder(VersionNumber::V2_3_0, test_token("c"))
            .versions_url(party.base.join("versions"))
            .endpoint(ModuleId::Locations, InterfaceRole::Sender, party.base.join("locations"))
            .party(msp())
            .build(),
    });

    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let request = forwardable(Method::PUT, Some(msp()), InterfaceRole::Receiver);
    let relayed = forwarder.relay(&request, &msp(), RoutingHeaders::new(cpo(), msp())).await;
    let error = relayed.outcome.expect_err("the Receiver interface is not implemented");
    assert_eq!(error.status_code(), StatusCode::UNKNOWN_RECEIVER);
    assert!(error.to_string().to_ascii_uppercase().contains("RECEIVER"), "{error}");
    assert!(party.seen().is_empty(), "nothing was sent");
}

#[tokio::test]
async fn an_unreachable_party_becomes_a_connection_problem_rather_than_a_transport_error() {
    let table = RoutingTable::new();
    // A port nothing is listening on.
    let dead = Url::new("http://127.0.0.1:1").expect("valid URL");
    table.upsert(platform("msp", &msp(), Role::Emsp, &dead));

    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let request = forwardable(Method::PUT, Some(msp()), InterfaceRole::Receiver);
    let relayed = forwarder.relay(&request, &msp(), RoutingHeaders::new(cpo(), msp())).await;
    let error = relayed.outcome.expect_err("nothing is listening");
    assert!(
        matches!(
            error.status_code(),
            StatusCode::CONNECTION_PROBLEM | StatusCode::TIMEOUT_ON_FORWARDED_REQUEST
        ),
        "{error}",
    );
}

#[tokio::test]
async fn one_failing_party_in_a_broadcast_surfaces_its_own_code_to_the_sender() {
    let good = downstream().await;
    let dead = Url::new("http://127.0.0.1:1").expect("valid URL");
    let table = RoutingTable::new();
    table.upsert(platform("msp-a", &msp(), Role::Emsp, &good.base));
    table.upsert(platform("msp-b", &other_msp(), Role::Emsp, &dead));

    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let request = forwardable(Method::PUT, Some(hub_party()), InterfaceRole::Receiver);
    let results = forwarder.broadcast(&request, Role::Cpo).await;

    assert_eq!(results.len(), 2);
    assert_eq!(results.iter().filter(|r| r.is_success()).count(), 1);
    assert_ne!(aggregate(&results, AggregatePolicy::FirstErrorWins), StatusCode::SUCCESS);
    // …but a hub that owns delivery can say so, and one party is enough for "at least delivered".
    assert_eq!(aggregate(&results, AggregatePolicy::AnySuccess), StatusCode::SUCCESS);
    assert_eq!(aggregate(&results, AggregatePolicy::AlwaysSucceed), StatusCode::SUCCESS);
    assert_eq!(good.seen().len(), 1, "the reachable party was still delivered to");
}

#[tokio::test]
async fn a_suspended_platform_is_not_broadcast_to() {
    let party = downstream().await;
    let table = RoutingTable::new();
    table.upsert(platform("msp", &msp(), Role::Emsp, &party.base));
    assert!(table.set_status("msp", ConnectionStatus::Suspended));

    let client = client();
    let forwarder = Forwarder::new(client.transport(), &table, hub_party());
    let request = forwardable(Method::PUT, Some(hub_party()), InterfaceRole::Receiver);
    let results = forwarder.broadcast(&request, Role::Cpo).await;

    assert!(results.is_empty(), "a suspended platform is not a target");
    assert_eq!(
        aggregate(&results, AggregatePolicy::AlwaysSucceed),
        StatusCode::CONNECTION_PROBLEM,
        "a broadcast that reached nobody is not a success, whatever the policy says",
    );
    assert!(party.seen().is_empty());
}

#[tokio::test]
async fn a_get_addressed_to_the_hub_on_a_receiver_interface_is_refused_not_broadcast() {
    // "GET SHALL NOT be used in combination with Broadcast Push."
    let request = forwardable(Method::GET, Some(hub_party()), InterfaceRole::Receiver);
    let error = request.scenario(&hub_party()).expect_err("not a scenario the spec defines");
    assert!(matches!(error, OcpiError::NotRoutable(_)), "{error:?}");
    assert_eq!(error.status_code(), StatusCode::INVALID_PARAMETERS);
}
