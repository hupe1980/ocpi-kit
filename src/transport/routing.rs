//! Message routing: the four `OCPI-*` headers and the scenarios that decide what goes in them.
//!
//! The spec gives five tables of who goes in `to` and who goes in `from`, one per scenario, and
//! getting them wrong is the classic hub bug. [`RoutingScenario`] encodes all five, so an
//! integrator picks a scenario rather than filling headers by hand.
//!
//! Spec: 2.3.0 §transport_and_format_message_routing

use core::fmt;

use http::{HeaderMap, HeaderValue};

use crate::ModuleId;
use crate::types::PartyRef;

use super::headers::{
    OCPI_FROM_COUNTRY_CODE, OCPI_FROM_PARTY_ID, OCPI_TO_COUNTRY_CODE, OCPI_TO_PARTY_ID, header_party,
};

/// The `OCPI-to-*` and `OCPI-from-*` headers of one message.
///
/// > *When implementing OCPI these four headers SHALL be implemented for any request/response
/// > to/from a Functional Module. This does not mean they have to be present in all request.*
///
/// `to` is `None` for an [Open Routing Request](RoutingScenario::OpenRoutingRequest), where the
/// hub decides the destination from the content.
///
/// Spec: 2.3.0 §transport_and_format_message_routing
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingHeaders {
    /// The party this message is to be sent to, absent for an Open Routing Request.
    pub to: Option<PartyRef>,
    /// The party this message is sent from.
    pub from: PartyRef,
}

impl RoutingHeaders {
    /// Headers addressed from `from` to `to`.
    #[must_use]
    pub fn new(from: PartyRef, to: PartyRef) -> Self {
        Self { to: Some(to), from }
    }

    /// Headers with no `to`, for an Open Routing Request.
    ///
    /// > *For an Open Routing Request, the TO headers in the request from the requesting party to
    /// > the Hub MUST be omitted.*
    #[must_use]
    pub fn open(from: PartyRef) -> Self {
        Self { to: None, from }
    }

    /// The headers of the response to this request, which swap `to` and `from`.
    ///
    /// For an Open Routing Request the responder is the receiving party, which the requester did
    /// not know; pass it as `responder`.
    #[must_use]
    pub fn response_from(&self, responder: PartyRef) -> Self {
        Self { to: Some(self.from.clone()), from: responder }
    }

    /// Reads the routing headers from a header map.
    ///
    /// Returns `None` when the `from` pair is absent, which is either a spec violation or a
    /// configuration module — see [`RoutingHeaders::applies_to`].
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        Some(Self {
            to: header_party(headers, &OCPI_TO_COUNTRY_CODE, &OCPI_TO_PARTY_ID),
            from: header_party(headers, &OCPI_FROM_COUNTRY_CODE, &OCPI_FROM_PARTY_ID)?,
        })
    }

    /// Writes the routing headers into a header map.
    pub fn write_to(&self, headers: &mut HeaderMap) {
        if let Some(to) = &self.to {
            insert_party(headers, &OCPI_TO_COUNTRY_CODE, &OCPI_TO_PARTY_ID, to);
        } else {
            headers.remove(OCPI_TO_COUNTRY_CODE);
            headers.remove(OCPI_TO_PARTY_ID);
        }
        insert_party(headers, &OCPI_FROM_COUNTRY_CODE, &OCPI_FROM_PARTY_ID, &self.from);
    }

    /// Whether these headers belong on a request to `module`.
    ///
    /// > *The requests/responses to/from Configuration Modules: Credentials, Versions and Hub
    /// > Client Info are not to be routed, and are for Platform-to-Platform or Platform-to-Hub
    /// > communication. Thus routing headers SHALL NOT be used with these modules.*
    #[must_use]
    pub fn applies_to(module: &ModuleId) -> bool {
        module.is_functional()
    }
}

fn insert_party(
    headers: &mut HeaderMap,
    country_header: &http::HeaderName,
    party_header: &http::HeaderName,
    party: &PartyRef,
) {
    if let Ok(v) = HeaderValue::from_str(party.country_code.as_str()) {
        headers.insert(country_header.clone(), v);
    }
    if let Ok(v) = HeaderValue::from_str(party.party_id.as_str()) {
        headers.insert(party_header.clone(), v);
    }
}

impl fmt::Display for RoutingHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.to {
            Some(to) => write!(f, "{} -> {to}", self.from),
            None => write!(f, "{} -> (open)", self.from),
        }
    }
}

