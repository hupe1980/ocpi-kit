//! Typed clients for the OCPI modules, in the canonical 2.3.0 model.
//!
//! Each module has up to two clients: a **Sender** client, which pulls from the party that owns the
//! data, and a **Receiver** client, which pushes to the party that receives it.
//!
//! They take and return [`v2_3_0`](crate::v2_3_0) objects **whatever version the peer speaks**: a
//! 2.2.1 CPO's Locations arrive here as 2.3.0 objects, and a `PUT` back to it is written in 2.2.1.
//! Load-bearing rather than convenient — a 2.2.1 `Tariff` has no `tax_included`, which 2.3.0
//! requires. The translation is [`convert`](crate::convert), and an outgoing object that loses a
//! field logs a `tracing` warning naming it by JSON Pointer.
//!
//! [`ModuleClient`]'s own `get`/`put`/`post`/`patch`/`list` translate nothing and decode exactly
//! the type you name; the `*_bridged` variants beside them are what the typed clients use.

use http::Method;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::convert::wire::{BridgeError, ObjectKind};
use crate::transport::{
    OcpiError, OcpiRequest, Page, PageQuery, Patch, ReceiverEndpoint, RequestIds, RoutingHeaders,
    SenderEndpoint,
};
use crate::types::{PartyRef, Url, Validate};
use crate::v2_3_0::tokens::TokenType;
use crate::{InterfaceRole, ModuleId};

use super::http::{Transport, check_outgoing};
use super::paging::PageStream;
use super::peer::Peer;

/// The shared plumbing of every module client.
#[derive(Clone, Debug)]
pub struct ModuleClient<'a> {
    transport: &'a Transport,
    peer: &'a Peer,
    module: ModuleId,
    from: PartyRef,
    to: Option<PartyRef>,
}

impl<'a> ModuleClient<'a> {
    /// Builds a client for one module of one peer.
    ///
    /// `from` is the party this process is speaking as; `to` is the party at the peer, which
    /// defaults to the peer's only party when it has just one.
    #[must_use]
    pub fn new(transport: &'a Transport, peer: &'a Peer, module: ModuleId, from: PartyRef) -> Self {
        let to = peer.default_party().cloned();
        Self { transport, peer, module, from, to }
    }

    /// Addresses a specific party at the peer, for a platform that hosts several.
    #[must_use]
    pub fn to(mut self, party: PartyRef) -> Self {
        self.to = Some(party);
        self
    }

    /// Omits the `OCPI-to-*` headers, making this an Open Routing Request.
    ///
    /// > *For an Open Routing Request, the TO headers in the request from the requesting party to
    /// > the Hub MUST be omitted.*
    #[must_use]
    pub fn open_routing(mut self) -> Self {
        self.to = None;
        self
    }

    /// The peer this client talks to.
    #[must_use]
    pub const fn peer(&self) -> &Peer {
        self.peer
    }

    /// The Sender endpoint of this module, if the peer implements it.
    #[must_use]
    pub fn sender_endpoint(&self) -> Option<SenderEndpoint> {
        self.peer.sender(&self.module)
    }

    /// The Receiver endpoint of this module, if the peer implements it.
    #[must_use]
    pub fn receiver_endpoint(&self) -> Option<ReceiverEndpoint> {
        self.peer.receiver(&self.module)
    }

    fn routing(&self) -> RoutingHeaders {
        RoutingHeaders { to: self.to.clone(), from: self.from.clone() }
    }

    fn request(&self, method: Method, url: Url) -> OcpiRequest {
        OcpiRequest::new(method, url, self.module.clone()).routed(self.routing())
    }

    fn missing(&self, role: InterfaceRole) -> OcpiError {
        OcpiError::NotFound(format!(
            "the peer does not implement the {} interface of the {} module",
            role, self.module
        ))
    }

    /// `GET {url}`, decoding one object.
    ///
    /// # Errors
    ///
    /// Propagates transport, decoding and OCPI-level errors.
    pub async fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T, OcpiError> {
        let request = self.request(Method::GET, url);
        self.transport.send(&request, self.peer.token(), self.peer.quirks()).await
    }

    /// `GET {url}`, decoding one page of a list endpoint.
    ///
    /// # Errors
    ///
    /// Propagates transport, decoding and OCPI-level errors.
    pub async fn get_page<T: DeserializeOwned>(&self, url: Url) -> Result<Page<T>, OcpiError> {
        let request = self.request(Method::GET, url);
        self.transport.send_page(&request, self.peer.token(), self.peer.quirks()).await
    }

    /// `PUT {url}` with a body, discarding the response payload.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::Invalid`] when the body does not conform and the client is configured
    /// to check outgoing objects, plus the usual transport and OCPI errors.
    pub async fn put<T: Serialize + Validate>(&self, url: Url, body: &T) -> Result<(), OcpiError> {
        check_outgoing(body, self.transport.config())?;
        let request = self.request(Method::PUT, url).with_body(body)?;
        let (response, _) = self
            .transport
            .send_with_headers::<serde_json::Value>(&request, self.peer.token(), self.peer.quirks())
            .await?;
        if response.is_success() { Ok(()) } else { Err(response.into_result().unwrap_err()) }
    }

    /// `POST {url}` with a body, decoding the response payload.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::put`].
    pub async fn post<B: Serialize + Validate, T: DeserializeOwned>(
        &self,
        url: Url,
        body: &B,
    ) -> Result<T, OcpiError> {
        check_outgoing(body, self.transport.config())?;
        let request = self.request(Method::POST, url).with_body(body)?;
        self.transport.send(&request, self.peer.token(), self.peer.quirks()).await
    }

