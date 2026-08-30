//! Forwarding one request through the hub, and fanning one out.

use http::Method;

use crate::client::Transport;
use crate::transport::{
    OcpiError, OcpiRequest, OcpiResponse, RequestIds, RoutingHeaders, RoutingScenario, StatusCode,
};
use crate::types::{PartyRef, Url};
use crate::{InterfaceRole, ModuleId};

use super::routing_table::RoutingTable;

/// What the hub was asked to relay.
#[derive(Debug)]
pub struct Forwardable {
    /// The HTTP method of the incoming request.
    pub method: Method,
    /// The module it addresses.
    pub module: ModuleId,
    /// Which interface of that module.
    pub interface: InterfaceRole,
    /// The path below the module endpoint, e.g. `NL/TNM/LOC1`.
    pub path: String,
    /// The query string, without the leading `?`.
    pub query: Option<String>,
    /// The routing headers as they arrived.
    pub routing: RoutingHeaders,
    /// The IDs as they arrived. The hub renews the request ID and keeps the correlation ID.
    pub ids: RequestIds,
    /// The request body, if any.
    pub body: Option<Vec<u8>>,
}

impl Forwardable {
    /// The scenario this request is, from its headers and method.
    ///
    /// > *To send a Broadcast Push, the client uses the party-id and country-code of the Hub in
    /// > the 'OCPI-to-' headers.*
    /// >
    /// > *When … the requesting party does not know the destination of a request, the 'OCPI-to-'
    /// > headers can be omitted.*
    /// >
    /// > *To request a GET All from a Hub, the client uses the party-id and country-code of the
    /// > Hub in the 'OCPI-to-' headers, and calls the GET method on the Sender interface.*
    ///
    /// # Why this can fail
    ///
    /// Addressing the hub itself is the one ambiguous case: it means a Broadcast Push for a
    /// write and a GET All for a `GET` on a Sender interface, and the two remaining combinations
    /// are not scenarios at all.
    ///
    /// > *GET SHALL NOT be used in combination with Broadcast Push. If the requesting party wants
    /// > to GET information of which it does not know the receiving party, an Open Routing
    /// > Request MUST be used.*
    ///
    /// A `GET` on a Receiver interface addressed to the hub is therefore refused rather than
    /// quietly broadcast — the sender is told to omit the `OCPI-to-` headers instead — and so is
    /// a write addressed to the hub on a Sender interface, which is neither a push to the
    /// connected parties nor a read to merge.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::NotRoutable`], a `2001`, for those two combinations.
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing
    pub fn scenario(&self, hub: &PartyRef) -> Result<RoutingScenario, OcpiError> {
        match &self.routing.to {
            None => Ok(RoutingScenario::OpenRoutingRequest),
            Some(to) if to == hub => match (self.method == Method::GET, self.interface) {
                (true, InterfaceRole::Sender) => Ok(RoutingScenario::GetAllViaHub { hub: hub.clone() }),
                (false, InterfaceRole::Receiver) => Ok(RoutingScenario::BroadcastPush { hub: hub.clone() }),
                (true, InterfaceRole::Receiver) => Err(OcpiError::NotRoutable(
                    "a GET addressed to the hub on a Receiver interface is neither a GET All (which is \
                     a GET on a Sender interface) nor a Broadcast Push (which SHALL NOT be a \
                     GET); omit the OCPI-to- headers to make it an Open Routing Request"
                        .to_owned(),
                )),
                (false, InterfaceRole::Sender) => Err(OcpiError::NotRoutable(format!(
                    "a {} addressed to the hub on a Sender interface is not a scenario the \
                     specification defines; address the receiving party directly, or omit the \
                     OCPI-to- headers for an Open Routing Request",
                    self.method
                ))),
            },
            Some(_) => Ok(RoutingScenario::Direct),
        }
    }

    /// The URL this request becomes at `base`, the receiving platform's endpoint for the module.
    #[must_use]
    pub fn url_at(&self, base: &Url) -> Url {
        let url = if self.path.is_empty() { base.clone() } else { base.join(&self.path) };
        match &self.query {
            Some(query) if !query.is_empty() => url.with_query(query),
            _ => url,
        }
    }
}

