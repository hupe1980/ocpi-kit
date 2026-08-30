//! The modules that were server-side traits with no route and client-side URLs with no caller:
//! Charging Profiles, Payments, Hub Client Info, and the asynchronous half of Commands.
//!
//! What this file is really testing is *agreement*. A router mount and a URL builder are two
//! independent statements about the same path, written in different files; unit tests on either
//! side pass happily while the two disagree. Every test here drives this crate's client against
//! this crate's server over a real socket, so a path that only one side believes in fails.
//!
//! The three Charging Profiles result callbacks get particular attention: their bodies are
//! indistinguishable from one another, so the only thing that routes them is the URL, and
//! `CallbackUrls` is the one place that knows how to write it.

#![cfg(all(feature = "client", feature = "server", feature = "testkit"))]

use std::sync::Arc;
use std::sync::Mutex;

use ocpi_kit::client::{ClientConfig, OcpiClient, Peer};
use ocpi_kit::server::{
    AuthenticatedPeer, CallbackUrls, ChargingProfilesReceiver, ChargingProfilesSender, CommandsSender,
    Created, Handled, HubClientInfoReceiver, HubClientInfoSender, InMemoryTokenStore, OcpiRouter,
    PaymentsReceiver, PaymentsSender, RequestContext,
};
use ocpi_kit::testkit::{test_cpo, test_msp, test_token};
use ocpi_kit::transport::{OcpiError, Page, PageQuery, Patch, StatusCode};
use ocpi_kit::types::{DateTime, PartyRef, Url};
use ocpi_kit::v2_3_0::charging_profiles::{
    ActiveChargingProfile, ActiveChargingProfileResult, ChargingProfile, ChargingProfilePeriod,
    ChargingProfileResponse, ChargingProfileResponseType, ChargingProfileResult, ChargingRateUnit,
    ClearProfileResult, SetChargingProfile,
};
use ocpi_kit::v2_3_0::commands::{CommandResult, CommandResultType};
use ocpi_kit::v2_3_0::hub_client_info::{ClientInfo, ConnectionStatus};
use ocpi_kit::v2_3_0::payments::{CaptureStatusCode, FinancialAdviceConfirmation, Terminal};
use ocpi_kit::v2_3_0::types::{Price, Role};
use ocpi_kit::{InterfaceRole, ModuleId, VersionNumber};

// ---------------------------------------------------------------------------------------------
// A party that records what it was asked, so the test can assert the request arrived intact.
// ---------------------------------------------------------------------------------------------

/// Everything the handlers saw, in order.
#[derive(Debug, Default)]
struct Journal(Mutex<Vec<String>>);

impl Journal {
    fn record(&self, entry: impl Into<String>) {
        self.0.lock().expect("journal lock").push(entry.into());
    }

    fn entries(&self) -> Vec<String> {
        self.0.lock().expect("journal lock").clone()
    }
}

#[derive(Clone)]
struct Party {
    journal: Arc<Journal>,
    terminals: Arc<Mutex<Vec<Terminal>>>,
}

impl Party {
    fn new() -> Self {
        Self { journal: Arc::new(Journal::default()), terminals: Arc::new(Mutex::new(Vec::new())) }
    }

    fn terminal(&self, id: &str) -> Terminal {
        self.terminals
            .lock()
            .expect("terminals lock")
            .iter()
            .find(|t| t.terminal_id.as_str() == id)
            .cloned()
            .unwrap_or_else(|| sample_terminal(id))
    }
}

fn timestamp() -> DateTime {
    "2024-01-15T10:00:00Z".parse().expect("a valid timestamp")
}

fn sample_terminal(id: &str) -> Terminal {
    Terminal::builder().terminal_id(id).last_updated(timestamp()).build()
}

fn sample_confirmation(id: &str) -> FinancialAdviceConfirmation {
    FinancialAdviceConfirmation::builder()
        .id(id)
        .authorization_reference("AUTH-1")
        .total_costs(Price::new("12.50".parse().expect("a valid number")))
        .currency("EUR")
        .eft_data(vec!["**** 1234".into()])
        .capture_status_code(CaptureStatusCode::Success)
        .last_updated(timestamp())
        .build()
}

