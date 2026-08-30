//! The client against peers that behave badly.
//!
//! `tests/end_to_end.rs` drives this crate's client against this crate's server, which proves the
//! two agree. It cannot prove the client survives a peer that *disagrees* — and every real
//! roaming partner disagrees somewhere. These tests build peers out of `wiremock` that are wrong
//! in one specific, documented way each, and assert what the client does about it.

#![cfg(all(feature = "client", feature = "testkit"))]

use ocpi_kit::client::{ClientConfig, OcpiClient, Peer, Registration};
use ocpi_kit::testkit::{sample, test_cpo, test_msp, test_token};
use ocpi_kit::transport::{OcpiError, PageQuery, Quirks, StatusCode};
use ocpi_kit::types::{Url, Validate};
use ocpi_kit::v2_3_0::locations::Location;
use ocpi_kit::v2_3_0::versions::{Endpoint, Version, VersionDetails};
use ocpi_kit::{InterfaceRole, ModuleId, VersionNumber};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client() -> OcpiClient {
    OcpiClient::with_config(ClientConfig::for_testing()).expect("can build a client")
}

fn base(server: &MockServer) -> Url {
    Url::new(server.uri()).expect("wiremock gives a valid URL")
}

fn envelope(data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "data": data,
        "status_code": 1000,
        "status_message": "Success",
        "timestamp": "2024-03-01T10:00:00Z",
    })
}

fn locations_peer(server: &MockServer) -> Peer {
    Peer::builder(VersionNumber::V2_3_0, test_token("c"))
        .versions_url(base(server).join("versions"))
        .endpoint(ModuleId::Locations, InterfaceRole::Sender, base(server).join("locations"))
        .party(test_msp())
        .build()
}