    /// `PATCH {url}` with a merge patch.
    ///
    /// The patch must carry `last_updated`; the specification's own example of a `2001 Invalid or
    /// missing parameters` is a PATCH that does not.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::Decode`] when the patch has no `last_updated`, plus the usual
    /// transport and OCPI errors.
    pub async fn patch<T>(&self, url: Url, patch: &Patch<T>) -> Result<(), OcpiError> {
        if patch.last_updated().is_none() {
            return Err(OcpiError::Decode {
                path: "/last_updated".to_owned(),
                message: "a PATCH must carry `last_updated`".to_owned(),
            });
        }
        let request = self.request(Method::PATCH, url).with_body(patch.as_value())?;
        let (response, _) = self
            .transport
            .send_with_headers::<serde_json::Value>(&request, self.peer.token(), self.peer.quirks())
            .await?;
        if response.is_success() { Ok(()) } else { Err(response.into_result().unwrap_err()) }
    }

    /// `DELETE {url}`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn delete(&self, url: Url) -> Result<(), OcpiError> {
        let request = self.request(Method::DELETE, url);
        let (response, _) = self
            .transport
            .send_with_headers::<serde_json::Value>(&request, self.peer.token(), self.peer.quirks())
            .await?;
        if response.is_success() { Ok(()) } else { Err(response.into_result().unwrap_err()) }
    }

    /// The peer's version, when it is one this crate has to translate for. `None` is the fast
    /// path: the peer already speaks the canonical model.
    fn foreign_version(&self) -> Option<&crate::VersionNumber> {
        let version = self.peer.version();
        (*version != crate::CANONICAL_VERSION).then_some(version)
    }

    /// `GET {url}`, translating the peer's version into the canonical model.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::get`], plus [`OcpiError::Decode`] when the peer's document is not the
    /// object this endpoint carries, and [`OcpiError::Unsupported`] when this build has no
    /// conversions for the peer's version.
    pub async fn get_bridged<T: DeserializeOwned>(&self, url: Url, kind: ObjectKind) -> Result<T, OcpiError> {
        let Some(theirs) = self.foreign_version() else { return self.get(url).await };
        let value: serde_json::Value = self.get(url).await?;
        let converted =
            kind.bridge(theirs, &crate::CANONICAL_VERSION, value).map_err(|e| bridge_error(e, kind))?;
        decode(converted.value)
    }

    /// `PUT {url}`, writing the body in the version the peer speaks.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::put`], plus [`OcpiError::Unsupported`] when this build cannot write the
    /// peer's version.
    pub async fn put_bridged<T: Serialize + Validate>(
        &self,
        url: Url,
        body: &T,
        kind: ObjectKind,
    ) -> Result<(), OcpiError> {
        check_outgoing(body, self.transport.config())?;
        let Some(value) = self.for_peer(body, kind)? else { return self.put(url, body).await };
        let request = self.request(Method::PUT, url).with_body(&value)?;
        self.expect_success(request).await
    }

    /// `POST {url}`, writing the body in the peer's version and reading the answer back out of it.
    ///
    /// `request_kind` and `response_kind` are separate because two OCPI endpoints send one object
    /// and answer with another — `POST {tokens}/{uid}/authorize` takes a `LocationReferences` and
    /// returns an `AuthorizationInfo`.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::post`], plus the translation errors of
    /// [`ModuleClient::get_bridged`].
    pub async fn post_bridged<B: Serialize + Validate, T: DeserializeOwned>(
        &self,
        url: Url,
        body: &B,
        request_kind: Option<ObjectKind>,
        response_kind: Option<ObjectKind>,
    ) -> Result<T, OcpiError> {
        check_outgoing(body, self.transport.config())?;
        let Some(theirs) = self.foreign_version().cloned() else {
            return self.post(url, body).await;
        };
        let request = match request_kind.and_then(|k| self.for_peer(body, k).transpose()) {
            Some(value) => self.request(Method::POST, url).with_body(&value?)?,
            None => self.request(Method::POST, url).with_body(body)?,
        };
        let answer: serde_json::Value =
            self.transport.send(&request, self.peer.token(), self.peer.quirks()).await?;
        let Some(kind) = response_kind else { return decode(answer) };
        let converted =
            kind.bridge(&theirs, &crate::CANONICAL_VERSION, answer).map_err(|e| bridge_error(e, kind))?;
        decode(converted.value)
    }

    /// `PATCH {url}` against a peer on another version.
    ///
    /// A merge patch is not an object, so it cannot be decoded, converted and re-encoded. It does
    /// not have to be: a patch writing only fields the two versions agree about means the same
    /// thing in both. One that writes a field they disagree about is refused, with the
    /// specification's own GET → PUT recovery in the message.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::patch`], plus [`OcpiError::Unsupported`] when the patch writes a field
    /// whose shape differs between the two versions.
    pub async fn patch_bridged<T>(
        &self,
        url: Url,
        patch: &Patch<T>,
        kind: ObjectKind,
    ) -> Result<(), OcpiError> {
        if let Some(theirs) = self.foreign_version()
            && !kind.patch_crosses_unchanged(&patch.fields())
        {
            return Err(OcpiError::Unsupported(format!(
                "this PATCH writes {:?}, and a {kind} does not carry {} the same way in OCPI \
                 {theirs} as in OCPI {}; a merge patch is not an object, so it cannot be \
                 translated. GET the object and PUT it back instead, which is the recovery the \
                 specification prescribes for a refused PATCH",
                patch.fields(),
                kind.divergent_fields().join(", "),
                crate::CANONICAL_VERSION,
            )));
        }
        self.patch(url, patch).await
    }

    /// Crawls a list endpoint, translating every page into the canonical model.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::list`].
    pub fn list_bridged<T: DeserializeOwned + Send + 'static>(
        &self,
        query: PageQuery,
        kind: ObjectKind,
    ) -> Result<PageStream<'a, T>, OcpiError> {
        Ok(self.list(query)?.bridging(kind))
    }

    /// Serialises `body` in the peer's version, or `None` when nothing has to change.
    fn for_peer<T: Serialize>(
        &self,
        body: &T,
        kind: ObjectKind,
    ) -> Result<Option<serde_json::Value>, OcpiError> {
        let Some(theirs) = self.foreign_version() else { return Ok(None) };
        let value = serde_json::to_value(body)
            .map_err(|e| OcpiError::Decode { path: "/".to_owned(), message: e.to_string() })?;
        let converted =
            kind.bridge(&crate::CANONICAL_VERSION, theirs, value).map_err(|e| bridge_error(e, kind))?;
        if let Some(note) = converted.lossy.to_status_message() {
            tracing::warn!(
                ocpi.peer_version = %theirs,
                ocpi.object = %kind,
                "{note}",
            );
        }
        Ok(Some(converted.value))
    }

    async fn expect_success(&self, request: OcpiRequest) -> Result<(), OcpiError> {
        let (response, _) = self
            .transport
            .send_with_headers::<serde_json::Value>(&request, self.peer.token(), self.peer.quirks())
            .await?;
        if response.is_success() { Ok(()) } else { Err(response.into_result().unwrap_err()) }
    }

    /// Crawls every page of a Sender list endpoint.
    ///
    /// The objects arrive exactly as the peer wrote them; see [`ModuleClient::list_bridged`] for
    /// the version-translating form the typed clients use.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::NotFound`] when the peer does not implement the Sender interface.
    pub fn list<T: DeserializeOwned + Send + 'static>(
        &self,
        query: PageQuery,
    ) -> Result<PageStream<'a, T>, OcpiError> {
        let endpoint = self.sender_endpoint().ok_or_else(|| self.missing(InterfaceRole::Sender))?;
        let query = match self.peer.quirks().peer_max_page_limit {
            Some(max) => query.clamped_to(max),
            None => query,
        };
        Ok(PageStream::new(
            self.transport,
            self.peer,
            self.module.clone(),
            self.routing(),
            endpoint.list(&query),
        ))
    }
}