fn accepted() -> ChargingProfileResponse {
    ChargingProfileResponse::builder().result(ChargingProfileResponseType::Accepted).timeout(30u32).build()
}

fn sample_profile() -> ActiveChargingProfile {
    ActiveChargingProfile::builder()
        .start_date_time(timestamp())
        .charging_profile(
            ChargingProfile::builder()
                .charging_rate_unit(ChargingRateUnit::Watts)
                .charging_profile_period(vec![ChargingProfilePeriod {
                    start_period: 0,
                    limit: "11000".parse().expect("a valid number"),
                    extensions: ocpi_kit::types::Extensions::default(),
                }])
                .build(),
        )
        .build()
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

impl ChargingProfilesReceiver for Party {
    async fn active_charging_profile(
        &self,
        session_id: String,
        duration_seconds: u64,
        response_url: Url,
        _context: RequestContext,
    ) -> Handled<ChargingProfileResponse> {
        self.journal.record(format!("GET active {session_id} {duration_seconds}s -> {response_url}"));
        Ok(accepted())
    }

    async fn set_charging_profile(
        &self,
        session_id: String,
        request: SetChargingProfile,
        _context: RequestContext,
    ) -> Handled<ChargingProfileResponse> {
        self.journal.record(format!("PUT profile {session_id} -> {}", request.response_url));
        Ok(accepted())
    }

    async fn clear_charging_profile(
        &self,
        session_id: String,
        response_url: Url,
        _context: RequestContext,
    ) -> Handled<ChargingProfileResponse> {
        self.journal.record(format!("DELETE profile {session_id} -> {response_url}"));
        Ok(accepted())
    }
}

impl ChargingProfilesSender for Party {
    async fn active_charging_profile_result(
        &self,
        unique_id: String,
        result: ActiveChargingProfileResult,
        _context: RequestContext,
    ) -> Handled<()> {
        self.journal.record(format!("active result {unique_id} {:?}", result.result));
        Ok(())
    }

    async fn charging_profile_result(
        &self,
        unique_id: String,
        result: ChargingProfileResult,
        _context: RequestContext,
    ) -> Handled<()> {
        self.journal.record(format!("set result {unique_id} {:?}", result.result));
        Ok(())
    }

    async fn clear_profile_result(
        &self,
        unique_id: String,
        result: ClearProfileResult,
        _context: RequestContext,
    ) -> Handled<()> {
        self.journal.record(format!("clear result {unique_id} {:?}", result.result));
        Ok(())
    }

    async fn put_active_charging_profile(
        &self,
        session_id: String,
        _profile: ActiveChargingProfile,
        _context: RequestContext,
    ) -> Handled<()> {
        self.journal.record(format!("pushed active profile for {session_id}"));
        Ok(())
    }
}

impl CommandsSender for Party {
    async fn command_result(
        &self,
        unique_id: String,
        result: CommandResult,
        _context: RequestContext,
    ) -> Handled<()> {
        self.journal.record(format!("command result {unique_id} {:?}", result.result));
        Ok(())
    }
}

impl PaymentsSender for Party {
    async fn terminals(&self, _q: PageQuery, _c: RequestContext) -> Handled<Page<Terminal>> {
        Ok(Page::single(self.terminals.lock().expect("terminals lock").clone()))
    }

    async fn terminal(&self, terminal_id: String, _c: RequestContext) -> Handled<Terminal> {
        Ok(self.terminal(&terminal_id))
    }

    async fn put_terminal(
        &self,
        terminal_id: String,
        terminal: Terminal,
        _c: RequestContext,
    ) -> Handled<Terminal> {
        self.journal.record(format!("put terminal {terminal_id}"));
        Ok(terminal)
    }

    async fn patch_terminal(
        &self,
        terminal_id: String,
        patch: Patch<Terminal>,
        _c: RequestContext,
    ) -> Handled<Terminal> {
        self.journal.record(format!("patch terminal {terminal_id} {:?}", patch.fields()));
        patch.apply(&self.terminal(&terminal_id))
    }

    async fn activate_terminal(&self, terminal: Patch<Terminal>, _c: RequestContext) -> Handled<Terminal> {
        // "The terminal_id is optional in the activation request as it will be set by the PTP."
        self.journal.record(format!("activate {:?}", terminal.fields()));
        let mut created = sample_terminal("TERM-NEW");
        created.reference = terminal
            .as_value()
            .get("reference")
            .and_then(serde_json::Value::as_str)
            .map(ocpi_kit::types::CiString::new_lenient);
        self.terminals.lock().expect("terminals lock").push(created.clone());
        Ok(created)
    }

    async fn deactivate_terminal(&self, terminal_id: String, _c: RequestContext) -> Handled<Terminal> {
        self.journal.record(format!("deactivate {terminal_id}"));
        Ok(self.terminal(&terminal_id))
    }

    async fn financial_advice_confirmations(
        &self,
        _q: PageQuery,
        _c: RequestContext,
    ) -> Handled<Page<FinancialAdviceConfirmation>> {
        Ok(Page::single(vec![sample_confirmation("FAC1")]))
    }

    async fn financial_advice_confirmation(
        &self,
        id: String,
        _c: RequestContext,
    ) -> Handled<FinancialAdviceConfirmation> {
        Ok(sample_confirmation(&id))
    }
}

impl PaymentsReceiver for Party {
    async fn terminal(&self, terminal_id: String, _c: RequestContext) -> Handled<Terminal> {
        self.journal.record(format!("receiver GET terminal {terminal_id}"));
        Ok(sample_terminal(&terminal_id))
    }

    async fn post_terminal(&self, terminal: Terminal, _c: RequestContext) -> Handled<Terminal> {
        self.journal.record(format!("receiver POST terminal {}", terminal.terminal_id));
        Ok(terminal)
    }

    async fn financial_advice_confirmation(
        &self,
        id: String,
        _c: RequestContext,
    ) -> Handled<FinancialAdviceConfirmation> {
        Ok(sample_confirmation(&id))
    }

    async fn post_financial_advice_confirmation(
        &self,
        confirmation: FinancialAdviceConfirmation,
        _c: RequestContext,
    ) -> Handled<FinancialAdviceConfirmation> {
        self.journal.record(format!("receiver POST fac {}", confirmation.id));
        Ok(confirmation)
    }
}

impl HubClientInfoSender for Party {
    async fn list(&self, _q: PageQuery, _c: RequestContext) -> Handled<Page<ClientInfo>> {
        Ok(Page::single(vec![client_info(&test_cpo(), ConnectionStatus::Connected)]))
    }
}

impl HubClientInfoReceiver for Party {
    async fn client_info(&self, party: PartyRef, _c: RequestContext) -> Handled<ClientInfo> {
        self.journal.record(format!("client info for {party}"));
        Ok(client_info(&party, ConnectionStatus::Offline))
    }

    async fn put_client_info(
        &self,
        party: PartyRef,
        info: ClientInfo,
        _c: RequestContext,
    ) -> Handled<Created> {
        self.journal.record(format!("put client info {party} {:?}", info.status));
        Ok(Created::Yes)
    }
}

fn client_info(party: &PartyRef, status: ConnectionStatus) -> ClientInfo {
    ClientInfo::new(party.clone(), Role::Cpo, status, timestamp())
}

// ---------------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------------

struct Running {
    base: Url,
    party: Party,
    _handle: tokio::task::JoinHandle<()>,
}

/// Starts a party that mounts every interface added here, on an ephemeral port.
async fn start() -> Running {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("can bind");
    let port = listener.local_addr().expect("has an address").port();
    let base = Url::new(format!("http://127.0.0.1:{port}")).expect("valid URL");

    let tokens = Arc::new(InMemoryTokenStore::new());
    tokens.insert(
        test_token("c"),
        AuthenticatedPeer {
            peer_id: "peer".to_owned(),
            role: ocpi_kit::transport::TokenRole::C,
            parties: vec![test_msp()],
            version: VersionNumber::V2_3_0,
        },
    );

    let party = Party::new();
    let app = OcpiRouter::new(VersionNumber::V2_3_0, base.clone(), tokens)
        .charging_profiles_receiver(party.clone())
        .charging_profiles_sender(party.clone())
        .commands_sender(party.clone())
        .payments_sender(party.clone())
        .payments_receiver(party.clone())
        .hub_client_info_sender(party.clone())
        .hub_client_info_receiver(party.clone())
        .build();

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Running { base, party, _handle: handle }
}

fn client() -> OcpiClient {
    OcpiClient::with_config(ClientConfig::for_testing()).expect("can build a client")
}

/// A peer whose endpoints are exactly where the router mounted them.
fn peer_at(base: &Url) -> Peer {
    let receiver = |module: ModuleId, path: &str| (module, base.join("receiver").join(path));
    let mut builder = Peer::builder(VersionNumber::V2_3_0, test_token("c"))
        .versions_url(base.join("versions"))
        .party(test_cpo());
    for (module, url) in [
        (ModuleId::ChargingProfiles, base.join("chargingprofiles")),
        (ModuleId::Payments, base.join("payments")),
        (ModuleId::HubClientInfo, base.join("hubclientinfo")),
        (ModuleId::Commands, base.join("commands")),
    ] {
        builder = builder.endpoint(module, InterfaceRole::Sender, url);
    }
    for (module, url) in [
        receiver(ModuleId::ChargingProfiles, "chargingprofiles"),
        receiver(ModuleId::Payments, "payments"),
        receiver(ModuleId::HubClientInfo, "hubclientinfo"),
    ] {
        builder = builder.endpoint(module, InterfaceRole::Receiver, url);
    }
    builder.build()
}

// ---------------------------------------------------------------------------------------------
// Charging Profiles
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_three_charging_profile_verbs_reach_their_handlers() {
    let server = start().await;
    let client = client();
    let peer = peer_at(&server.base);
    let profiles = peer.charging_profiles(client.transport(), test_msp());
    let callbacks = CallbackUrls::new(server.base.clone());

    let response = profiles
        .active_charging_profile("SESSION-1", 900, &callbacks.active_charging_profile_result("req-1"))
        .await
        .expect("the GET reaches the handler");
    assert_eq!(response.result, ChargingProfileResponseType::Accepted);
    assert!(response.expects_result(), "an ACCEPTED answer promises a callback");

    profiles
        .set_charging_profile(
            "SESSION-1",
            &SetChargingProfile::builder()
                .charging_profile(sample_profile().charging_profile)
                .response_url(callbacks.charging_profile_result("req-2"))
                .build(),
        )
        .await
        .expect("the PUT reaches the handler");

    profiles
        .clear_charging_profile("SESSION-1", &callbacks.clear_profile_result("req-3"))
        .await
        .expect("the DELETE reaches the handler");

    let seen = server.party.journal.entries();
    assert_eq!(seen.len(), 3, "{seen:?}");
    // The duration and both response URLs survived the round trip through the query string.
    assert!(seen[0].contains("SESSION-1 900s"), "{}", seen[0]);
    assert!(seen[0].ends_with("/chargingprofiles/result/active/req-1"), "{}", seen[0]);
    assert!(seen[1].ends_with("/chargingprofiles/result/set/req-2"), "{}", seen[1]);
    assert!(seen[2].ends_with("/chargingprofiles/result/clear/req-3"), "{}", seen[2]);
}

#[tokio::test]
async fn a_get_without_a_response_url_is_a_2001_rather_than_a_silent_default() {
    // The parameter is required, and the Charge Point's answer would otherwise go nowhere.
    let server = start().await;
    let body = reqwest::Client::new()
        .get(server.base.join("receiver/chargingprofiles/SESSION-1").as_str())
        .header("Authorization", test_token("c").to_header_value())
        .send()
        .await
        .expect("the request completes")
        .json::<serde_json::Value>()
        .await
        .expect("an OCPI envelope");
    assert_eq!(body["status_code"], StatusCode::INVALID_PARAMETERS.get());
    assert!(body["status_message"].as_str().expect("a message").contains("response_url"), "{body}");
}

#[tokio::test]
async fn each_result_kind_is_routed_by_its_url_because_the_bodies_cannot_tell_them_apart() {
    // `ChargingProfileResult` and `ClearProfileResult` are the same JSON object: `{"result": …}`.
    // Nothing but the URL can distinguish a rejected PUT from a rejected DELETE, which is why
    // `CallbackUrls` exists and why there are three mounts rather than one.
    let server = start().await;
    let callbacks = CallbackUrls::new(server.base.clone());
    let http = reqwest::Client::new();
    let identical = serde_json::json!({ "result": "REJECTED" });

    for url in [
        callbacks.active_charging_profile_result("r1"),
        callbacks.charging_profile_result("r1"),
        callbacks.clear_profile_result("r1"),
    ] {
        let body = if url.as_str().contains("/active/") {
            serde_json::json!({ "result": "ACCEPTED", "profile": sample_profile() })
        } else {
            identical.clone()
        };
        let status = http
            .post(url.as_str())
            .header("Authorization", test_token("c").to_header_value())
            .json(&body)
            .send()
            .await
            .expect("the request completes")
            .json::<serde_json::Value>()
            .await
            .expect("an OCPI envelope");
        assert_eq!(status["status_code"], StatusCode::SUCCESS.get(), "{url}: {status}");
    }

    let seen = server.party.journal.entries();
    assert_eq!(
        seen,
        vec!["active result r1 Accepted", "set result r1 Rejected", "clear result r1 Rejected"],
        "three identical-looking bodies reached three different handlers",
    );
}

#[tokio::test]
async fn a_cpo_can_volunteer_a_changed_active_profile() {
    let server = start().await;
    let client = client();
    let peer = peer_at(&server.base);
    peer.charging_profiles(client.transport(), test_msp())
        .push_active_charging_profile("SESSION-9", &sample_profile())
        .await
        .expect("the PUT on the Sender interface reaches the handler");
    assert_eq!(server.party.journal.entries(), vec!["pushed active profile for SESSION-9"]);
}

// ---------------------------------------------------------------------------------------------
// Commands: the asynchronous half
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_command_result_reaches_the_sender_at_the_url_it_published() {
    // The spec's own example shape: `.../commands/RESERVE_NOW/1234`.
    let server = start().await;
    let callbacks = CallbackUrls::new(server.base.clone());
    let url = callbacks.command_result("RESERVE_NOW", "1234");
    assert!(url.as_str().ends_with("/commands/RESERVE_NOW/1234"), "{url}");

    let envelope = reqwest::Client::new()
        .post(url.as_str())
        .header("Authorization", test_token("c").to_header_value())
        .json(&serde_json::json!({ "result": "ACCEPTED" }))
        .send()
        .await
        .expect("the request completes")
        .json::<serde_json::Value>()
        .await
        .expect("an OCPI envelope");
    assert_eq!(envelope["status_code"], StatusCode::SUCCESS.get());
    assert!(envelope.get("data").is_none(), "the spec leaves `data` unset here");
    assert_eq!(
        server.party.journal.entries(),
        vec![format!("command result 1234 {:?}", CommandResultType::Accepted)]
    );
}

// ---------------------------------------------------------------------------------------------
// Payments
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_payments_sender_interface_round_trips_through_one_discovered_endpoint() {
    // The module has one ModuleID and two endpoint URLs; the client derives both sub-paths from
    // the single discovered `payments` endpoint, and the router mounts them there.
    let server = start().await;
    let client = client();
    let peer = peer_at(&server.base);
    let payments = peer.payments(client.transport(), test_msp());

    let created = payments
        .activate_terminal(&Patch::<Terminal>::from_value(serde_json::json!({ "reference": "SN-42" })))
        .await
        .expect("activation reaches the handler");
    assert_eq!(created.terminal_id.as_str(), "TERM-NEW");
    assert_eq!(created.reference.as_ref().expect("a reference").as_str(), "SN-42");

    let fetched = payments.terminal("TERM-NEW").await.expect("the terminal can be fetched");
    assert_eq!(fetched.terminal_id.as_str(), "TERM-NEW");

    let mut listed = payments.list_terminals(PageQuery::new()).expect("the peer implements it");
    let mut ids = Vec::new();
    while let Some(terminal) = listed.next().await.expect("the crawl succeeds") {
        ids.push(terminal.terminal_id.as_str().to_owned());
    }
    assert_eq!(ids, vec!["TERM-NEW"]);

    let patched = payments
        .patch_terminal(
            "TERM-NEW",
            &Patch::<Terminal>::from_value(serde_json::json!({
                "location_ids": ["LOC1", "LOC2"],
                "last_updated": "2024-02-01T09:00:00Z",
            })),
        )
        .await
        .expect("the PATCH reaches the handler");
    assert_eq!(patched.location_ids.len(), 2, "the assignment was applied");

    payments.deactivate_terminal("TERM-NEW").await.expect("deactivation reaches the handler");

    let confirmation =
        payments.financial_advice_confirmation("FAC1").await.expect("the confirmation is fetched");
    assert!(confirmation.is_fully_captured());

    let mut confirmations =
        payments.list_financial_advice_confirmations(PageQuery::new()).expect("the peer implements it");
    assert!(confirmations.next().await.expect("the crawl succeeds").is_some());

    let seen = server.party.journal.entries();
    assert!(seen.iter().any(|e| e.starts_with("activate")), "{seen:?}");
    assert!(seen.iter().any(|e| e.starts_with("patch terminal TERM-NEW")), "{seen:?}");
    assert!(seen.contains(&"deactivate TERM-NEW".to_owned()), "{seen:?}");
}

#[tokio::test]
async fn activate_is_a_static_segment_that_a_terminal_id_cannot_shadow() {
    // `/payments/terminals/activate` and `/payments/terminals/{terminal_id}` overlap; the static
    // segment has to win, or activation silently becomes a GET of a terminal called "activate".
    let server = start().await;
    let client = client();
    let peer = peer_at(&server.base);
    let payments = peer.payments(client.transport(), test_msp());

    payments
        .activate_terminal(&Patch::<Terminal>::from_value(serde_json::json!({})))
        .await
        .expect("activation is routed to the activate handler");
    assert!(
        server.party.journal.entries().iter().any(|e| e.starts_with("activate")),
        "the request reached `activate_terminal`, not `terminal`",
    );
}

#[tokio::test]
async fn the_payments_receiver_interface_uses_post_not_put() {
    // The one Receiver interface in OCPI whose objects are created with POST and whose URLs
    // carry no owning party.
    let server = start().await;
    let client = client();
    let peer = peer_at(&server.base);
    let payments = peer.payments(client.transport(), test_msp());

    let echoed = payments
        .post_terminal_to_receiver(&sample_terminal("TERM-7"))
        .await
        .expect("the POST reaches the CPO's handler");
    assert_eq!(echoed.terminal_id.as_str(), "TERM-7");

    let echoed = payments
        .post_financial_advice_confirmation(&sample_confirmation("FAC-7"))
        .await
        .expect("the POST reaches the CPO's handler");
    assert_eq!(echoed.id.as_str(), "FAC-7");

    let seen = server.party.journal.entries();
    assert!(seen.contains(&"receiver POST terminal TERM-7".to_owned()), "{seen:?}");
    assert!(seen.contains(&"receiver POST fac FAC-7".to_owned()), "{seen:?}");
}

// ---------------------------------------------------------------------------------------------
// Hub Client Info
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn hub_client_info_round_trips_without_routing_headers() {
    // A configuration module: "routing headers SHALL NOT be used with these modules."
    let server = start().await;
    let client = client();
    let peer = peer_at(&server.base);
    let hub_info = peer.hub_client_info(client.transport(), test_msp());

    let mut listed = hub_info.list(PageQuery::new()).expect("the peer implements the Sender interface");
    let first = listed.next().await.expect("the crawl succeeds").expect("one entry");
    assert_eq!(first.status, ConnectionStatus::Connected);

    let one = hub_info.client_info(&test_cpo()).await.expect("the GET reaches the handler");
    assert_eq!(one.status, ConnectionStatus::Offline);

    hub_info
        .put_client_info(&client_info(&test_cpo(), ConnectionStatus::Suspended))
        .await
        .expect("the PUT reaches the handler");

    let seen = server.party.journal.entries();
    assert!(seen.iter().any(|e| e.starts_with("client info for")), "{seen:?}");
    assert!(seen.iter().any(|e| e.contains("Suspended")), "{seen:?}");
}

#[tokio::test]
async fn client_info_is_about_a_party_the_caller_does_not_have_to_own() {
    // A hub tells each client about every *other* party, so the ownership rule that guards the
    // other client-owned-object URLs must not apply here. The authenticated peer speaks for the
    // eMSP; the object is about the CPO.
    let server = start().await;
    let client = client();
    let peer = peer_at(&server.base);
    let other = PartyRef::new("DE", "XYZ").expect("a valid party");

    let info = peer
        .hub_client_info(client.transport(), test_msp())
        .client_info(&other)
        .await
        .expect("a 404 here would make the module unusable for its actual purpose");
    assert_eq!(info.party_id.as_str(), "XYZ");
}

// ---------------------------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_version_details_advertise_every_newly_mounted_interface() {
    let server = start().await;
    let details = reqwest::Client::new()
        .get(server.base.as_str())
        .header("Authorization", test_token("c").to_header_value())
        .send()
        .await
        .expect("the request completes")
        .json::<serde_json::Value>()
        .await
        .expect("an OCPI envelope");

    let advertised: Vec<(String, String)> = details["data"]["endpoints"]
        .as_array()
        .expect("an endpoint list")
        .iter()
        .map(|e| {
            (
                e["identifier"].as_str().expect("an identifier").to_owned(),
                e["role"].as_str().expect("a role").to_owned(),
            )
        })
        .collect();

    for expected in [
        ("chargingprofiles", "RECEIVER"),
        ("chargingprofiles", "SENDER"),
        ("commands", "SENDER"),
        ("payments", "SENDER"),
        ("payments", "RECEIVER"),
        ("hubclientinfo", "SENDER"),
        ("hubclientinfo", "RECEIVER"),
    ] {
        assert!(
            advertised.iter().any(|(id, role)| id == expected.0 && role == expected.1),
            "{expected:?} is mounted but not advertised; discovery would disagree with reality: \
             {advertised:?}",
        );
    }
}