/// The outcome of relaying one request to one party.
#[derive(Debug)]
pub struct Relayed {
    /// The party the request went to.
    pub party: PartyRef,
    /// What came back, or why nothing did.
    pub outcome: Result<OcpiResponse<serde_json::Value>, OcpiError>,
}

impl Relayed {
    /// Whether the party answered with a success status code.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.outcome.as_ref().is_ok_and(OcpiResponse::is_success)
    }
}

/// Relays requests to the platforms in a [`RoutingTable`].
#[derive(Debug)]
pub struct Forwarder<'a> {
    transport: &'a Transport,
    table: &'a RoutingTable,
    hub: PartyRef,
}

impl<'a> Forwarder<'a> {
    /// A forwarder for the hub party `hub`.
    #[must_use]
    pub fn new(transport: &'a Transport, table: &'a RoutingTable, hub: PartyRef) -> Self {
        Self { transport, table, hub }
    }

    /// The hub's own party reference.
    #[must_use]
    pub const fn hub(&self) -> &PartyRef {
        &self.hub
    }

    /// Relays one request to the party the `OCPI-to-*` headers name.
    ///
    /// The forwarded request gets a **new** `X-Request-ID` and the **same** `X-Correlation-ID`:
    ///
    /// > *When a Hub forwards a request to a party, the request to this party SHALL contain a new
    /// > unique value in the X-Request-ID HTTP header, not a copy … the request SHALL contain the
    /// > same X-Correlation-ID HTTP header.*
    ///
    /// # Errors
    ///
    /// Returns the `4xxx` code that fits: `4001` when the destination is unknown, `4003` when it
    /// is not connected, `4002` on a timeout, `4000` otherwise.
    ///
    /// Spec: 2.3.0 §transport_and_format_unique_messageg_ids, §status_codes_4xxx_hub_errors
    pub async fn relay(&self, request: &Forwardable, to: &PartyRef, routing: RoutingHeaders) -> Relayed {
        let target = self.table.with_platform(to, |platform| {
            platform
                .peer
                .endpoint_url(&request.module, request.interface)
                .cloned()
                .map(|base| (base, platform.peer.token().clone(), platform.peer.quirks().clone()))
        });

        let (base, token, quirks) = match target {
            Err(e) => return Relayed { party: to.clone(), outcome: Err(e) },
            Ok(None) => {
                return Relayed {
                    party: to.clone(),
                    outcome: Err(OcpiError::Remote {
                        status_code: StatusCode::UNKNOWN_RECEIVER,
                        status_message: Some(format!(
                            "{to} does not implement the {} interface of {}",
                            request.interface, request.module
                        )),
                    }),
                };
            }
            Ok(Some(target)) => target,
        };

        let mut outgoing =
            OcpiRequest::new(request.method.clone(), request.url_at(&base), request.module.clone())
                .routed(routing)
                .with_ids(request.ids.forwarded());
        outgoing.body = request.body.clone();

        let outcome = self
            .transport
            .send_with_headers::<serde_json::Value>(&outgoing, &token, &quirks)
            .await
            .map(|(response, _)| response)
            .map_err(map_hub_error);

        Relayed { party: to.clone(), outcome }
    }

    /// Fans a Broadcast Push out to every party with an opposite role.
    ///
    /// > *When using Broadcast Push, the Hub broadcasts received information to all connected
    /// > clients … This means only one request to the Hub will be necessary.*
    ///
    /// Every target is attempted; the results are returned in full so the caller decides what to
    /// tell the sender. [`aggregate`] implements the usual policy.
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing_broadcast_push
    pub async fn broadcast(
        &self,
        request: &Forwardable,
        sender_role: crate::v2_3_0::types::Role,
    ) -> Vec<Relayed> {
        let targets = self.table.broadcast_targets(&request.routing.from, sender_role, &request.module);
        let mut results = Vec::with_capacity(targets.len());
        for (_, party) in targets {
            // "Broadcast request | Hub to receiving platform | Receiving-party | Hub"
            let routing = RoutingHeaders::new(self.hub.clone(), party.clone());
            results.push(self.relay(request, &party, routing).await);
        }
        results
    }