/// One of the five routing arrangements the specification tabulates.
///
/// Each variant knows how to fill the headers for both the request and the response, in both
/// directions across a hub. This is the whole of §transport_and_format_message_routing's
/// "Overview of required/optional routing headers for different scenarios" in one type.
///
/// ```
/// use ocpi_kit::transport::RoutingScenario;
/// use ocpi_kit::types::PartyRef;
///
/// let cpo = PartyRef::new("NL", "TNM").unwrap();
/// let msp = PartyRef::new("DE", "ABC").unwrap();
/// let hub = PartyRef::new("NL", "HUB").unwrap();
///
/// // A broadcast push addresses the hub, not the eventual receivers.
/// let scenario = RoutingScenario::BroadcastPush { hub: hub.clone() };
/// let request = scenario.request_headers(&cpo, None);
/// assert_eq!(request.to, Some(hub));
/// assert_eq!(request.from, cpo);
/// ```
///
/// Spec: 2.3.0 §transport_and_format_message_routing
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RoutingScenario {
    /// Requesting platform to receiving platform, directly or through a hub that just relays.
    ///
    /// | | TO | FROM |
    /// |---|---|---|
    /// | request | receiving party | requesting party |
    /// | response | requesting party | receiving party |
    Direct,

    /// A push to every connected party with the opposite role, fanned out by the hub.
    ///
    /// | | TO | FROM |
    /// |---|---|---|
    /// | requester → hub | **hub** | requesting party |
    /// | hub → requester | requesting party | **hub** |
    /// | hub → receiver | receiving party | **hub** |
    /// | receiver → hub | **hub** | receiving party |
    ///
    /// > *GET SHALL NOT be used in combination with Broadcast Push.*
    BroadcastPush {
        /// The hub that fans the message out.
        hub: PartyRef,
    },

    /// The requester does not know the destination; the hub decides from the content.
    ///
    /// | | TO | FROM |
    /// |---|---|---|
    /// | requester → hub | *omitted* | requesting party |
    /// | hub → receiver | receiving party | requesting party |
    /// | receiver → hub | requesting party | receiving party |
    /// | hub → requester | requesting party | receiving party |
    ///
    /// > *Open Routing Requests are possible for GET (Not GET ALL), POST, PUT, PATCH and DELETE.*
    OpenRoutingRequest,

    /// A GET on a Sender interface implemented by the hub, merging several parties' objects.
    ///
    /// | | TO | FROM |
    /// |---|---|---|
    /// | requester → hub | **hub** | requesting party |
    /// | hub → requester | requesting party | **hub** |
    GetAllViaHub {
        /// The hub that merges the objects.
        hub: PartyRef,
    },
}

impl RoutingScenario {
    /// The headers of the request the requesting party sends.
    ///
    /// `receiver` is the destination party; it is ignored by the scenarios that address the hub
    /// or omit the `to` headers entirely, and may be `None` there.
    #[must_use]
    pub fn request_headers(&self, requester: &PartyRef, receiver: Option<&PartyRef>) -> RoutingHeaders {
        match self {
            Self::Direct => RoutingHeaders { to: receiver.cloned(), from: requester.clone() },
            Self::BroadcastPush { hub } | Self::GetAllViaHub { hub } => {
                RoutingHeaders { to: Some(hub.clone()), from: requester.clone() }
            }
            Self::OpenRoutingRequest => RoutingHeaders::open(requester.clone()),
        }
    }

    /// The headers of the response the requesting party will receive.
    ///
    /// `receiver` is the party that actually answered, which the hub knows even when the
    /// requester did not.
    #[must_use]
    pub fn response_headers(&self, requester: &PartyRef, receiver: Option<&PartyRef>) -> RoutingHeaders {
        match self {
            Self::Direct | Self::OpenRoutingRequest => RoutingHeaders {
                to: Some(requester.clone()),
                from: receiver.cloned().unwrap_or_else(|| requester.clone()),
            },
            Self::BroadcastPush { hub } | Self::GetAllViaHub { hub } => {
                RoutingHeaders { to: Some(requester.clone()), from: hub.clone() }
            }
        }
    }

    /// The headers the hub puts on the request it forwards to the receiving party.
    ///
    /// Returns `None` for [`GetAllViaHub`](Self::GetAllViaHub), where the hub answers from its
    /// own merged view and forwards nothing verbatim.
    #[must_use]
    pub fn forwarded_request_headers(
        &self,
        requester: &PartyRef,
        receiver: &PartyRef,
    ) -> Option<RoutingHeaders> {
        match self {
            // "Direct request | Hub to receiving platform | Receiving-party | Requesting-party"
            Self::Direct | Self::OpenRoutingRequest => {
                Some(RoutingHeaders::new(requester.clone(), receiver.clone()))
            }
            // "Broadcast request | Hub to receiving platform | Receiving-party | Hub"
            Self::BroadcastPush { hub } => Some(RoutingHeaders::new(hub.clone(), receiver.clone())),
            Self::GetAllViaHub { .. } => None,
        }
    }

