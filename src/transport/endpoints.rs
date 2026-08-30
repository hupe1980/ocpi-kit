//! Endpoint URL builders, one per documented URL shape.
//!
//! > *The URLs of the endpoints in this document are descriptive only. The exact URL can be found
//! > by fetching the endpoint information from the API info endpoint.*
//!
//! So every builder here takes the **discovered** base URL of a module and appends only the parts
//! the specification does define: the client-owned object path, the nested Location path, the
//! command name, and so on. Nothing here invents a base path.
//!
//! Spec: 2.3.0 §transport_and_format_interface_endpoints,
//! §transport_and_format_client_owned_object_push

use crate::types::{PartyRef, Url};

use super::pagination::PageQuery;

/// URLs on a module's **Sender** interface: the data owner's own objects.
///
/// A Sender interface is addressed by object id alone, because the owner is the party being
/// called.
///
/// Spec: 2.3.0 §mod_locations_cpo_interface and the equivalent section of each module
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SenderEndpoint {
    base: Url,
}

impl SenderEndpoint {
    /// Wraps a discovered Sender endpoint URL.
    #[must_use]
    pub fn new(base: Url) -> Self {
        Self { base }
    }

    /// The base URL as discovered.
    #[must_use]
    pub const fn base(&self) -> &Url {
        &self.base
    }

    /// `GET {base}?[date_from]&[date_to]&[offset]&[limit]` — the paginated list.
    #[must_use]
    pub fn list(&self, query: &PageQuery) -> Url {
        query.apply_to(&self.base)
    }

    /// `GET {base}/{id}` — one object.
    #[must_use]
    pub fn object(&self, id: &str) -> Url {
        self.base.join(id)
    }

    /// `GET {locations}/{location_id}[/{evse_uid}[/{connector_id}]]`.
    ///
    /// Spec: 2.3.0 §mod_locations_get_object_request_parameters
    #[must_use]
    pub fn location(&self, location_id: &str, evse_uid: Option<&str>, connector_id: Option<&str>) -> Url {
        let mut url = self.base.join(location_id);
        if let Some(evse) = evse_uid {
            url = url.join(evse);
            if let Some(connector) = connector_id {
                url = url.join(connector);
            }
        }
        url
    }

    /// `POST {tokens}/{token_uid}/authorize[?type={type}]` — real-time authorization.
    ///
    /// Spec: 2.3.0 §mod_tokens_real-time_authorization
    #[must_use]
    pub fn token_authorize(&self, token_uid: &str, token_type: Option<&str>) -> Url {
        let url = self.base.join(token_uid).join("authorize");
        match token_type {
            Some(t) => url.with_query(&format!("type={t}")),
            None => url,
        }
    }

    /// `PUT {sessions}/{session_id}/charging_preferences`.
    ///
    /// Spec: 2.3.0 §mod_sessions_set_charging_preferences
    #[must_use]
    pub fn charging_preferences(&self, session_id: &str) -> Url {
        self.base.join(session_id).join("charging_preferences")
    }

    /// `POST {commands}/{command}` — a command request on the Receiver's Sender-side endpoint.
    ///
    /// Spec: 2.3.0 §mod_commands_commands_module
    #[must_use]
    pub fn command(&self, command: &str) -> Url {
        self.base.join(command)
    }

    /// The Payments **terminals** sub-interface, `{payments}/terminals`.
    ///
    /// # Spec gap: one `ModuleID`, two endpoint URLs
    ///
    /// The Payments chapter declares *"Module Identifier: `payments`"* and then addresses its
    /// two interfaces through two different variables,
    /// `{payments_terminals_endpoint_url}` and
    /// `{payments_financial_advice_confirmation_endpoint_url}`. Version discovery cannot express
    /// that: an [`Endpoint`](crate::v2_3_0::versions::Endpoint) is keyed by `identifier` **and**
    /// `role`, so one module and one interface role has exactly one URL. A PTP that advertised
    /// both would have to publish two `payments`/`SENDER` endpoints, and a client reading them
    /// would have no way to tell which was which.
    ///
    /// The reading this crate takes — and which the specification's own examples support, since
    /// they are `…/payments/terminals/` and `…/payments/financial-advice-confirmations/` — is
    /// that the discovered `payments` endpoint is the **base** the two hang off. This has been
    /// reported upstream; until it is resolved, [`SenderEndpoint::payments_terminals`] and
    /// [`SenderEndpoint::payments_financial_advice_confirmations`] also **tolerate** a peer that
    /// advertised one of the sub-paths directly, so either reading interoperates.
    #[must_use]
    pub fn payments_terminals(&self) -> Self {
        Self::new(sub_path(&self.base, "terminals"))
    }