impl<'a> ModuleClient<'a> {
    /// A paginated crawl starting from an explicit URL rather than the module's own list
    /// endpoint.
    ///
    /// Most modules have one list endpoint and [`list`](Self::list) finds it. Payments is the
    /// exception: it declares a single `ModuleID` and then addresses its two interfaces through
    /// two different endpoint variables, which version discovery cannot express, so the sub-path
    /// has to come from the caller. See
    /// [`SenderEndpoint::payments_terminals`](crate::transport::SenderEndpoint::payments_terminals).
    #[must_use]
    pub fn list_from<T: DeserializeOwned + Send + 'static>(
        &self,
        base: &Url,
        query: &PageQuery,
    ) -> PageStream<'a, T> {
        PageStream::new(self.transport, self.peer, self.module.clone(), self.routing(), query.apply_to(base))
    }
}

/// Pulls Locations from a CPO.
///
/// Spec: 2.3.0 §mod_locations_cpo_interface
#[derive(Clone, Debug)]
pub struct LocationsSender<'a>(ModuleClient<'a>);

impl<'a> LocationsSender<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {locations}` — every Location, paginated.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::list`].
    pub fn list(
        &self,
        query: PageQuery,
    ) -> Result<PageStream<'a, crate::v2_3_0::locations::Location>, OcpiError> {
        self.0.list_bridged(query, ObjectKind::Location)
    }

    /// `GET {locations}/{location_id}`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn location(&self, location_id: &str) -> Result<crate::v2_3_0::locations::Location, OcpiError> {
        let endpoint = self.0.sender_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Sender))?;
        self.0.get_bridged(endpoint.location(location_id, None, None), ObjectKind::Location).await
    }

    /// `GET {locations}/{location_id}/{evse_uid}`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn evse(
        &self,
        location_id: &str,
        evse_uid: &str,
    ) -> Result<crate::v2_3_0::locations::Evse, OcpiError> {
        let endpoint = self.0.sender_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Sender))?;
        self.0.get_bridged(endpoint.location(location_id, Some(evse_uid), None), ObjectKind::Evse).await
    }

    /// `GET {locations}/{location_id}/{evse_uid}/{connector_id}`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn connector(
        &self,
        location_id: &str,
        evse_uid: &str,
        connector_id: &str,
    ) -> Result<crate::v2_3_0::locations::Connector, OcpiError> {
        let endpoint = self.0.sender_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Sender))?;
        self.0
            .get_bridged(
                endpoint.location(location_id, Some(evse_uid), Some(connector_id)),
                ObjectKind::Connector,
            )
            .await
    }
}

/// Pushes Locations to an eMSP or NSP.
///
/// These are **client-owned objects**: the URL carries the owner's `country_code` and `party_id`,
/// and `POST` is not used.
///
/// > *POST is not supported for these kinds of modules. PUT is used to send new objects.*
///
/// Spec: 2.3.0 §mod_locations_emsp_interface
#[derive(Clone, Debug)]
pub struct LocationsReceiver<'a>(ModuleClient<'a>);