    /// Whether a GET may use this scenario.
    ///
    /// > *GET SHALL NOT be used in combination with Broadcast Push. If the requesting party wants
    /// > to GET information of which it does not know the receiving party, an Open Routing
    /// > Request MUST be used.*
    #[must_use]
    pub const fn allows_get(&self) -> bool {
        !matches!(self, Self::BroadcastPush { .. })
    }

    /// Whether a write (POST, PUT, PATCH, DELETE) may use this scenario.
    ///
    /// A GET All is by definition a read.
    #[must_use]
    pub const fn allows_write(&self) -> bool {
        !matches!(self, Self::GetAllViaHub { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpo() -> PartyRef {
        PartyRef::new("NL", "TNM").unwrap()
    }
    fn msp() -> PartyRef {
        PartyRef::new("DE", "ABC").unwrap()
    }
    fn hub() -> PartyRef {
        PartyRef::new("NL", "HUB").unwrap()
    }

    #[test]
    fn direct_request_and_response_swap_the_parties() {
        let s = RoutingScenario::Direct;
        let req = s.request_headers(&cpo(), Some(&msp()));
        assert_eq!(req.to, Some(msp()));
        assert_eq!(req.from, cpo());
        let resp = s.response_headers(&cpo(), Some(&msp()));
        assert_eq!(resp.to, Some(cpo()));
        assert_eq!(resp.from, msp());
    }

    #[test]
    fn broadcast_push_addresses_the_hub_then_the_hub_speaks_for_itself() {
        let s = RoutingScenario::BroadcastPush { hub: hub() };
        // "Broadcast request | Requesting platform to Hub | Hub | Requesting-party"
        let req = s.request_headers(&cpo(), None);
        assert_eq!((req.to, req.from), (Some(hub()), cpo()));
        // "Broadcast response | Hub to requesting platform | Requesting-party | Hub"
        let resp = s.response_headers(&cpo(), None);
        assert_eq!((resp.to, resp.from), (Some(cpo()), hub()));
        // "Broadcast request | Hub to receiving platform | Receiving-party | Hub"
        let fwd = s.forwarded_request_headers(&cpo(), &msp()).unwrap();
        assert_eq!((fwd.to, fwd.from), (Some(msp()), hub()));
        assert!(!s.allows_get(), "GET SHALL NOT be used with Broadcast Push");
    }

    #[test]
    fn open_routing_omits_the_to_headers_only_on_the_first_hop() {
        let s = RoutingScenario::OpenRoutingRequest;
        let req = s.request_headers(&cpo(), None);
        assert_eq!(req.to, None, "the TO headers MUST be omitted");
        // "Open request | Hub to receiving platform | Receiving-party | Requesting-party"
        let fwd = s.forwarded_request_headers(&cpo(), &msp()).unwrap();
        assert_eq!((fwd.to, fwd.from), (Some(msp()), cpo()));
        assert!(s.allows_get() && s.allows_write());
    }

    #[test]
    fn get_all_via_hub_is_answered_by_the_hub_itself() {
        let s = RoutingScenario::GetAllViaHub { hub: hub() };
        let req = s.request_headers(&msp(), None);
        assert_eq!((req.to, req.from), (Some(hub()), msp()));
        let resp = s.response_headers(&msp(), None);
        assert_eq!((resp.to, resp.from), (Some(msp()), hub()));
        assert_eq!(s.forwarded_request_headers(&msp(), &cpo()), None);
        assert!(!s.allows_write(), "a GET All is a read");
    }

    #[test]
    fn configuration_modules_are_never_routed() {
        assert!(!RoutingHeaders::applies_to(&ModuleId::Credentials));
        assert!(!RoutingHeaders::applies_to(&ModuleId::Versions));
        assert!(!RoutingHeaders::applies_to(&ModuleId::HubClientInfo));
        assert!(RoutingHeaders::applies_to(&ModuleId::Locations));
    }

    #[test]
    fn headers_round_trip_and_an_open_request_has_no_to() {
        let mut headers = HeaderMap::new();
        RoutingHeaders::new(cpo(), msp()).write_to(&mut headers);
        assert_eq!(RoutingHeaders::from_headers(&headers), Some(RoutingHeaders::new(cpo(), msp())));

        RoutingHeaders::open(cpo()).write_to(&mut headers);
        let parsed = RoutingHeaders::from_headers(&headers).unwrap();
        assert_eq!(parsed.to, None, "writing an open request clears any previous TO headers");
        assert_eq!(parsed.from, cpo());
    }
}