    /// Answers an Open Routing Request by asking the router to pick a destination.
    ///
    /// > *When a Hub has the intelligence to route messages based on the content of the request …
    /// > The Hub can then decide to which party a request needs to be routed, or that it needs to
    /// > be broadcasted if the destination cannot be determined.*
    ///
    /// # Errors
    ///
    /// Returns `4001` when the router cannot decide.
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing_open_routing_request
    pub async fn open_route(
        &self,
        request: &Forwardable,
        router: &dyn OpenRouter,
    ) -> Result<Relayed, OcpiError> {
        let to = router.destination(request).ok_or_else(|| OcpiError::Remote {
            status_code: StatusCode::UNKNOWN_RECEIVER,
            status_message: Some("the hub could not determine a destination from the request".to_owned()),
        })?;
        // "Open request | Hub to receiving platform | Receiving-party | Requesting-party"
        let routing = RoutingHeaders::new(request.routing.from.clone(), to.clone());
        Ok(self.relay(request, &to, routing).await)
    }

    /// Answers a GET All by asking every party that implements the Sender interface.
    ///
    /// > *The Hub can then combine objects from different connected parties and return them to
    /// > the client. The client can determine the owner of the objects by looking at the
    /// > `country_code` and `party_id` in the individual objects returned by the hub.*
    ///
    /// Because ownership is carried inside each object, merging is a concatenation; the hub does
    /// not have to rewrite anything, and — importantly — *"the `last_updated` fields SHALL NOT be
    /// updated by the Hub"*.
    ///
    /// Spec: 2.3.0 §transport_and_format_get_all_via_hubs
    pub async fn get_all(&self, request: &Forwardable) -> Vec<Relayed> {
        let sources = self.table.get_all_sources(&request.routing.from, &request.module);
        let mut results = Vec::with_capacity(sources.len());
        for (_, party) in sources {
            // The GET All table covers only the two legs between the requester and the hub
            // ("Requesting platform to Hub | Hub | Requesting-party"); it says nothing about the
            // leg from the hub onward, because from the sending party's side that leg is an
            // ordinary request. So it takes the ordinary relay headers — "Direct request | Hub to
            // receiving platform | Receiving-party | Requesting-party" — which also means the
            // answering party can see who actually asked, and authorise accordingly.
            let routing = RoutingHeaders::new(request.routing.from.clone(), party.clone());
            results.push(self.relay(request, &party, routing).await);
        }
        results
    }
}

/// Decides where an Open Routing Request should go, from its content.
///
/// The specification leaves this entirely to the hub — *"When a Hub has the intelligence to route
/// messages based on the content of the request"* — so it is a trait. A typical implementation
/// looks at the `country_code`/`party_id` inside the body, or at the issuer of a token.
pub trait OpenRouter: Send + Sync {
    /// The party this request should be routed to, or `None` to give up with `4001`.
    fn destination(&self, request: &Forwardable) -> Option<PartyRef>;
}

/// An [`OpenRouter`] that reads `country_code` and `party_id` out of the request body.
///
/// This covers the common case: every client-owned object carries the party that owns it.
#[derive(Debug, Default)]
pub struct BodyOwnerRouter;

impl OpenRouter for BodyOwnerRouter {
    fn destination(&self, request: &Forwardable) -> Option<PartyRef> {
        let body = request.body.as_ref()?;
        let value: serde_json::Value = serde_json::from_slice(body).ok()?;
        let country = value.get("country_code")?.as_str()?;
        let party = value.get("party_id")?.as_str()?;
        PartyRef::new(country, party).ok()
    }
}

/// How a fan-out's results become one answer for the sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AggregatePolicy {
    /// Report the first failure. The safe default: the sender learns something went wrong.
    FirstErrorWins,
    /// Report success as long as one party accepted the message.
    AnySuccess,
    /// Always report success; the hub owns delivery from here.
    AlwaysSucceed,
}