impl<'a> LocationsReceiver<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `PUT {locations}/{country_code}/{party_id}/{location_id}`.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn put_location(
        &self,
        owner: &PartyRef,
        location: &crate::v2_3_0::locations::Location,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0
            .put_bridged(
                endpoint.location(owner, location.id.as_str(), None, None),
                location,
                ObjectKind::Location,
            )
            .await
    }

    /// `PUT {locations}/{country_code}/{party_id}/{location_id}/{evse_uid}`.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn put_evse(
        &self,
        owner: &PartyRef,
        location_id: &str,
        evse: &crate::v2_3_0::locations::Evse,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0
            .put_bridged(
                endpoint.location(owner, location_id, Some(evse.uid.as_str()), None),
                evse,
                ObjectKind::Evse,
            )
            .await
    }

    /// `PATCH {locations}/{country_code}/{party_id}/{location_id}[/{evse_uid}[/{connector_id}]]`.
    ///
    /// This is how an EVSE is retired: *"REMOVED via PATCH status, never DELETE"*.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors, and refuses a patch without `last_updated`.
    pub async fn patch<T>(
        &self,
        owner: &PartyRef,
        location_id: &str,
        evse_uid: Option<&str>,
        connector_id: Option<&str>,
        patch: &Patch<T>,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        let kind = match (evse_uid, connector_id) {
            (None, _) => ObjectKind::Location,
            (Some(_), None) => ObjectKind::Evse,
            (Some(_), Some(_)) => ObjectKind::Connector,
        };
        self.0.patch_bridged(endpoint.location(owner, location_id, evse_uid, connector_id), patch, kind).await
    }
}

/// Real-time authorization and Token pulls, on the eMSP's Sender interface.
///
/// Spec: 2.3.0 §mod_tokens_emsp_interface
#[derive(Clone, Debug)]
pub struct TokensSender<'a>(ModuleClient<'a>);

impl<'a> TokensSender<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {tokens}` — every Token, paginated.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::list`].
    pub fn list(&self, query: PageQuery) -> Result<PageStream<'a, crate::v2_3_0::tokens::Token>, OcpiError> {
        self.0.list_bridged(query, ObjectKind::Token)
    }

    /// `POST {tokens}/{token_uid}/authorize[?type=]` — a real-time authorization.
    ///
    /// > *`LocationReferences`: Location and EVSEs for which the driver wants to charge.*
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors; a `2004 Unknown Token` from the eMSP arrives
    /// as [`OcpiError::Remote`].
    pub async fn authorize(
        &self,
        token_uid: &str,
        token_type: Option<crate::v2_3_0::tokens::TokenType>,
        location: Option<&crate::v2_3_0::tokens::LocationReferences>,
    ) -> Result<crate::v2_3_0::tokens::AuthorizationInfo, OcpiError> {
        let endpoint = self.0.sender_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Sender))?;
        let url = endpoint.token_authorize(
            token_uid,
            token_type.as_ref().map(super::super::v2_3_0::tokens::TokenType::as_str),
        );
        // The request is a `LocationReferences`, which is the same object in both versions;
        // the answer is an `AuthorizationInfo`, which is not.
        match location {
            Some(references) => {
                self.0.post_bridged(url, references, None, Some(ObjectKind::AuthorizationInfo)).await
            }
            // The body is optional; an empty object keeps the Content-Type consistent.
            None => {
                self.0
                    .post_bridged(url, &serde_json::json!({}), None, Some(ObjectKind::AuthorizationInfo))
                    .await
            }
        }
    }
}

/// Pulls CDRs from a CPO, and pushes them to an eMSP.
///
/// The CDRs module is the one place OCPI uses a server-owned `POST`:
///
/// > *POST … returns the URL to the new object in the `Location` header.*
///
/// Spec: 2.3.0 §mod_cdrs_cdrs_module
#[derive(Clone, Debug)]
pub struct CdrsClient<'a>(ModuleClient<'a>);

impl<'a> CdrsClient<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {cdrs}` — every CDR, paginated.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::list`].
    pub fn list(&self, query: PageQuery) -> Result<PageStream<'a, crate::v2_3_0::cdrs::Cdr>, OcpiError> {
        self.0.list_bridged(query, ObjectKind::Cdr)
    }

    /// `POST {cdrs}` — pushes a CDR, returning the URL from the `Location` response header.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn post(&self, cdr: &crate::v2_3_0::cdrs::Cdr) -> Result<Option<Url>, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        check_outgoing(cdr, self.0.transport.config())?;
        let request = match self.0.for_peer(cdr, ObjectKind::Cdr)? {
            Some(value) => self.0.request(Method::POST, endpoint.base().clone()).with_body(&value)?,
            None => self.0.request(Method::POST, endpoint.base().clone()).with_body(cdr)?,
        };
        let (response, headers) = self
            .0
            .transport
            .send_with_headers::<serde_json::Value>(&request, self.0.peer.token(), self.0.peer.quirks())
            .await?;
        if !response.is_success() {
            return Err(response.into_result().unwrap_err());
        }
        Ok(crate::transport::header_str(&headers, &crate::transport::headers::LOCATION).map(Url::new_lenient))
    }
}

/// Sends commands to a CPO.
///
/// Spec: 2.3.0 §mod_commands_commands_module
#[derive(Clone, Debug)]
pub struct CommandsClient<'a>(ModuleClient<'a>);

impl<'a> CommandsClient<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `POST {commands}/{command}` — sends a command and returns the synchronous response.
    ///
    /// The [`CommandResponse`](crate::v2_3_0::commands::CommandResponse) carries a `timeout`; the
    /// asynchronous [`CommandResult`](crate::v2_3_0::commands::CommandResult) arrives later at the
    /// `response_url` the command carried, which this party must be serving.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn send(
        &self,
        command: &crate::v2_3_0::commands::Command,
    ) -> Result<crate::v2_3_0::commands::CommandResponse, OcpiError> {
        use crate::v2_3_0::commands::Command;
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        // The `response_url` is a URL this party will be called back on, so it is checked against
        // the same policy as anything else this process would fetch.
        let url = endpoint.base().join(command.command_type().as_str());
        match command {
            // `CommandResponse` is the same object in every version; only two of the five
            // request bodies carry a `Token`, and only that is translated.
            Command::CancelReservation(c) => self.0.post_bridged(url, c, None, None).await,
            Command::ReserveNow(c) => {
                self.0.post_bridged(url, c.as_ref(), Some(ObjectKind::ReserveNow), None).await
            }
            Command::StartSession(c) => {
                self.0.post_bridged(url, c.as_ref(), Some(ObjectKind::StartSession), None).await
            }
            Command::StopSession(c) => self.0.post_bridged(url, c, None, None).await,
            Command::UnlockConnector(c) => self.0.post_bridged(url, c, None, None).await,
        }
    }
}

