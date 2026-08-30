//! The reference peer, driven by this crate's own client and conformance runner.
//!
//! [`MockPeer`](ocpi_kit::testkit::MockPeer) is what `ocpi serve-mock` runs and what an
//! integrator points a half-written client at. That only means anything if it is *conformant*, so
//! this file holds it to the same standard the crate holds a partner to: the conformance runner
//! must find nothing, and every typed client call must round-trip against it.
//!
//! It is also the widest end-to-end path in the suite. `tests/end_to_end.rs` mounts two modules
//! by hand; this mounts all five object modules on both interfaces at once, which is the
//! arrangement where the Sender and Receiver URL shapes actually collide.

#![cfg(all(feature = "client", feature = "server", feature = "testkit"))]

use ocpi_kit::client::{ClientConfig, Conformance, OcpiClient, Outcome, Peer};
use ocpi_kit::server::OcpiRouter;
use ocpi_kit::testkit::{MockPeer, sample, test_cpo, test_msp, test_token};
use ocpi_kit::transport::{PageQuery, Patch};
use ocpi_kit::types::{DateTime, Url};
use ocpi_kit::{InterfaceRole, ModuleId, VersionNumber};

struct Running {
    base: Url,
    peer: MockPeer,
    _handle: tokio::task::JoinHandle<()>,
}

/// Starts a seeded mock CPO with `extra` further Locations, so pagination has something to do.
async fn start(version: VersionNumber, extra: usize) -> Running {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("can bind");
    let port = listener.local_addr().expect("has an address").port();
    let base = Url::new(format!("http://127.0.0.1:{port}")).expect("valid URL");

    let peer = MockPeer::cpo(base.clone()).seeded();
    for i in 0..extra {
        let mut location = sample::location(&format!("LOC{}", i + 2)).expect("valid sample");
        location.last_updated =
            DateTime::from_unix_timestamp(1_705_312_800 + i64::try_from(i).expect("small") * 60)
                .expect("in range");
        peer.locations.put(location);
    }
    let app = peer.mount(OcpiRouter::new(version, base.clone(), peer.token_store())).build();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Running { base, peer, _handle: handle }
}

fn client() -> OcpiClient {
    OcpiClient::with_config(ClientConfig::for_testing()).expect("can build a client")
}

/// A client peer pointed at the mock, with the endpoints discovery would have found.
fn peer_at(base: &Url, version: VersionNumber) -> Peer {
    let mut builder = Peer::builder(version, test_token("c"))
        .versions_url(base.join("versions"))
        .endpoint(ModuleId::Credentials, InterfaceRole::Sender, base.join("credentials"))
        .party(test_cpo());
    for module in
        [ModuleId::Locations, ModuleId::Sessions, ModuleId::Cdrs, ModuleId::Tariffs, ModuleId::Tokens]
    {
        builder = builder
            .endpoint(module.clone(), InterfaceRole::Sender, base.join(module.as_str()))
            .endpoint(module.clone(), InterfaceRole::Receiver, base.join("receiver").join(module.as_str()));
    }
    builder.build()
}

#[tokio::test]
async fn the_reference_peer_passes_this_crates_own_conformance_run() {
    // Two Locations at least, so `module.offset` has something to distinguish and does not skip.
    let server = start(VersionNumber::V2_3_0, 4).await;
    let report =
        Conformance::new(server.base.join("versions"), test_token("c")).run(client().transport()).await;

    assert!(!report.has_failures(), "the peer a partner integrates against must be clean:\n{report}");
    for id in ["module.offset", "module.date_from", "module.link", "module.objects"] {
        let check = report
            .checks
            .iter()
            .find(|c| c.id == id && c.outcome != Outcome::Skipped)
            .unwrap_or_else(|| panic!("{id} never ran against any module:\n{report}"));
        assert_eq!(check.outcome, Outcome::Pass, "{id}: {}", check.detail);
    }
}