/// Turns the results of a fan-out into the status code the sender is told.
///
/// A broadcast that reached nobody is `4003 Connection problem`; one where a party rejected the
/// object surfaces that party's own code, because the sender needs to see it.
///
/// Spec: 2.3.0 §status_codes_4xxx_hub_errors
#[must_use]
pub fn aggregate(results: &[Relayed], policy: AggregatePolicy) -> StatusCode {
    if results.is_empty() {
        return StatusCode::CONNECTION_PROBLEM;
    }
    let succeeded = results.iter().filter(|r| r.is_success()).count();
    match policy {
        AggregatePolicy::AlwaysSucceed => StatusCode::SUCCESS,
        AggregatePolicy::AnySuccess if succeeded > 0 => StatusCode::SUCCESS,
        _ => {
            if succeeded == results.len() {
                return StatusCode::SUCCESS;
            }
            results.iter().find(|r| !r.is_success()).map_or(StatusCode::HUB_ERROR, |failed| {
                match &failed.outcome {
                    Ok(response) => response.status_code,
                    Err(error) => error.status_code(),
                }
            })
        }
    }
}

/// Maps a transport failure onto the hub error code that describes it.
fn map_hub_error(error: OcpiError) -> OcpiError {
    match &error {
        OcpiError::Transport(message) => {
            let lower = message.to_ascii_lowercase();
            let code = if lower.contains("timeout") || lower.contains("timed out") {
                StatusCode::TIMEOUT_ON_FORWARDED_REQUEST
            } else {
                StatusCode::CONNECTION_PROBLEM
            };
            OcpiError::Remote { status_code: code, status_message: Some(message.clone()) }
        }
        _ => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DateTime;

    fn hub() -> PartyRef {
        PartyRef::new("NL", "HUB").unwrap()
    }

    fn request(method: Method, to: Option<PartyRef>, interface: InterfaceRole) -> Forwardable {
        Forwardable {
            method,
            module: ModuleId::Locations,
            interface,
            path: String::new(),
            query: None,
            routing: RoutingHeaders { to, from: PartyRef::new("NL", "TNM").unwrap() },
            ids: RequestIds::generate(),
            body: None,
        }
    }

    #[test]
    fn the_scenario_is_read_off_the_headers_and_method() {
        // "the client uses the party-id and country-code of the Hub in the 'OCPI-to-' headers"
        // plus a GET on a Sender interface is a GET All …
        assert!(matches!(
            request(Method::GET, Some(hub()), InterfaceRole::Sender).scenario(&hub()).unwrap(),
            RoutingScenario::GetAllViaHub { .. }
        ));
        // … and the same headers with a write is a Broadcast Push.
        assert!(matches!(
            request(Method::PUT, Some(hub()), InterfaceRole::Receiver).scenario(&hub()).unwrap(),
            RoutingScenario::BroadcastPush { .. }
        ));
        // No TO headers at all is an Open Routing Request.
        assert_eq!(
            request(Method::PUT, None, InterfaceRole::Receiver).scenario(&hub()).unwrap(),
            RoutingScenario::OpenRoutingRequest
        );
        // Anyone else in the TO headers is a plain relay.
        assert_eq!(
            request(Method::GET, Some(PartyRef::new("DE", "ABC").unwrap()), InterfaceRole::Sender)
                .scenario(&hub())
                .unwrap(),
            RoutingScenario::Direct
        );
    }

    #[test]
    fn a_get_addressed_to_the_hub_is_never_silently_broadcast() {
        // "GET SHALL NOT be used in combination with Broadcast Push." Classifying this as a
        // Broadcast Push would contradict `RoutingScenario::allows_get`, so it is refused with
        // the advice the spec itself gives.
        let error = request(Method::GET, Some(hub()), InterfaceRole::Receiver).scenario(&hub()).unwrap_err();
        assert_eq!(error.status_code(), StatusCode::INVALID_PARAMETERS);
        assert!(error.to_string().contains("Open Routing Request"), "{error}");

        // Nor is a write on a Sender interface a scenario at all.
        let error = request(Method::PUT, Some(hub()), InterfaceRole::Sender).scenario(&hub()).unwrap_err();
        assert_eq!(error.status_code(), StatusCode::INVALID_PARAMETERS);
    }

    #[test]
    fn every_scenario_agrees_with_what_it_says_it_allows() {
        // The classification and the method rules are two statements of the same spec text;
        // this pins them together.
        for (method, interface) in [
            (Method::GET, InterfaceRole::Sender),
            (Method::GET, InterfaceRole::Receiver),
            (Method::PUT, InterfaceRole::Sender),
            (Method::PUT, InterfaceRole::Receiver),
            (Method::POST, InterfaceRole::Receiver),
            (Method::DELETE, InterfaceRole::Receiver),
        ] {
            for to in [None, Some(hub()), Some(PartyRef::new("DE", "ABC").unwrap())] {
                let r = request(method.clone(), to, interface);
                let Ok(scenario) = r.scenario(&hub()) else { continue };
                if method == Method::GET {
                    assert!(scenario.allows_get(), "{scenario:?} classified a GET it forbids");
                } else {
                    assert!(scenario.allows_write(), "{scenario:?} classified a write it forbids");
                }
            }
        }
    }

    #[test]
    fn the_forwarded_url_keeps_the_path_and_query() {
        let mut r = request(Method::GET, Some(hub()), InterfaceRole::Sender);
        r.path = "NL/TNM/LOC1".to_owned();
        r.query = Some("offset=50&limit=10".to_owned());
        let base = Url::new("https://msp.example.com/ocpi/emsp/2.3.0/locations").unwrap();
        assert_eq!(
            r.url_at(&base).as_str(),
            "https://msp.example.com/ocpi/emsp/2.3.0/locations/NL/TNM/LOC1?offset=50&limit=10"
        );
    }

    #[test]
    fn the_body_router_reads_the_owner_out_of_the_object() {
        let mut r = request(Method::PUT, None, InterfaceRole::Receiver);
        r.body = Some(br#"{"country_code":"DE","party_id":"ABC","id":"LOC1"}"#.to_vec());
        assert_eq!(BodyOwnerRouter.destination(&r), Some(PartyRef::new("DE", "ABC").unwrap()));
        r.body = Some(br#"{"id":"LOC1"}"#.to_vec());
        assert_eq!(BodyOwnerRouter.destination(&r), None);
    }

    fn relayed(party: &str, status: StatusCode) -> Relayed {
        Relayed {
            party: PartyRef::new("DE", party).unwrap(),
            outcome: Ok(OcpiResponse {
                data: None,
                status_code: status,
                status_message: None,
                timestamp: DateTime::from_unix_timestamp(0).unwrap(),
            }),
        }
    }

    #[test]
    fn aggregation_surfaces_the_first_failure_by_default() {
        let results =
            vec![relayed("AAA", StatusCode::SUCCESS), relayed("BBB", StatusCode::INVALID_PARAMETERS)];
        assert_eq!(
            aggregate(&results, AggregatePolicy::FirstErrorWins),
            StatusCode::INVALID_PARAMETERS,
            "the sender needs to see the receiving party's own code"
        );
        assert_eq!(aggregate(&results, AggregatePolicy::AnySuccess), StatusCode::SUCCESS);
        assert_eq!(aggregate(&results, AggregatePolicy::AlwaysSucceed), StatusCode::SUCCESS);
    }

    #[test]
    fn a_broadcast_that_reached_nobody_is_a_connection_problem() {
        assert_eq!(aggregate(&[], AggregatePolicy::AlwaysSucceed), StatusCode::CONNECTION_PROBLEM);
    }

    #[test]
    fn a_timeout_becomes_4002_and_a_refused_connection_4003() {
        let timeout = map_hub_error(OcpiError::Transport("operation timed out".into()));
        assert_eq!(timeout.status_code(), StatusCode::TIMEOUT_ON_FORWARDED_REQUEST);
        let refused = map_hub_error(OcpiError::Transport("connection refused".into()));
        assert_eq!(refused.status_code(), StatusCode::CONNECTION_PROBLEM);
    }
}