/// Pulls Sessions from a CPO, and sets a driver's charging preferences.
///
/// Spec: 2.3.0 §mod_sessions_cpo_interface
#[derive(Clone, Debug)]
pub struct SessionsSender<'a>(ModuleClient<'a>);

impl<'a> SessionsSender<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {sessions}` — every Session, paginated.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::list`].
    pub fn list(
        &self,
        query: PageQuery,
    ) -> Result<PageStream<'a, crate::v2_3_0::sessions::Session>, OcpiError> {
        self.0.list_bridged(query, ObjectKind::Session)
    }

    /// `PUT {sessions}/{session_id}/charging_preferences`.
    ///
    /// The response is a
    /// [`ChargingPreferencesResponse`](crate::v2_3_0::sessions::ChargingPreferencesResponse), not
    /// an acknowledgement: a CPO that accepts the request may still answer
    /// `NOT_POSSIBLE`, and the caller has to look.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn set_charging_preferences(
        &self,
        session_id: &str,
        preferences: &crate::v2_3_0::sessions::ChargingPreferences,
    ) -> Result<crate::v2_3_0::sessions::ChargingPreferencesResponse, OcpiError> {
        let endpoint = self.0.sender_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Sender))?;
        check_outgoing(preferences, self.0.transport.config())?;
        let request =
            self.0.request(Method::PUT, endpoint.charging_preferences(session_id)).with_body(preferences)?;
        self.0.transport.send(&request, self.0.peer.token(), self.0.peer.quirks()).await
    }
}

/// Pushes Sessions to an eMSP.
///
/// Spec: 2.3.0 §mod_sessions_emsp_interface
#[derive(Clone, Debug)]
pub struct SessionsReceiver<'a>(ModuleClient<'a>);

impl<'a> SessionsReceiver<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {sessions}/{country_code}/{party_id}/{session_id}` — what the peer has stored.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn session(
        &self,
        owner: &PartyRef,
        session_id: &str,
    ) -> Result<crate::v2_3_0::sessions::Session, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0.get_bridged(endpoint.object(owner, session_id), ObjectKind::Session).await
    }

    /// `PUT {sessions}/{country_code}/{party_id}/{session_id}`.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn put_session(
        &self,
        owner: &PartyRef,
        session: &crate::v2_3_0::sessions::Session,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0.put_bridged(endpoint.object(owner, session.id.as_str()), session, ObjectKind::Session).await
    }

    /// `PATCH {sessions}/{country_code}/{party_id}/{session_id}`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors, and refuses a patch without `last_updated`.
    pub async fn patch<T>(
        &self,
        owner: &PartyRef,
        session_id: &str,
        patch: &Patch<T>,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0.patch_bridged(endpoint.object(owner, session_id), patch, ObjectKind::Session).await
    }
}

/// Pulls Tariffs from a CPO.
///
/// Spec: 2.3.0 §mod_tariffs_cpo_interface
#[derive(Clone, Debug)]
pub struct TariffsSender<'a>(ModuleClient<'a>);

impl<'a> TariffsSender<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {tariffs}` — every Tariff, paginated.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::list`].
    pub fn list(
        &self,
        query: PageQuery,
    ) -> Result<PageStream<'a, crate::v2_3_0::tariffs::Tariff>, OcpiError> {
        self.0.list_bridged(query, ObjectKind::Tariff)
    }
}

/// Pushes Tariffs to an eMSP.
///
/// The Tariffs Receiver interface is the one client-owned-object interface with a `DELETE`:
/// a Tariff that no longer applies is removed, not marked.
///
/// Spec: 2.3.0 §mod_tariffs_emsp_interface
#[derive(Clone, Debug)]
pub struct TariffsReceiver<'a>(ModuleClient<'a>);

impl<'a> TariffsReceiver<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {tariffs}/{country_code}/{party_id}/{tariff_id}`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn tariff(
        &self,
        owner: &PartyRef,
        tariff_id: &str,
    ) -> Result<crate::v2_3_0::tariffs::Tariff, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0.get_bridged(endpoint.object(owner, tariff_id), ObjectKind::Tariff).await
    }

    /// `PUT {tariffs}/{country_code}/{party_id}/{tariff_id}`.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn put_tariff(
        &self,
        owner: &PartyRef,
        tariff: &crate::v2_3_0::tariffs::Tariff,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0.put_bridged(endpoint.object(owner, tariff.id.as_str()), tariff, ObjectKind::Tariff).await
    }

    /// `DELETE {tariffs}/{country_code}/{party_id}/{tariff_id}`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn delete_tariff(&self, owner: &PartyRef, tariff_id: &str) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0.delete(endpoint.object(owner, tariff_id)).await
    }
}

/// Pushes Tokens to a CPO.
///
/// Spec: 2.3.0 §mod_tokens_cpo_interface
#[derive(Clone, Debug)]
pub struct TokensReceiver<'a>(ModuleClient<'a>);