#[tokio::test]
async fn a_missing_interface_is_reported_before_a_request_is_made() {
    // A peer that never advertised the module: the client says so rather than inventing a URL.
    let server = start().await;
    let client = client();
    let bare = Peer::builder(VersionNumber::V2_3_0, test_token("c"))
        .versions_url(server.base.join("versions"))
        .party(test_cpo())
        .build();
    let error = bare
        .payments(client.transport(), test_msp())
        .terminal("TERM-1")
        .await
        .expect_err("the peer does not implement Payments");
    assert!(matches!(error, OcpiError::NotFound(_)), "{error:?}");
}

// ---------------------------------------------------------------------------------------------
// Router configuration
// ---------------------------------------------------------------------------------------------

/// Both interfaces of an ambiguously-shaped module on one router, with no prefix to separate
/// them, is refused at start-up — in **either** mount order.
///
/// The Charging Profiles Sender and Receiver interfaces are both keyed by `{session_id}`, and
/// Payments' by `terminals/{terminal_id}`. No route ordering can tell those apart, so the
/// alternative to this panic is a router that silently misroutes in production.
#[test]
fn mounting_both_interfaces_with_no_prefix_is_refused_in_either_order() {
    use ocpi_kit::server::ServerConfig;

    fn router() -> OcpiRouter {
        OcpiRouter::new(
            VersionNumber::V2_3_0,
            Url::new("https://e.com/ocpi/2.3.0").expect("valid URL"),
            Arc::new(InMemoryTokenStore::new()),
        )
        .with_config(ServerConfig::default().one_router_per_role())
    }

    let receiver_first = std::panic::catch_unwind(|| {
        router().charging_profiles_receiver(Party::new()).charging_profiles_sender(Party::new())
    });
    assert!(receiver_first.is_err(), "receiver then sender");

    let sender_first =
        std::panic::catch_unwind(|| router().payments_sender(Party::new()).payments_receiver(Party::new()));
    assert!(sender_first.is_err(), "sender then receiver");

    // With the default prefix, the Receiver interfaces sit one segment deeper and both fit.
    let both = std::panic::catch_unwind(|| {
        OcpiRouter::new(
            VersionNumber::V2_3_0,
            Url::new("https://e.com/ocpi/2.3.0").expect("valid URL"),
            Arc::new(InMemoryTokenStore::new()),
        )
        .charging_profiles_receiver(Party::new())
        .charging_profiles_sender(Party::new())
        .payments_sender(Party::new())
        .payments_receiver(Party::new())
        .build()
    });
    assert!(both.is_ok(), "the default receiver_path_prefix separates them");
}