/// Mounts `/versions` and `/2.3.0` on a mock peer.
async fn mount_discovery(server: &MockServer) {
    let versions = vec![Version::new(VersionNumber::V2_3_0, base(server).join("2.3.0"))];
    Mock::given(method("GET"))
        .and(path("/versions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(envelope(serde_json::to_value(&versions).expect("serialises"))),
        )
        .mount(server)
        .await;

    let details = VersionDetails::new(
        VersionNumber::V2_3_0,
        vec![
            Endpoint::new(ModuleId::Credentials, InterfaceRole::Sender, base(server).join("credentials")),
            Endpoint::new(ModuleId::Locations, InterfaceRole::Sender, base(server).join("locations")),
        ],
    );
    Mock::given(method("GET"))
        .and(path("/2.3.0"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(envelope(serde_json::to_value(&details).expect("serialises"))),
        )
        .mount(server)
        .await;
}

// ---------------------------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn discovery_reads_the_versions_and_the_details_of_the_newest_common_one() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let selected = Registration::new(base(&server).join("versions"), test_token("c"))
        .discover(client().transport())
        .await
        .expect("discovery succeeds")
        .select_best(client().transport())
        .await
        .expect("a common version exists");

    assert_eq!(selected.version(), &VersionNumber::V2_3_0);
    assert!(selected.details().credentials_url().is_some());
    selected
        .require(&[(ModuleId::Locations, InterfaceRole::Sender)])
        .expect("the peer advertises Locations/SENDER");
}

#[tokio::test]
async fn a_peer_that_offers_no_version_we_speak_is_refused_before_anything_is_sent() {
    let server = MockServer::start().await;
    // A peer that only speaks a version this build does not model.
    let versions = serde_json::json!([{ "version": "3.0", "url": format!("{}/3.0", server.uri()) }]);
    Mock::given(method("GET"))
        .and(path("/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(versions)))
        .mount(&server)
        .await;

    let discovered = Registration::new(base(&server).join("versions"), test_token("c"))
        .discover(client().transport())
        .await
        .expect("the version list still parses — an unknown version is not a decode error");

    assert!(discovered.best_common_version().is_none());
    assert!(discovered.select_best(client().transport()).await.is_err(), "there is nothing to select");
}

#[tokio::test]
async fn a_peer_missing_a_required_module_is_refused_before_credentials_are_posted() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;

    let selected = Registration::new(base(&server).join("versions"), test_token("c"))
        .discover(client().transport())
        .await
        .expect("discovery succeeds")
        .select_best(client().transport())
        .await
        .expect("a common version exists");

    // "In case the Sender cannot find the endpoints it expects, it is expected NOT to send the
    // POST request with credentials to the Receiver."
    let error = selected
        .require(&[(ModuleId::Cdrs, InterfaceRole::Sender)])
        .expect_err("this peer has no CDRs endpoint");
    match error {
        OcpiError::Remote { status_code, status_message } => {
            assert_eq!(status_code, StatusCode::NO_MATCHING_ENDPOINTS);
            assert!(
                status_message.as_deref().unwrap_or_default().contains("cdrs"),
                "the message should name what is missing: {status_message:?}"
            );
        }
        other => panic!("expected 3003, got {other}"),
    }

    // And nothing was sent to the credentials endpoint.
    assert!(
        !server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .any(|r| r.url.path().contains("credentials")),
        "require() must not touch the peer"
    );
}

// ---------------------------------------------------------------------------------------------
// The authorization header
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_token_is_sent_base64_encoded_by_default() {
    let server = MockServer::start().await;
    let expected = format!("Token {}", base64_of(test_token("c").expose_secret()));

    Mock::given(method("GET"))
        .and(path("/locations"))
        .and(header("authorization", expected.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(serde_json::json!([]))))
        .expect(1)
        .mount(&server)
        .await;

    let c = client();
    let peer = locations_peer(&server);
    let mut stream = peer
        .locations(c.transport(), test_msp())
        .list(PageQuery::new())
        .expect("the peer implements Locations/SENDER");
    assert!(stream.next().await.expect("the request succeeds").is_none());
}

#[tokio::test]
async fn a_peer_that_wants_an_unencoded_token_is_accommodated_by_a_quirk() {
    let server = MockServer::start().await;
    let expected = format!("Token {}", test_token("c").expose_secret());

    Mock::given(method("GET"))
        .and(path("/locations"))
        .and(header("authorization", expected.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(serde_json::json!([]))))
        .expect(1)
        .mount(&server)
        .await;

    let mut quirks = Quirks::default();
    quirks.send_unencoded_token = true;
    let c = client();
    let mut peer = locations_peer(&server);
    peer.set_quirks(quirks);

    let mut stream = peer
        .locations(c.transport(), test_msp())
        .list(PageQuery::new())
        .expect("the peer implements Locations/SENDER");
    assert!(stream.next().await.expect("the request succeeds").is_none());
}

fn base64_of(value: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(value)
}

// ---------------------------------------------------------------------------------------------
// Pagination against a peer that does not cooperate
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_crawl_follows_the_link_header_across_pages() {
    let server = MockServer::start().await;
    let page = |ids: &[&str]| {
        serde_json::to_value(
            ids.iter().map(|id| sample::location(id).expect("valid sample")).collect::<Vec<_>>(),
        )
        .expect("serialises")
    };

    Mock::given(method("GET"))
        .and(path("/locations"))
        .and(query_param("offset", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-Total-Count", "4")
                .insert_header("X-Limit", "2")
                .set_body_json(envelope(page(&["LOC3", "LOC4"]))),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/locations"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-Total-Count", "4")
                .insert_header("X-Limit", "2")
                .insert_header("Link", format!("<{}/locations?offset=2&limit=2>; rel=\"next\"", server.uri()))
                .set_body_json(envelope(page(&["LOC1", "LOC2"]))),
        )
        .mount(&server)
        .await;

    let c = client();
    let peer = locations_peer(&server);
    let mut stream = peer
        .locations(c.transport(), test_msp())
        .list(PageQuery::new())
        .expect("the peer implements Locations/SENDER");

    let mut ids = Vec::new();
    while let Some(location) = stream.next().await.expect("each page decodes") {
        ids.push(location.id.as_str().to_owned());
    }
    assert_eq!(ids, ["LOC1", "LOC2", "LOC3", "LOC4"], "the crawl follows rel=\"next\" to the end");
}

/// The conformance runner against a peer whose pagination filters do nothing.
///
/// Both failures are invisible in any single response: the objects are correct, the envelope is
/// correct, and a client only finds out when a crawl never terminates or a nightly incremental
/// pull takes six hours. They are exactly what a conformance run is for.
#[tokio::test]
async fn the_conformance_runner_catches_a_peer_that_ignores_offset_and_date_from() {
    use ocpi_kit::client::{Conformance, Outcome};

    let server = MockServer::start().await;
    mount_discovery(&server).await;

    // The same two objects for every query, whatever `offset` or `date_from` says.
    let page = serde_json::to_value(vec![
        sample::location("LOC1").expect("valid sample"),
        sample::location("LOC2").expect("valid sample"),
    ])
    .expect("serialises");
    Mock::given(method("GET"))
        .and(path("/locations"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-Total-Count", "2")
                .insert_header("X-Limit", "10")
                .set_body_json(envelope(page)),
        )
        .mount(&server)
        .await;

    let report =
        Conformance::new(base(&server).join("versions"), test_token("c")).run(client().transport()).await;

    let outcome = |id: &str| report.checks.iter().find(|c| c.id == id).map(|c| c.outcome);
    assert_eq!(outcome("module.offset"), Some(Outcome::Fail), "{report}");
    assert_eq!(outcome("module.date_from"), Some(Outcome::Fail), "{report}");
    assert!(
        report.failures().any(|c| c.detail.contains("never terminate")),
        "the offset finding should say what it costs:\n{report}"
    );
}

#[tokio::test]
async fn a_shrinking_result_set_rewinds_by_one_object_and_yields_no_duplicates() {
    // "While crawling over the pages one of these objects is updated. The client detects this:
    //  X-Total-Count will be lower in the next request. It is advised to redo the previous GET
    //  with the `offset` lowered by 1 (if the `offset` was not 0) and after that continue
    //  crawling the 'next' page links."
    //
    // The GET to redo is the one that saw the count drop, and its objects are discarded. Six
    // objects, pages of two: the crawl takes A and B, then asks for offset 2 and is told the set
    // is now five long — one object before its window is gone, so everything after it slid down
    // by one and the object at the new offset 1 would be skipped. Redoing the *just-attempted*
    // GET one lower picks it up. Rewinding to the previous page's offset instead would re-emit a
    // whole page the caller has already seen, which is the bug this pins down.
    let server = MockServer::start().await;
    let page = |ids: &[&str]| {
        serde_json::to_value(
            ids.iter().map(|id| sample::location(id).expect("valid sample")).collect::<Vec<_>>(),
        )
        .expect("serialises")
    };
    let next = |offset: u32| format!("<{}/locations?offset={offset}&limit=2>; rel=\"next\"", server.uri());

    let mount = async |offset: Option<&str>, total: &str, link: Option<String>, body| {
        let mut template =
            ResponseTemplate::new(200).insert_header("X-Total-Count", total).insert_header("X-Limit", "2");
        if let Some(link) = link {
            template = template.insert_header("Link", link);
        }
        let mock = Mock::given(method("GET")).and(path("/locations"));
        let mock = match offset {
            Some(offset) => mock.and(query_param("offset", offset)),
            None => mock,
        };
        mock.respond_with(template.set_body_json(envelope(body))).mount(&server).await;
    };

    // The rewind target, and the page after it. Mounted first: `wiremock` takes the first mock
    // that matches, and the catch-all below matches everything.
    mount(Some("1"), "5", Some(next(3)), page(&["LOC3", "LOC4"])).await;
    mount(Some("3"), "5", None, page(&["LOC5"])).await;
    // The page that notices the shrink. Its objects are the ones that would have skipped LOC3.
    mount(Some("2"), "5", Some(next(4)), page(&["LOC4", "LOC5"])).await;
    // The first page, before anything changed.
    mount(None, "6", Some(next(2)), page(&["LOC1", "LOC2"])).await;

    let c = client();
    let peer = locations_peer(&server);
    let mut stream = peer
        .locations(c.transport(), test_msp())
        .list(PageQuery::new())
        .expect("the peer implements Locations/SENDER");

    let mut ids = Vec::new();
    while let Some(location) = stream.next().await.expect("each page decodes") {
        ids.push(location.id.as_str().to_owned());
    }
    assert_eq!(stream.corrections(), 1, "the shrink was noticed exactly once");
    assert_eq!(ids, ["LOC1", "LOC2", "LOC3", "LOC4", "LOC5"], "nothing skipped, nothing repeated");
}

#[tokio::test]
async fn a_peer_that_never_stops_offering_a_next_page_is_cut_off() {
    let server = MockServer::start().await;
    // Every page points at itself: a peer with an off-by-one in its pagination, which is a real
    // and recurring interop failure. Without a cap the crawl never returns.
    Mock::given(method("GET"))
        .and(path("/locations"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-Total-Count", "1000000")
                .insert_header("Link", format!("<{}/locations?offset=0&limit=1>; rel=\"next\"", server.uri()))
                .set_body_json(envelope(
                    serde_json::to_value(vec![sample::location("LOC1").expect("valid sample")])
                        .expect("serialises"),
                )),
        )
        .mount(&server)
        .await;

    let c = client();
    let peer = locations_peer(&server);
    let mut stream = peer
        .locations(c.transport(), test_msp())
        .list(PageQuery::new())
        .expect("the peer implements Locations/SENDER");

    let mut seen = 0usize;
    loop {
        // A stream that ends and a stream that errors are both fine; running forever is not.
        match stream.next().await {
            Ok(Some(_)) => seen += 1,
            Ok(None) | Err(_) => break,
        }
        assert!(seen <= ocpi_kit::client::DEFAULT_MAX_PAGES + 1, "the crawl must be bounded");
    }
    assert!(seen > 0, "some objects were read before the cap");
}

// ---------------------------------------------------------------------------------------------
// Envelopes that are wrong
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_ocpi_error_in_a_200_body_is_surfaced_as_an_error() {
    let server = MockServer::start().await;
    // "2003 Unknown Location" carried in a 200, which is what the specification requires and
    // what a client that only looks at HTTP statuses gets wrong.
    Mock::given(method("GET"))
        .and(path("/locations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status_code": 2003,
            "status_message": "Unknown Location",
            "timestamp": "2024-03-01T10:00:00Z",
        })))
        .mount(&server)
        .await;

    let c = client();
    let peer = locations_peer(&server);
    let mut stream = peer
        .locations(c.transport(), test_msp())
        .list(PageQuery::new())
        .expect("the peer implements Locations/SENDER");

    match stream.next().await {
        Err(OcpiError::Remote { status_code, .. }) => {
            assert_eq!(status_code.get(), 2003, "a 2xxx in a 200 body is still an error");
        }
        other => panic!("expected the OCPI status to surface, got {other:?}"),
    }
}

#[tokio::test]
async fn a_non_conformant_object_still_decodes_and_is_reported_by_validation() {
    let server = MockServer::start().await;
    // A Location whose `country_code` is three characters, which the property table says is 2.
    // The page must still be readable: one bad object cannot cost the caller the whole page.
    let mut raw = serde_json::to_value(sample::location("LOC1").expect("valid sample")).expect("serialises");
    raw["country_code"] = serde_json::json!("BEL");
    Mock::given(method("GET"))
        .and(path("/locations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(serde_json::json!([raw]))))
        .mount(&server)
        .await;

    let c = client();
    let peer = locations_peer(&server);
    let mut stream = peer
        .locations(c.transport(), test_msp())
        .list(PageQuery::new())
        .expect("the peer implements Locations/SENDER");

    let location: Location = stream.next().await.expect("the page decodes").expect("one object");
    assert_eq!(location.country_code.as_str(), "BEL", "the value arrives intact");

    let violations = location.validate().expect_err("but it is not conformant");
    assert!(
        violations.iter().any(|v| v.pointer == "/country_code"),
        "the violation points at the field: {violations}"
    );
}

#[tokio::test]
async fn a_transient_failure_is_retried_for_a_get() {
    let server = MockServer::start().await;
    // The first attempt fails with a 503, the second succeeds. Only GET may be retried.
    Mock::given(method("GET"))
        .and(path("/locations"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/locations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(serde_json::json!([]))))
        .expect(1)
        .mount(&server)
        .await;

    let c = client();
    let peer = locations_peer(&server);
    let mut stream = peer
        .locations(c.transport(), test_msp())
        .list(PageQuery::new())
        .expect("the peer implements Locations/SENDER");
    assert!(stream.next().await.expect("the retry succeeds").is_none());
}

#[tokio::test]
async fn a_write_is_never_retried() {
    let server = MockServer::start().await;
    // "the client should not queue the message and retry the same message again later."
    Mock::given(method("PUT")).respond_with(ResponseTemplate::new(503)).expect(1).mount(&server).await;

    let c = client();
    let peer = Peer::builder(VersionNumber::V2_3_0, test_token("c"))
        .versions_url(base(&server).join("versions"))
        .endpoint(ModuleId::Locations, InterfaceRole::Receiver, base(&server).join("locations"))
        .party(test_msp())
        .build();

    let location = sample::location("LOC1").expect("valid sample");
    let result =
        peer.locations_receiver(c.transport(), test_msp()).put_location(&test_cpo(), &location).await;
    assert!(result.is_err(), "the 503 surfaces");
    // `expect(1)` on the mock asserts the single attempt when the server is dropped.
}