impl<'a> TokensReceiver<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {tokens}/{country_code}/{party_id}/{token_uid}[?type=]`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn token(
        &self,
        owner: &PartyRef,
        token_uid: &str,
        token_type: Option<crate::v2_3_0::tokens::TokenType>,
    ) -> Result<crate::v2_3_0::tokens::Token, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0
            .get_bridged(
                endpoint.token(owner, token_uid, token_type.as_ref().map(TokenType::as_str)),
                ObjectKind::Token,
            )
            .await
    }

    /// `PUT {tokens}/{country_code}/{party_id}/{token_uid}[?type=]`.
    ///
    /// The `type` is taken from the Token itself, which is where the peer will look for it too.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn put_token(
        &self,
        owner: &PartyRef,
        token: &crate::v2_3_0::tokens::Token,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        let url = endpoint.token(owner, token.uid.as_str(), Some(token.token_type.as_str()));
        self.0.put_bridged(url, token, ObjectKind::Token).await
    }

    /// `PATCH {tokens}/{country_code}/{party_id}/{token_uid}[?type=]`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors, and refuses a patch without `last_updated`.
    pub async fn patch<T>(
        &self,
        owner: &PartyRef,
        token_uid: &str,
        token_type: Option<crate::v2_3_0::tokens::TokenType>,
        patch: &Patch<T>,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        let url = endpoint.token(owner, token_uid, token_type.as_ref().map(TokenType::as_str));
        self.0.patch_bridged(url, patch, ObjectKind::Token).await
    }
}

/// Drives a CPO's Charging Profiles Receiver interface, as an eMSP or SCSP.
///
/// Every method here answers twice: the [`ChargingProfileResponse`] returned is the CPO's own
/// immediate verdict, and — when that is `ACCEPTED` — the Charge Point's answer follows at the
/// `response_url`, which this party must be serving. Build those URLs with
/// [`CallbackUrls`](crate::server::CallbackUrls) if the server side is this crate's too.
///
/// [`ChargingProfileResponse`]: crate::v2_3_0::charging_profiles::ChargingProfileResponse
///
/// Spec: 2.3.0 §mod_charging_profiles_cpo_interface
#[derive(Clone, Debug)]
pub struct ChargingProfilesClient<'a>(ModuleClient<'a>);

impl<'a> ChargingProfilesClient<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {chargingprofiles}/{session_id}?duration=&response_url=`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn active_charging_profile(
        &self,
        session_id: &str,
        duration_seconds: u64,
        response_url: &Url,
    ) -> Result<crate::v2_3_0::charging_profiles::ChargingProfileResponse, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        self.0.get(endpoint.active_charging_profile(session_id, duration_seconds, response_url)).await
    }

    /// `PUT {chargingprofiles}/{session_id}` with a `SetChargingProfile` body.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn set_charging_profile(
        &self,
        session_id: &str,
        request: &crate::v2_3_0::charging_profiles::SetChargingProfile,
    ) -> Result<crate::v2_3_0::charging_profiles::ChargingProfileResponse, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        check_outgoing(request, self.0.transport.config())?;
        let outgoing =
            self.0.request(Method::PUT, endpoint.charging_profile(session_id)).with_body(request)?;
        self.0.transport.send(&outgoing, self.0.peer.token(), self.0.peer.quirks()).await
    }

    /// `DELETE {chargingprofiles}/{session_id}?response_url=`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn clear_charging_profile(
        &self,
        session_id: &str,
        response_url: &Url,
    ) -> Result<crate::v2_3_0::charging_profiles::ChargingProfileResponse, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        let outgoing =
            self.0.request(Method::DELETE, endpoint.clear_charging_profile(session_id, response_url));
        self.0.transport.send(&outgoing, self.0.peer.token(), self.0.peer.quirks()).await
    }

    /// `PUT {chargingprofiles}/{session_id}` on the **Sender** interface — a CPO volunteering a
    /// changed active profile to the party that set one.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn push_active_charging_profile(
        &self,
        session_id: &str,
        profile: &crate::v2_3_0::charging_profiles::ActiveChargingProfile,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.sender_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Sender))?;
        self.0.put(endpoint.object(session_id), profile).await
    }
}

/// Reads and pushes `ClientInfo`, the hub's view of who is connected.
///
/// A configuration module: these requests carry no routing headers, which
/// [`OcpiRequest::routed`](crate::transport::OcpiRequest::routed) enforces on the way out.
///
/// Spec: 2.3.0 §mod_hub_client_info_module
#[derive(Clone, Debug)]
pub struct HubClientInfoClient<'a>(ModuleClient<'a>);

impl<'a> HubClientInfoClient<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    /// `GET {hubclientinfo}` — every `ClientInfo` the hub publishes, paginated.
    ///
    /// # Errors
    ///
    /// As [`ModuleClient::list`].
    pub fn list(
        &self,
        query: PageQuery,
    ) -> Result<PageStream<'a, crate::v2_3_0::hub_client_info::ClientInfo>, OcpiError> {
        self.0.list_bridged(query, ObjectKind::ClientInfo)
    }

    /// `GET {hubclientinfo}/{country_code}/{party_id}` — one party's status.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn client_info(
        &self,
        party: &PartyRef,
    ) -> Result<crate::v2_3_0::hub_client_info::ClientInfo, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        let url = endpoint.base().join(party.country_code.as_str()).join(party.party_id.as_str());
        self.0.get_bridged(url, ObjectKind::ClientInfo).await
    }

    /// `PUT {hubclientinfo}/{country_code}/{party_id}` — the hub telling a client about a party.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn put_client_info(
        &self,
        info: &crate::v2_3_0::hub_client_info::ClientInfo,
    ) -> Result<(), OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        let url = endpoint.base().join(info.country_code.as_str()).join(info.party_id.as_str());
        self.0.put_bridged(url, info, ObjectKind::ClientInfo).await
    }
}