    /// The Payments **financial advice confirmations** sub-interface. See
    /// [`payments_terminals`](Self::payments_terminals) for why this is derived rather than
    /// discovered.
    #[must_use]
    pub fn payments_financial_advice_confirmations(&self) -> Self {
        Self::new(sub_path(&self.base, "financial-advice-confirmations"))
    }

    /// `{payments}/terminals/{terminal_id}` — one payment terminal.
    ///
    /// Call this on the endpoint from [`payments_terminals`](Self::payments_terminals).
    #[must_use]
    pub fn terminal(&self, terminal_id: &str) -> Url {
        self.base.join(terminal_id)
    }

    /// `POST {payments}/terminals/{terminal_id}/deactivate`.
    #[must_use]
    pub fn terminal_deactivate(&self, terminal_id: &str) -> Url {
        self.base.join(terminal_id).join("deactivate")
    }

    /// `POST {payments}/terminals/activate`.
    #[must_use]
    pub fn terminal_activate(&self) -> Url {
        self.base.join("activate")
    }
}

/// Appends `segment` to `base`, unless the peer already advertised it there.
///
/// The tolerance is deliberate: see [`SenderEndpoint::payments_terminals`].
fn sub_path(base: &Url, segment: &str) -> Url {
    if base.as_str().trim_end_matches('/').ends_with(segment) {
        return base.clone();
    }
    base.join(segment)
}

/// URLs on a module's **Receiver** interface: objects owned by the *client*.
///
/// > *Client Owned Object URL definition: `{base-ocpi-url}/{end-point}/{country-code}/{party-id}/
/// > {object-id}`*
/// >
/// > *POST is not supported for these kinds of modules. PUT is used to send new objects to the
/// > servers.*
///
/// Spec: 2.3.0 §transport_and_format_client_owned_object_push
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverEndpoint {
    base: Url,
}

impl ReceiverEndpoint {
    /// Wraps a discovered Receiver endpoint URL.
    #[must_use]
    pub fn new(base: Url) -> Self {
        Self { base }
    }

    /// The base URL as discovered.
    #[must_use]
    pub const fn base(&self) -> &Url {
        &self.base
    }

    /// `{base}/{country_code}/{party_id}/{object_id}`.
    #[must_use]
    pub fn object(&self, owner: &PartyRef, object_id: &str) -> Url {
        self.base.join(owner.country_code.as_str()).join(owner.party_id.as_str()).join(object_id)
    }

    /// `{locations}/{country_code}/{party_id}/{location_id}[/{evse_uid}[/{connector_id}]]`.
    ///
    /// Spec: 2.3.0 §mod_locations_request_parameters_msp
    #[must_use]
    pub fn location(
        &self,
        owner: &PartyRef,
        location_id: &str,
        evse_uid: Option<&str>,
        connector_id: Option<&str>,
    ) -> Url {
        let mut url = self.object(owner, location_id);
        if let Some(evse) = evse_uid {
            url = url.join(evse);
            if let Some(connector) = connector_id {
                url = url.join(connector);
            }
        }
        url
    }

    /// `{chargingprofiles}/{session_id}[?duration=&response_url=]`.
    ///
    /// The Charging Profiles Receiver interface is keyed by session, not by owning party.
    ///
    /// Spec: 2.3.0 §mod_charging_profiles_module
    #[must_use]
    pub fn charging_profile(&self, session_id: &str) -> Url {
        self.base.join(session_id)
    }