#[tokio::test]
async fn every_seeded_module_answers_its_typed_client() {
    let server = start(VersionNumber::V2_3_0, 0).await;
    let c = client();
    let peer = peer_at(&server.base, VersionNumber::V2_3_0);
    let me = test_msp();

    let location = peer.locations(c.transport(), me.clone()).location("LOC1").await.expect("seeded");
    assert_eq!(location.id.as_str(), "LOC1");
    let evse = peer.locations(c.transport(), me.clone()).evse("LOC1", "3256").await.expect("seeded");
    assert_eq!(evse.uid.as_str(), "3256");
    peer.locations(c.transport(), me.clone()).connector("LOC1", "3256", "1").await.expect("seeded");

    let sessions =
        peer.sessions(c.transport(), me.clone()).list(PageQuery::new()).expect("mounted").collect_all();
    assert_eq!(sessions.await.expect("a page").len(), 1);
    let cdrs = peer.cdrs(c.transport(), me.clone()).list(PageQuery::new()).expect("mounted").collect_all();
    assert_eq!(cdrs.await.expect("a page").len(), 1);
    let tariffs =
        peer.tariffs(c.transport(), me.clone()).list(PageQuery::new()).expect("mounted").collect_all();
    assert_eq!(tariffs.await.expect("a page").len(), 1);

    // The real-time authorization, both answers the specification names.
    let allowed = peer
        .tokens(c.transport(), me.clone())
        .authorize("012345678", None, None)
        .await
        .expect("a known token");
    assert_eq!(allowed.allowed, ocpi_kit::v2_3_0::tokens::AllowedType::Allowed);
    let unknown =
        peer.tokens(c.transport(), me).authorize("not-a-token", None, None).await.expect_err("2004");
    assert_eq!(unknown.status_code(), ocpi_kit::transport::StatusCode::UNKNOWN_TOKEN);
}

#[tokio::test]
async fn a_partner_can_push_and_the_peer_keeps_it() {
    let server = start(VersionNumber::V2_3_0, 0).await;
    let c = client();
    let peer = peer_at(&server.base, VersionNumber::V2_3_0);
    let me = test_msp();

    let mut token = sample::token("push-1").expect("valid sample");
    token.country_code = me.country_code.clone();
    token.party_id = me.party_id.clone();
    peer.tokens_receiver(c.transport(), me.clone())
        .put_token(&me, &token)
        .await
        .expect("a partner may write under its own party");
    assert_eq!(server.peer.tokens.len(), 2, "the seeded one and the pushed one");

    // And the PATCH rule end to end: a patch carrying `last_updated` is applied to the stored
    // object, and one without it never reaches the handler.
    let patch: Patch<ocpi_kit::v2_3_0::tokens::Token> = Patch::from_value(serde_json::json!({
        "valid": false,
        "last_updated": "2024-06-01T00:00:00Z",
    }));
    peer.tokens_receiver(c.transport(), me.clone())
        .patch(&me, "push-1", None, &patch)
        .await
        .expect("a well-formed patch");
    assert!(!server.peer.tokens.get("push-1").expect("still there").valid);

    let bad: Patch<ocpi_kit::v2_3_0::tokens::Token> = Patch::from_value(serde_json::json!({ "valid": true }));
    peer.tokens_receiver(c.transport(), me)
        .patch(&test_msp(), "push-1", None, &bad)
        .await
        .expect_err("a PATCH without last_updated is a 2001");
    assert!(!server.peer.tokens.get("push-1").expect("unchanged").valid);
}

#[tokio::test]
async fn writing_under_a_party_that_is_not_yours_is_refused() {
    let server = start(VersionNumber::V2_3_0, 0).await;
    let c = client();
    let peer = peer_at(&server.base, VersionNumber::V2_3_0);

    // The mock's token store says this partner speaks for DE/ABC. Writing under NL/TNM is
    // "blocking client access to objects that do not belong to them", which is a 404.
    let token = sample::token("push-2").expect("valid sample");
    peer.tokens_receiver(c.transport(), test_msp())
        .put_token(&test_cpo(), &token)
        .await
        .expect_err("not this partner's party");
    assert_eq!(server.peer.tokens.len(), 1, "nothing was stored");
}

#[tokio::test]
async fn the_reference_peer_serves_2_2_1_from_the_same_handlers() {
    // The mock is written once, against the canonical model. A partner still on 2.2.1 gets 2.2.1.
    let server = start(VersionNumber::V2_2_1, 4).await;
    let report =
        Conformance::new(server.base.join("versions"), test_token("c")).run(client().transport()).await;
    assert!(!report.has_failures(), "a 2.2.1 mock must be as clean as a 2.3.0 one:\n{report}");
    assert_eq!(report.version.as_ref(), Some(&VersionNumber::V2_2_1));

    // And the bytes really are 2.2.1: a 2.2.1 Tariff has no `tax_included`.
    let body: serde_json::Value = reqwest::Client::new()
        .get(server.base.join("tariffs").as_str())
        .header("Authorization", test_token("c").to_header_value())
        .send()
        .await
        .expect("the mock answers")
        .json()
        .await
        .expect("an envelope");
    assert!(body["data"][0].get("tax_included").is_none(), "{body}");
}