/// The Payments module, from either side.
///
/// The PTP owns the objects, so the *Sender* methods are the ones a CPO calls to drive a
/// terminal, and the *Receiver* methods are the ones a PTP calls to seed the CPO's copy. See
/// [`SenderEndpoint::payments_terminals`](crate::transport::SenderEndpoint::payments_terminals)
/// for how the module's two endpoint URLs are resolved from one discovered `payments` endpoint.
///
/// Spec: 2.3.0 §mod_payments_payments_module
#[derive(Clone, Debug)]
pub struct PaymentsClient<'a>(ModuleClient<'a>);

impl<'a> PaymentsClient<'a> {
    /// Wraps a module client.
    #[must_use]
    pub const fn new(client: ModuleClient<'a>) -> Self {
        Self(client)
    }

    fn terminals(&self) -> Result<SenderEndpoint, OcpiError> {
        Ok(self
            .0
            .sender_endpoint()
            .ok_or_else(|| self.0.missing(InterfaceRole::Sender))?
            .payments_terminals())
    }

    fn confirmations(&self) -> Result<SenderEndpoint, OcpiError> {
        Ok(self
            .0
            .sender_endpoint()
            .ok_or_else(|| self.0.missing(InterfaceRole::Sender))?
            .payments_financial_advice_confirmations())
    }

    /// `GET {payments}/terminals` — every Terminal, paginated.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub fn list_terminals(
        &self,
        query: PageQuery,
    ) -> Result<PageStream<'a, crate::v2_3_0::payments::Terminal>, OcpiError> {
        let endpoint = self.terminals()?;
        Ok(PageStream::new(
            self.0.transport,
            self.0.peer,
            self.0.module.clone(),
            self.0.routing(),
            endpoint.list(&query),
        ))
    }

    /// `GET {payments}/terminals/{terminal_id}`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn terminal(&self, terminal_id: &str) -> Result<crate::v2_3_0::payments::Terminal, OcpiError> {
        self.0.get(self.terminals()?.terminal(terminal_id)).await
    }

    /// `PUT {payments}/terminals/{terminal_id}` — the CPO updating a terminal's location data.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn put_terminal(
        &self,
        terminal: &crate::v2_3_0::payments::Terminal,
    ) -> Result<crate::v2_3_0::payments::Terminal, OcpiError> {
        check_outgoing(terminal, self.0.transport.config())?;
        let url = self.terminals()?.terminal(terminal.terminal_id.as_str());
        let request = self.0.request(Method::PUT, url).with_body(terminal)?;
        self.0.transport.send(&request, self.0.peer.token(), self.0.peer.quirks()).await
    }

    /// `PATCH {payments}/terminals/{terminal_id}` — assigning Locations or EVSEs.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn patch_terminal<T>(
        &self,
        terminal_id: &str,
        patch: &Patch<T>,
    ) -> Result<crate::v2_3_0::payments::Terminal, OcpiError> {
        let url = self.terminals()?.terminal(terminal_id);
        let request = self.0.request(Method::PATCH, url).with_body(patch.as_value())?;
        self.0.transport.send(&request, self.0.peer.token(), self.0.peer.quirks()).await
    }

    /// `POST {payments}/terminals/activate`.
    ///
    /// The body is a [`Patch`] rather than a `Terminal`, because
    /// *"the terminal_id is optional in the activation request as it will be set by the PTP"* —
    /// which is not a `Terminal`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn activate_terminal<T>(
        &self,
        terminal: &Patch<T>,
    ) -> Result<crate::v2_3_0::payments::Terminal, OcpiError> {
        let request = self
            .0
            .request(Method::POST, self.terminals()?.terminal_activate())
            .with_body(terminal.as_value())?;
        self.0.transport.send(&request, self.0.peer.token(), self.0.peer.quirks()).await
    }

    /// `POST {payments}/terminals/{terminal_id}/deactivate`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn deactivate_terminal(
        &self,
        terminal_id: &str,
    ) -> Result<crate::v2_3_0::payments::Terminal, OcpiError> {
        let url = self.terminals()?.terminal_deactivate(terminal_id);
        let request = self.0.request(Method::POST, url).with_body(&serde_json::json!({}))?;
        self.0.transport.send(&request, self.0.peer.token(), self.0.peer.quirks()).await
    }

    /// `GET {payments}/financial-advice-confirmations` — paginated.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub fn list_financial_advice_confirmations(
        &self,
        query: PageQuery,
    ) -> Result<PageStream<'a, crate::v2_3_0::payments::FinancialAdviceConfirmation>, OcpiError> {
        let endpoint = self.confirmations()?;
        Ok(PageStream::new(
            self.0.transport,
            self.0.peer,
            self.0.module.clone(),
            self.0.routing(),
            endpoint.list(&query),
        ))
    }

    /// `GET {payments}/financial-advice-confirmations/{id}`.
    ///
    /// # Errors
    ///
    /// Propagates transport and OCPI-level errors.
    pub async fn financial_advice_confirmation(
        &self,
        id: &str,
    ) -> Result<crate::v2_3_0::payments::FinancialAdviceConfirmation, OcpiError> {
        self.0.get(self.confirmations()?.object(id)).await
    }

    /// `POST {payments}/terminals` on the CPO's **Receiver** interface — the PTP creating a
    /// terminal in the CPO's system.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn post_terminal_to_receiver(
        &self,
        terminal: &crate::v2_3_0::payments::Terminal,
    ) -> Result<crate::v2_3_0::payments::Terminal, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        check_outgoing(terminal, self.0.transport.config())?;
        let url = endpoint.payments_terminals().base().clone();
        let request = self.0.request(Method::POST, url).with_body(terminal)?;
        self.0.transport.send(&request, self.0.peer.token(), self.0.peer.quirks()).await
    }

    /// `POST {payments}/financial-advice-confirmations` on the CPO's **Receiver** interface.
    ///
    /// # Errors
    ///
    /// Propagates validation, transport and OCPI-level errors.
    pub async fn post_financial_advice_confirmation(
        &self,
        confirmation: &crate::v2_3_0::payments::FinancialAdviceConfirmation,
    ) -> Result<crate::v2_3_0::payments::FinancialAdviceConfirmation, OcpiError> {
        let endpoint = self.0.receiver_endpoint().ok_or_else(|| self.0.missing(InterfaceRole::Receiver))?;
        check_outgoing(confirmation, self.0.transport.config())?;
        let url = endpoint.payments_financial_advice_confirmations().base().clone();
        let request = self.0.request(Method::POST, url).with_body(confirmation)?;
        self.0.transport.send(&request, self.0.peer.token(), self.0.peer.quirks()).await
    }
}