    /// `GET {chargingprofiles}/{session_id}?duration={duration}&response_url={response_url}`.
    #[must_use]
    pub fn active_charging_profile(
        &self,
        session_id: &str,
        duration_seconds: u64,
        response_url: &Url,
    ) -> Url {
        self.base.join(session_id).with_query(&format!(
            "duration={duration_seconds}&response_url={}",
            percent_encode(response_url.as_str())
        ))
    }

    /// `DELETE {chargingprofiles}/{session_id}?response_url={response_url}`.
    #[must_use]
    pub fn clear_charging_profile(&self, session_id: &str, response_url: &Url) -> Url {
        self.base
            .join(session_id)
            .with_query(&format!("response_url={}", percent_encode(response_url.as_str())))
    }

    /// `{tokens}/{country_code}/{party_id}/{token_uid}[?type={type}]`.
    ///
    /// > *`type`: Token.type of the Token of the Token object to retrieve. Default if omitted:
    /// > RFID*
    ///
    /// Spec: 2.3.0 §mod_tokens_cpo_interface
    #[must_use]
    pub fn token(&self, owner: &PartyRef, token_uid: &str, token_type: Option<&str>) -> Url {
        let url = self.object(owner, token_uid);
        match token_type {
            Some(t) => url.with_query(&format!("type={t}")),
            None => url,
        }
    }

    /// `{payments}/terminals` on a CPO's Receiver interface. See
    /// [`SenderEndpoint::payments_terminals`] for why this is derived rather than discovered.
    #[must_use]
    pub fn payments_terminals(&self) -> Self {
        Self::new(sub_path(&self.base, "terminals"))
    }

    /// `{payments}/financial-advice-confirmations` on a CPO's Receiver interface.
    #[must_use]
    pub fn payments_financial_advice_confirmations(&self) -> Self {
        Self::new(sub_path(&self.base, "financial-advice-confirmations"))
    }
}