/// Convenience constructors hanging off a [`Peer`].
impl Peer {
    /// A client for an arbitrary module.
    #[must_use]
    pub fn module<'a>(
        &'a self,
        transport: &'a Transport,
        module: ModuleId,
        from: PartyRef,
    ) -> ModuleClient<'a> {
        ModuleClient::new(transport, self, module, from)
    }

    /// The Locations Sender client: pull Locations from this peer.
    #[must_use]
    pub fn locations<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> LocationsSender<'a> {
        LocationsSender::new(self.module(transport, ModuleId::Locations, from))
    }

    /// The Locations Receiver client: push Locations to this peer.
    #[must_use]
    pub fn locations_receiver<'a>(
        &'a self,
        transport: &'a Transport,
        from: PartyRef,
    ) -> LocationsReceiver<'a> {
        LocationsReceiver::new(self.module(transport, ModuleId::Locations, from))
    }

    /// The Tokens Sender client: pull Tokens from, and authorize against, this peer.
    #[must_use]
    pub fn tokens<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> TokensSender<'a> {
        TokensSender::new(self.module(transport, ModuleId::Tokens, from))
    }

    /// The CDRs client.
    #[must_use]
    pub fn cdrs<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> CdrsClient<'a> {
        CdrsClient::new(self.module(transport, ModuleId::Cdrs, from))
    }

    /// The Tokens Receiver client: push Tokens to this peer.
    #[must_use]
    pub fn tokens_receiver<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> TokensReceiver<'a> {
        TokensReceiver::new(self.module(transport, ModuleId::Tokens, from))
    }

    /// The Sessions Sender client: pull Sessions from this peer.
    #[must_use]
    pub fn sessions<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> SessionsSender<'a> {
        SessionsSender::new(self.module(transport, ModuleId::Sessions, from))
    }

    /// The Sessions Receiver client: push Sessions to this peer.
    #[must_use]
    pub fn sessions_receiver<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> SessionsReceiver<'a> {
        SessionsReceiver::new(self.module(transport, ModuleId::Sessions, from))
    }

    /// The Tariffs Sender client: pull Tariffs from this peer.
    #[must_use]
    pub fn tariffs<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> TariffsSender<'a> {
        TariffsSender::new(self.module(transport, ModuleId::Tariffs, from))
    }

    /// The Tariffs Receiver client: push Tariffs to this peer.
    #[must_use]
    pub fn tariffs_receiver<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> TariffsReceiver<'a> {
        TariffsReceiver::new(self.module(transport, ModuleId::Tariffs, from))
    }

    /// The Commands client.
    #[must_use]
    pub fn commands<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> CommandsClient<'a> {
        CommandsClient::new(self.module(transport, ModuleId::Commands, from))
    }

    /// The Charging Profiles client.
    #[must_use]
    pub fn charging_profiles<'a>(
        &'a self,
        transport: &'a Transport,
        from: PartyRef,
    ) -> ChargingProfilesClient<'a> {
        ChargingProfilesClient::new(self.module(transport, ModuleId::ChargingProfiles, from))
    }

    /// The Hub Client Info client.
    #[must_use]
    pub fn hub_client_info<'a>(
        &'a self,
        transport: &'a Transport,
        from: PartyRef,
    ) -> HubClientInfoClient<'a> {
        HubClientInfoClient::new(self.module(transport, ModuleId::HubClientInfo, from))
    }

    /// The Payments client.
    #[must_use]
    pub fn payments<'a>(&'a self, transport: &'a Transport, from: PartyRef) -> PaymentsClient<'a> {
        PaymentsClient::new(self.module(transport, ModuleId::Payments, from))
    }
}

/// Turns a translation failure into the OCPI error a caller can act on.
fn bridge_error(error: BridgeError, kind: ObjectKind) -> OcpiError {
    match error {
        BridgeError::Unsupported { from, to } => OcpiError::Unsupported(format!(
            "this build has no conversions between OCPI {from} and OCPI {to}, so a {kind} cannot \
             be carried between them"
        )),
        BridgeError::Decode { version, message, .. } => OcpiError::Decode {
            path: "/".to_owned(),
            message: format!("the peer's OCPI {version} {kind} could not be read: {message}"),
        },
    }
}

/// Decodes a translated document into the canonical type it is now written as.
fn decode<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, OcpiError> {
    serde_path_to_error::deserialize(value)
        .map_err(|e| OcpiError::Decode { path: e.path().to_string(), message: e.into_inner().to_string() })
}

/// Convenience: `RequestIds` for a caller that wants to correlate several requests.
#[must_use]
pub fn correlated_ids() -> RequestIds {
    RequestIds::generate()
}