/// Percent-encodes a URL so it can be carried as a query parameter value.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 16);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                use core::fmt::Write as _;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender(path: &str) -> SenderEndpoint {
        SenderEndpoint::new(Url::new(format!("https://e.com/ocpi/cpo/2.3.0/{path}")).unwrap())
    }
    fn receiver(path: &str) -> ReceiverEndpoint {
        ReceiverEndpoint::new(Url::new(format!("https://e.com/ocpi/emsp/2.3.0/{path}")).unwrap())
    }

    #[test]
    fn client_owned_object_urls_match_the_spec_example() {
        // "https://www.server.com/ocpi/cpo/2.2.1/tariffs/NL/TNM/14"
        let e = ReceiverEndpoint::new(Url::new("https://www.server.com/ocpi/cpo/2.2.1/tariffs").unwrap());
        assert_eq!(
            e.object(&PartyRef::new("NL", "TNM").unwrap(), "14").as_str(),
            "https://www.server.com/ocpi/cpo/2.2.1/tariffs/NL/TNM/14"
        );
    }

    #[test]
    fn a_trailing_slash_on_the_discovered_url_does_not_double_up() {
        let e = ReceiverEndpoint::new(Url::new("https://e.com/ocpi/emsp/2.3.0/tariffs/").unwrap());
        assert_eq!(
            e.object(&PartyRef::new("NL", "TNM").unwrap(), "14").as_str(),
            "https://e.com/ocpi/emsp/2.3.0/tariffs/NL/TNM/14"
        );
    }

    #[test]
    fn nested_location_urls_stop_where_the_caller_stops() {
        let s = sender("locations");
        assert_eq!(s.location("LOC1", None, None).as_str(), "https://e.com/ocpi/cpo/2.3.0/locations/LOC1");
        assert_eq!(
            s.location("LOC1", Some("3256"), None).as_str(),
            "https://e.com/ocpi/cpo/2.3.0/locations/LOC1/3256"
        );
        assert_eq!(
            s.location("LOC1", Some("3256"), Some("1")).as_str(),
            "https://e.com/ocpi/cpo/2.3.0/locations/LOC1/3256/1"
        );
        // A connector without an EVSE is not addressable; the EVSE segment wins.
        assert_eq!(
            s.location("LOC1", None, Some("1")).as_str(),
            "https://e.com/ocpi/cpo/2.3.0/locations/LOC1"
        );
    }

    #[test]
    fn token_authorization_carries_the_type_as_a_query_parameter() {
        let s = sender("tokens");
        assert_eq!(
            s.token_authorize("012345678", Some("RFID")).as_str(),
            "https://e.com/ocpi/cpo/2.3.0/tokens/012345678/authorize?type=RFID"
        );
        assert_eq!(
            s.token_authorize("012345678", None).as_str(),
            "https://e.com/ocpi/cpo/2.3.0/tokens/012345678/authorize"
        );
    }

    #[test]
    fn a_response_url_is_percent_encoded_into_the_query() {
        let r = receiver("chargingprofiles");
        let response = Url::new("https://msp.example.com/cb?id=1").unwrap();
        let url = r.active_charging_profile("101", 900, &response);
        assert_eq!(
            url.as_str(),
            "https://e.com/ocpi/emsp/2.3.0/chargingprofiles/101\
             ?duration=900&response_url=https%3A%2F%2Fmsp.example.com%2Fcb%3Fid%3D1"
        );
    }

    #[test]
    fn payment_terminal_urls_hang_off_the_discovered_payments_endpoint() {
        let s = sender("payments").payments_terminals();
        assert_eq!(s.terminal("TERM1").as_str(), "https://e.com/ocpi/cpo/2.3.0/payments/terminals/TERM1");
        assert_eq!(
            s.terminal_deactivate("TERM1").as_str(),
            "https://e.com/ocpi/cpo/2.3.0/payments/terminals/TERM1/deactivate"
        );
        assert_eq!(
            s.terminal_activate().as_str(),
            "https://e.com/ocpi/cpo/2.3.0/payments/terminals/activate"
        );
        assert_eq!(
            sender("payments").payments_financial_advice_confirmations().base().as_str(),
            "https://e.com/ocpi/cpo/2.3.0/payments/financial-advice-confirmations"
        );
    }

    #[test]
    fn a_peer_that_advertised_a_payments_sub_path_directly_still_works() {
        // The module has one ModuleID and two endpoint URLs, so peers differ on what they
        // publish. Both readings have to reach the same place.
        for advertised in ["payments", "payments/terminals", "payments/terminals/"] {
            assert_eq!(
                sender(advertised).payments_terminals().terminal("TERM1").as_str(),
                "https://e.com/ocpi/cpo/2.3.0/payments/terminals/TERM1",
                "advertised as {advertised}"
            );
        }
    }

    #[test]
    fn a_token_url_carries_the_owner_and_the_type() {
        let r = receiver("tokens");
        let owner = PartyRef::new("NL", "TNM").unwrap();
        assert_eq!(
            r.token(&owner, "012345678", Some("APP_USER")).as_str(),
            "https://e.com/ocpi/emsp/2.3.0/tokens/NL/TNM/012345678?type=APP_USER"
        );
        assert_eq!(
            r.token(&owner, "012345678", None).as_str(),
            "https://e.com/ocpi/emsp/2.3.0/tokens/NL/TNM/012345678"
        );
    }

    #[test]
    fn clearing_a_charging_profile_carries_the_response_url() {
        let r = receiver("chargingprofiles");
        let response = Url::new("https://msp.example.com/cb?id=1").unwrap();
        assert_eq!(
            r.clear_charging_profile("101", &response).as_str(),
            "https://e.com/ocpi/emsp/2.3.0/chargingprofiles/101\
             ?response_url=https%3A%2F%2Fmsp.example.com%2Fcb%3Fid%3D1"
        );
    }

    #[test]
    fn a_paginated_list_url_carries_the_filters() {
        let s = sender("cdrs");
        let q = PageQuery::new().with_offset(150).with_limit(50);
        assert_eq!(s.list(&q).as_str(), "https://e.com/ocpi/cpo/2.3.0/cdrs?offset=150&limit=50");
    }
}
