//! The router: mount the interfaces this party serves, get an `axum::Router`.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post, put};

use crate::transport::{OcpiError, Page, PageMeta, Quirks};
use crate::types::{PartyRef, Url};
use crate::{InterfaceRole, ModuleId, VersionNumber};

use super::auth::{MountedModules, TokenStore};
use super::error::{OcpiErrorResponse, OcpiReply};
use super::extract::{
    Auth, AuthState, ContentTypePolicy, Ids, OcpiJson, OcpiPatch, Owner, Page as PageParams, RequestContext,
    Routing, accepts_json,
};
use super::traits::{
    CdrsReceiver, CdrsSender, ChargingProfilesReceiver, ChargingProfilesSender, CommandsReceiver,
    CommandsSender, CredentialsHandler, HubClientInfoReceiver, HubClientInfoSender, LocationsReceiver,
    LocationsSender, PaymentsReceiver, PaymentsSender, SessionsReceiver, SessionsSender, TariffsReceiver,
    TariffsSender, TokensReceiver, TokensSender,
};

/// How the server behaves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ServerConfig {
    /// The interoperability profile for incoming requests, chiefly whether an unencoded
    /// `Authorization` token is accepted.
    pub quirks: Quirks,
    /// Whether objects are validated before being returned. Defaults to `true`.
    pub validate_outgoing: bool,
    /// The largest page a list endpoint will return, which is what `X-Limit` advertises.
    ///
    /// > *`X-Limit`: The maximum number of objects that the server can return.*
    pub max_page_limit: u64,
    /// A path segment inserted before every **Receiver** interface, e.g. `receiver`.
    ///
    /// # Why this exists
    ///
    /// The Locations Sender and Receiver interfaces have structurally identical URLs:
    ///
    /// ```text
    /// Sender:   {locations}/{location_id}/{evse_uid}/{connector_id}
    /// Receiver: {locations}/{country_code}/{party_id}/{location_id}
    /// ```
    ///
    /// Three path segments either way, so no router can tell them apart. The specification does
    /// not have this problem because the two are *different endpoints* with independently chosen
    /// URLs — in practice `/ocpi/cpo/2.3.0/locations` and `/ocpi/emsp/2.3.0/locations` — and
    /// *"The exact URL can be found by fetching the endpoint information from the API info
    /// endpoint"*.
    ///
    /// A platform that is both CPO and eMSP therefore has two choices, and this field picks
    /// between them:
    ///
    /// * `Some("receiver")` (the default) — one router, one `/versions`, and the Receiver
    ///   interfaces published one segment deeper. The generated version details say so, which is
    ///   the whole point of generating them.
    /// * `None` — the conventional split, with one [`OcpiRouter`] per role nested under its own
    ///   base URL. Mounting both interfaces of the Locations module on one router with no prefix
    ///   is a configuration error and panics at start-up with an explanation.
    ///
    /// Spec: 2.3.0 §transport_and_format_interface_endpoints
    pub receiver_path_prefix: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            quirks: Quirks::default(),
            validate_outgoing: true,
            max_page_limit: 100,
            receiver_path_prefix: Some("receiver".to_owned()),
        }
    }
}

/// Everything the extractors and handlers share.
pub struct OcpiState {
    tokens: Arc<dyn TokenStore>,
    config: ServerConfig,
    base_url: Url,
    mounted: MountedModules,
    version: VersionNumber,
}

impl OcpiState {
    /// The configuration in use.
    #[must_use]
    pub const fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// The base URL this server's endpoints are published under.
    #[must_use]
    pub const fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// The modules and interfaces that were mounted.
    #[must_use]
    pub const fn mounted(&self) -> &MountedModules {
        &self.mounted
    }

    /// The version details this server advertises, generated from what was mounted.
    #[must_use]
    pub fn version_details(&self) -> crate::v2_3_0::versions::VersionDetails {
        let endpoints = self
            .mounted
            .all()
            .iter()
            .map(|(module, role)| {
                let mut url = self.base_url.clone();
                if *role == InterfaceRole::Receiver
                    && let Some(prefix) = &self.config.receiver_path_prefix
                {
                    url = url.join(prefix);
                }
                crate::v2_3_0::versions::Endpoint::new(module.clone(), *role, url.join(module.as_str()))
            })
            .collect();
        crate::v2_3_0::versions::VersionDetails::new(self.version.clone(), endpoints)
    }
}

impl AuthState for Arc<OcpiState> {
    fn tokens(&self) -> &dyn TokenStore {
        self.tokens.as_ref()
    }
    fn quirks(&self) -> &Quirks {
        &self.config.quirks
    }
}

impl ContentTypePolicy for Arc<OcpiState> {
    fn accepts_content_type(&self, headers: &http::HeaderMap) -> bool {
        accepts_json(headers, self.config.quirks.lenient_content_type)
    }
}

/// Builds the `axum::Router` that serves one OCPI version.
///
/// ```no_run
/// use std::sync::Arc;
/// use ocpi_kit::server::{InMemoryTokenStore, OcpiRouter};
/// use ocpi_kit::types::Url;
/// use ocpi_kit::VersionNumber;
///
/// # fn build(locations: impl ocpi_kit::server::LocationsSender,
/// #          credentials: impl ocpi_kit::server::CredentialsHandler)
/// # -> Result<axum::Router, Box<dyn std::error::Error>> {
/// let router = OcpiRouter::new(
///         VersionNumber::V2_3_0,
///         Url::new("https://cpo.example.com/ocpi/cpo/2.3.0")?,
///         Arc::new(InMemoryTokenStore::new()),
///     )
///     .credentials(credentials)
///     .locations_sender(locations)
///     .build();
/// # Ok(router)
/// # }
/// ```
///
/// The version details endpoint is generated from exactly what was mounted, so a peer's
/// discovery can never disagree with what this server actually serves.
pub struct OcpiRouter {
    router: Router<Arc<OcpiState>>,
    tokens: Arc<dyn TokenStore>,
    config: ServerConfig,
    base_url: Url,
    mounted: MountedModules,
    version: VersionNumber,
}

impl OcpiRouter {
    /// The prefix Receiver routes are mounted under, captured before the router is moved.
    fn receiver_prefix(&self) -> Option<String> {
        self.config.receiver_path_prefix.clone()
    }

    /// Refuses, with an explanation, a combination no router can serve.
    ///
    /// # Panics
    ///
    /// Panics when both interfaces of the Locations module are mounted on one router with no
    /// [`ServerConfig::receiver_path_prefix`]; their URLs would be indistinguishable. Better a
    /// clear message at start-up than an opaque routing conflict.
    fn check_receiver_conflict(&self, module: &ModuleId) {
        let ambiguous =
            matches!(module, ModuleId::Locations | ModuleId::ChargingProfiles | ModuleId::Payments);
        if ambiguous
            && self.config.receiver_path_prefix.is_none()
            && self.mounted.contains(module, InterfaceRole::Sender)
        {
            panic!(
                "cannot mount both interfaces of the {module} module on one router with no \
                 receiver path prefix: the Sender and Receiver URLs have the same shape, so no \
                 route ordering can tell them apart. Either set \
                 ServerConfig::receiver_path_prefix, or build one OcpiRouter per interface role \
                 and nest them under different base URLs."
            );
        }
    }

    /// Starts a router for one OCPI version, published under `base_url`.
    #[must_use]
    pub fn new(version: VersionNumber, base_url: Url, tokens: Arc<dyn TokenStore>) -> Self {
        Self {
            router: Router::new(),
            tokens,
            config: ServerConfig::default(),
            base_url,
            mounted: MountedModules::new(),
            version,
        }
    }

    /// Overrides the server configuration.
    #[must_use]
    pub fn with_config(mut self, config: ServerConfig) -> Self {
        self.config = config;
        self
    }

    /// Mounts the credentials module, which every implementation must serve.
    #[must_use]
    pub fn credentials<H: CredentialsHandler>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Credentials, InterfaceRole::Sender);
        let get_handler = Arc::clone(&handler);
        let post_handler = Arc::clone(&handler);
        let put_handler = Arc::clone(&handler);
        let delete_handler = handler;
        self.router = self.router.route(
            "/credentials",
            get(async move |auth: Auth, ids: Ids| -> Result<_, OcpiErrorResponse> {
                let context = context_of(auth, ids, Routing(None), &ModuleId::Credentials)?;
                let ids = context.ids.clone();
                get_handler
                    .get(context)
                    .await
                    .map(|c| OcpiReply::ok(c).with_ids(ids.clone()))
                    .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
            })
            .post(
                async move |auth: Auth,
                            ids: Ids,
                            OcpiJson(body): OcpiJson<crate::v2_3_0::credentials::Credentials>|
                            -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, Routing(None), &ModuleId::Credentials)?;
                    let ids = context.ids.clone();
                    post_handler
                        .post(body, context)
                        .await
                        .map(|c| OcpiReply::ok(c).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            )
            .put(
                async move |auth: Auth,
                            ids: Ids,
                            OcpiJson(body): OcpiJson<crate::v2_3_0::credentials::Credentials>|
                            -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, Routing(None), &ModuleId::Credentials)?;
                    let ids = context.ids.clone();
                    put_handler
                        .put(body, context)
                        .await
                        .map(|c| OcpiReply::ok(c).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            )
            .delete(async move |auth: Auth, ids: Ids| -> Result<_, OcpiErrorResponse> {
                let context = context_of(auth, ids, Routing(None), &ModuleId::Credentials)?;
                let ids = context.ids.clone();
                delete_handler
                    .delete(context)
                    .await
                    .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                    .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
            }),
        );
        self
    }

    /// Mounts the Locations Sender interface: `GET` on locations, EVSEs and connectors.
    #[must_use]
    pub fn locations_sender<H: LocationsSender>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Locations, InterfaceRole::Sender);

        let list = Arc::clone(&handler);
        let one = Arc::clone(&handler);
        let evse = Arc::clone(&handler);
        let connector = handler;

        self.router = self
            .router
            .route(
                "/locations",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                PageParams(query): PageParams,
                                State(state): State<Arc<OcpiState>>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Locations)?;
                        let ids = context.ids.clone();
                        let responder = context.addressed_to().cloned();
                        let response_routing = responder.as_ref().and_then(|r| context.response_routing(r));
                        list.list(query, context)
                            .await
                            .map(|page| page_reply(page, &state))
                            .map(|reply| {
                                let reply = reply.with_ids(ids.clone());
                                match response_routing {
                                    Some(r) => reply.with_routing(r),
                                    None => reply,
                                }
                            })
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/locations/{location_id}",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(location_id): Path<String>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Locations)?;
                        let ids = context.ids.clone();
                        one.location(location_id, context)
                            .await
                            .map(|l| OcpiReply::ok(l).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/locations/{location_id}/{evse_uid}",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path((location_id, evse_uid)): Path<(String, String)>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Locations)?;
                        let ids = context.ids.clone();
                        evse.evse(location_id, evse_uid, context)
                            .await
                            .map(|e| OcpiReply::ok(e).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/locations/{location_id}/{evse_uid}/{connector_id}",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path((location_id, evse_uid, connector_id)): Path<(
                        String,
                        String,
                        String,
                    )>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Locations)?;
                        let ids = context.ids.clone();
                        connector
                            .connector(location_id, evse_uid, connector_id, context)
                            .await
                            .map(|c| OcpiReply::ok(c).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            );
        self
    }

    /// Mounts the Locations Receiver interface: the client-owned-object `GET`, `PUT` and `PATCH`.
    #[must_use]
    pub fn locations_receiver<H: LocationsReceiver>(mut self, handler: H) -> Self {
        let prefix = self.receiver_prefix();
        self.check_receiver_conflict(&ModuleId::Locations);
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Locations, InterfaceRole::Receiver);

        let get_one = Arc::clone(&handler);
        let put_location = Arc::clone(&handler);
        let put_evse = Arc::clone(&handler);
        let patch_any = handler;

        self.router = self
            .router
            .route(
                &receiver_path(prefix.as_deref(), "/locations/{country_code}/{party_id}/{location_id}"),
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path((country_code, party_id, location_id)): Path<(
                        String,
                        String,
                        String,
                    )>|
                                -> Result<_, OcpiErrorResponse> {
                        let (context, owner) = owned_context(
                            auth,
                            ids,
                            routing,
                            &ModuleId::Locations,
                            &country_code,
                            &party_id,
                        )?;
                        let ids = context.ids.clone();
                        get_one
                            .location(owner, location_id, context)
                            .await
                            .map(|l| OcpiReply::ok(l).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                )
                .put(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path((country_code, party_id, location_id)): Path<(
                        String,
                        String,
                        String,
                    )>,
                                OcpiJson(location): OcpiJson<crate::v2_3_0::locations::Location>|
                                -> Result<_, OcpiErrorResponse> {
                        let (context, owner) = owned_context(
                            auth,
                            ids,
                            routing,
                            &ModuleId::Locations,
                            &country_code,
                            &party_id,
                        )?;
                        let ids = context.ids.clone();
                        // "server implementations are advised to return 2001 when the {object-id}
                        //  in the URL is different from the id in the object being pushed"
                        if !location.id.eq_ignore_case(&location_id) {
                            return Err(OcpiErrorResponse::new(OcpiError::Decode {
                                path: "/id".to_owned(),
                                message: format!(
                                    "the object id {:?} does not match the {location_id:?} in the URL",
                                    location.id.as_str()
                                ),
                            })
                            .with_ids(ids));
                        }
                        put_location
                            .put_location(owner, location, context)
                            .await
                            .map(|created| {
                                OcpiReply::<()>::no_content()
                                    .with_http_status(status_of(created))
                                    .with_ids(ids.clone())
                            })
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                )
                .patch(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path((country_code, party_id, location_id)): Path<(
                        String,
                        String,
                        String,
                    )>,
                                OcpiPatch(patch): OcpiPatch<serde_json::Value>|
                                -> Result<_, OcpiErrorResponse> {
                        let (context, owner) = owned_context(
                            auth,
                            ids,
                            routing,
                            &ModuleId::Locations,
                            &country_code,
                            &party_id,
                        )?;
                        let ids = context.ids.clone();
                        patch_any
                            .patch(owner, location_id, None, None, patch, context)
                            .await
                            .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                &receiver_path(
                    prefix.as_deref(),
                    "/locations/{country_code}/{party_id}/{location_id}/{evse_uid}",
                ),
                put(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path((country_code, party_id, location_id, _evse_uid)): Path<(
                        String,
                        String,
                        String,
                        String,
                    )>,
                                OcpiJson(evse): OcpiJson<crate::v2_3_0::locations::Evse>|
                                -> Result<_, OcpiErrorResponse> {
                        let (context, owner) = owned_context(
                            auth,
                            ids,
                            routing,
                            &ModuleId::Locations,
                            &country_code,
                            &party_id,
                        )?;
                        let ids = context.ids.clone();
                        put_evse
                            .put_evse(owner, location_id, evse, context)
                            .await
                            .map(|created| {
                                OcpiReply::<()>::no_content()
                                    .with_http_status(status_of(created))
                                    .with_ids(ids.clone())
                            })
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            );
        self
    }

    /// Mounts the Tokens Sender interface: `GET` list and real-time authorization.
    #[must_use]
    pub fn tokens_sender<H: TokensSender>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Tokens, InterfaceRole::Sender);
        let list = Arc::clone(&handler);
        let authorize = handler;

        self.router = self
            .router
            .route(
                "/tokens",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                PageParams(query): PageParams,
                                State(state): State<Arc<OcpiState>>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Tokens)?;
                        let ids = context.ids.clone();
                        list.list(query, context)
                            .await
                            .map(|page| page_reply(page, &state).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/tokens/{token_uid}/authorize",
                post(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(token_uid): Path<String>,
                                Query(params): Query<TokenTypeQuery>,
                                body: axum::body::Bytes|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Tokens)?;
                        let ids = context.ids.clone();
                        // "LocationReferences: Location and EVSEs for which the driver wants to
                        //  charge" — the body is optional, so an empty or `{}` body means
                        //  "anywhere".
                        let location =
                            decode_optional_body::<crate::v2_3_0::tokens::LocationReferences>(&body)
                                .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids.clone()))?;
                        authorize
                            .authorize(token_uid, params.token_type, location, context)
                            .await
                            .map(|info| OcpiReply::ok(info).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            );
        self
    }

    /// Mounts the Tokens Receiver interface.
    #[must_use]
    pub fn tokens_receiver<H: TokensReceiver>(mut self, handler: H) -> Self {
        let prefix = self.receiver_prefix();
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Tokens, InterfaceRole::Receiver);
        let get_one = Arc::clone(&handler);
        let put_one = Arc::clone(&handler);
        let patch_one = handler;

        self.router = self.router.route(
            &receiver_path(prefix.as_deref(), "/tokens/{country_code}/{party_id}/{token_uid}"),
            get(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((country_code, party_id, token_uid)): Path<(String, String, String)>,
                            Query(params): Query<TokenTypeQuery>|
                            -> Result<_, OcpiErrorResponse> {
                    let (context, owner) =
                        owned_context(auth, ids, routing, &ModuleId::Tokens, &country_code, &party_id)?;
                    let ids = context.ids.clone();
                    get_one
                        .token(owner, token_uid, params.token_type, context)
                        .await
                        .map(|t| OcpiReply::ok(t).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            )
            .put(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((country_code, party_id, _token_uid)): Path<(String, String, String)>,
                            OcpiJson(token): OcpiJson<crate::v2_3_0::tokens::Token>|
                            -> Result<_, OcpiErrorResponse> {
                    let (context, owner) =
                        owned_context(auth, ids, routing, &ModuleId::Tokens, &country_code, &party_id)?;
                    let ids = context.ids.clone();
                    put_one
                        .put_token(owner, token, context)
                        .await
                        .map(|created| {
                            OcpiReply::<()>::no_content()
                                .with_http_status(status_of(created))
                                .with_ids(ids.clone())
                        })
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            )
            .patch(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((country_code, party_id, token_uid)): Path<(String, String, String)>,
                            Query(params): Query<TokenTypeQuery>,
                            OcpiPatch(patch): OcpiPatch<crate::v2_3_0::tokens::Token>|
                            -> Result<_, OcpiErrorResponse> {
                    let (context, owner) =
                        owned_context(auth, ids, routing, &ModuleId::Tokens, &country_code, &party_id)?;
                    let ids = context.ids.clone();
                    patch_one
                        .patch_token(owner, token_uid, params.token_type, patch, context)
                        .await
                        .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            ),
        );
        self
    }

    /// Mounts the CDRs Sender interface.
    #[must_use]
    pub fn cdrs_sender<H: CdrsSender>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Cdrs, InterfaceRole::Sender);
        self.router = self.router.route(
            "/cdrs",
            get(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            PageParams(query): PageParams,
                            State(state): State<Arc<OcpiState>>|
                            -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, routing, &ModuleId::Cdrs)?;
                    let ids = context.ids.clone();
                    handler
                        .list(query, context)
                        .await
                        .map(|page| page_reply(page, &state).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            ),
        );
        self
    }

    /// Mounts the CDRs Receiver interface, including the `POST` that returns a `Location` header.
    #[must_use]
    pub fn cdrs_receiver<H: CdrsReceiver>(mut self, handler: H) -> Self {
        let prefix = self.receiver_prefix();
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Cdrs, InterfaceRole::Receiver);
        let get_one = Arc::clone(&handler);
        let post_one = handler;

        self.router = self
            .router
            .route(
                &receiver_path(prefix.as_deref(), "/cdrs"),
                post(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                OcpiJson(cdr): OcpiJson<crate::v2_3_0::cdrs::Cdr>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Cdrs)?;
                        let ids = context.ids.clone();
                        post_one
                            .post_cdr(cdr, context)
                            .await
                            .map(|location| {
                                let mut headers = http::HeaderMap::new();
                                if let Ok(value) = http::HeaderValue::from_str(location.as_str()) {
                                    headers.insert(crate::transport::headers::LOCATION, value);
                                }
                                OcpiReply::<()>::no_content()
                                    .with_http_status(http::StatusCode::CREATED)
                                    .with_headers(headers)
                                    .with_ids(ids.clone())
                            })
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                &receiver_path(prefix.as_deref(), "/cdrs/{cdr_id}"),
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(cdr_id): Path<String>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Cdrs)?;
                        let ids = context.ids.clone();
                        get_one
                            .cdr(cdr_id, context)
                            .await
                            .map(|c| OcpiReply::ok(c).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            );
        self
    }

    /// Mounts the Sessions Sender interface, including `PUT charging_preferences`.
    #[must_use]
    pub fn sessions_sender<H: SessionsSender>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Sessions, InterfaceRole::Sender);
        let list = Arc::clone(&handler);
        let preferences = handler;

        self.router = self
            .router
            .route(
                "/sessions",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                PageParams(query): PageParams,
                                State(state): State<Arc<OcpiState>>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Sessions)?;
                        let ids = context.ids.clone();
                        list.list(query, context)
                            .await
                            .map(|page| page_reply(page, &state).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/sessions/{session_id}/charging_preferences",
                put(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(session_id): Path<String>,
                                OcpiJson(body): OcpiJson<crate::v2_3_0::sessions::ChargingPreferences>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Sessions)?;
                        let ids = context.ids.clone();
                        preferences
                            .set_charging_preferences(session_id, body, context)
                            .await
                            .map(|r| OcpiReply::ok(r).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            );
        self
    }

    /// Mounts the Sessions Receiver interface.
    #[must_use]
    pub fn sessions_receiver<H: SessionsReceiver>(mut self, handler: H) -> Self {
        let prefix = self.receiver_prefix();
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Sessions, InterfaceRole::Receiver);
        let get_one = Arc::clone(&handler);
        let put_one = Arc::clone(&handler);
        let patch_one = handler;

        self.router = self.router.route(
            &receiver_path(prefix.as_deref(), "/sessions/{country_code}/{party_id}/{session_id}"),
            get(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((country_code, party_id, session_id)): Path<(String, String, String)>|
                            -> Result<_, OcpiErrorResponse> {
                    let (context, owner) =
                        owned_context(auth, ids, routing, &ModuleId::Sessions, &country_code, &party_id)?;
                    let ids = context.ids.clone();
                    get_one
                        .session(owner, session_id, context)
                        .await
                        .map(|s| OcpiReply::ok(s).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            )
            .put(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((country_code, party_id, _session_id)): Path<(String, String, String)>,
                            OcpiJson(session): OcpiJson<crate::v2_3_0::sessions::Session>|
                            -> Result<_, OcpiErrorResponse> {
                    let (context, owner) =
                        owned_context(auth, ids, routing, &ModuleId::Sessions, &country_code, &party_id)?;
                    let ids = context.ids.clone();
                    put_one
                        .put_session(owner, session, context)
                        .await
                        .map(|created| {
                            OcpiReply::<()>::no_content()
                                .with_http_status(status_of(created))
                                .with_ids(ids.clone())
                        })
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            )
            .patch(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((country_code, party_id, session_id)): Path<(String, String, String)>,
                            OcpiPatch(patch): OcpiPatch<crate::v2_3_0::sessions::Session>|
                            -> Result<_, OcpiErrorResponse> {
                    let (context, owner) =
                        owned_context(auth, ids, routing, &ModuleId::Sessions, &country_code, &party_id)?;
                    let ids = context.ids.clone();
                    patch_one
                        .patch_session(owner, session_id, patch, context)
                        .await
                        .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            ),
        );
        self
    }

    /// Mounts the Tariffs Sender interface.
    #[must_use]
    pub fn tariffs_sender<H: TariffsSender>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Tariffs, InterfaceRole::Sender);
        self.router = self.router.route(
            "/tariffs",
            get(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            PageParams(query): PageParams,
                            State(state): State<Arc<OcpiState>>|
                            -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, routing, &ModuleId::Tariffs)?;
                    let ids = context.ids.clone();
                    handler
                        .list(query, context)
                        .await
                        .map(|page| page_reply(page, &state).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            ),
        );
        self
    }

    /// Mounts the Tariffs Receiver interface, including the `DELETE`.
    #[must_use]
    pub fn tariffs_receiver<H: TariffsReceiver>(mut self, handler: H) -> Self {
        let prefix = self.receiver_prefix();
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Tariffs, InterfaceRole::Receiver);
        let get_one = Arc::clone(&handler);
        let put_one = Arc::clone(&handler);
        let delete_one = handler;

        self.router = self.router.route(
            &receiver_path(prefix.as_deref(), "/tariffs/{country_code}/{party_id}/{tariff_id}"),
            get(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((country_code, party_id, tariff_id)): Path<(String, String, String)>|
                            -> Result<_, OcpiErrorResponse> {
                    let (context, owner) =
                        owned_context(auth, ids, routing, &ModuleId::Tariffs, &country_code, &party_id)?;
                    let ids = context.ids.clone();
                    get_one
                        .tariff(owner, tariff_id, context)
                        .await
                        .map(|t| OcpiReply::ok(t).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            )
            .put(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((country_code, party_id, _tariff_id)): Path<(String, String, String)>,
                            OcpiJson(tariff): OcpiJson<crate::v2_3_0::tariffs::Tariff>|
                            -> Result<_, OcpiErrorResponse> {
                    let (context, owner) =
                        owned_context(auth, ids, routing, &ModuleId::Tariffs, &country_code, &party_id)?;
                    let ids = context.ids.clone();
                    put_one
                        .put_tariff(owner, tariff, context)
                        .await
                        .map(|created| {
                            OcpiReply::<()>::no_content()
                                .with_http_status(status_of(created))
                                .with_ids(ids.clone())
                        })
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            ),
        );
        self.router = self.router.route(
            &receiver_path(prefix.as_deref(), "/tariffs/{country_code}/{party_id}/{tariff_id}/"),
            delete(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((country_code, party_id, tariff_id)): Path<(String, String, String)>|
                            -> Result<_, OcpiErrorResponse> {
                    let (context, owner) =
                        owned_context(auth, ids, routing, &ModuleId::Tariffs, &country_code, &party_id)?;
                    let ids = context.ids.clone();
                    delete_one
                        .delete_tariff(owner, tariff_id, context)
                        .await
                        .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            ),
        );
        self
    }

    /// Mounts the Commands Receiver interface: `POST {commands}/{command}`.
    #[must_use]
    pub fn commands_receiver<H: CommandsReceiver>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Commands, InterfaceRole::Receiver);
        self.router = self.router.route(
            "/commands/{command}",
            post(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path(command_name): Path<String>,
                            body: axum::body::Bytes|
                            -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, routing, &ModuleId::Commands)?;
                    let ids = context.ids.clone();
                    let command = parse_command(&command_name, &body)
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids.clone()))?;
                    handler
                        .command(command, context)
                        .await
                        .map(|r| OcpiReply::ok(r).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            ),
        );
        self
    }

    /// Mounts the Hub Client Info Sender interface: `GET {hubclientinfo}`.
    ///
    /// A configuration module, so its requests carry no routing headers.
    #[must_use]
    pub fn hub_client_info_sender<H: HubClientInfoSender>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::HubClientInfo, InterfaceRole::Sender);
        self.router = self.router.route(
            "/hubclientinfo",
            get(
                async move |auth: Auth,
                            ids: Ids,
                            PageParams(query): PageParams,
                            State(state): State<Arc<OcpiState>>|
                            -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, Routing(None), &ModuleId::HubClientInfo)?;
                    let ids = context.ids.clone();
                    handler
                        .list(query, context)
                        .await
                        .map(|page| page_reply(page, &state).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            ),
        );
        self
    }

    /// Mounts the Hub Client Info Receiver interface: `GET`/`PUT {hubclientinfo}/{cc}/{party}`.
    ///
    /// The party in the URL is the party the `ClientInfo` is **about**, not the party that owns
    /// the object — a hub pushes information about every other party to each of its clients — so
    /// the ownership rule that guards the other client-owned-object URLs deliberately does not
    /// apply here.
    #[must_use]
    pub fn hub_client_info_receiver<H: HubClientInfoReceiver>(mut self, handler: H) -> Self {
        let prefix = self.receiver_prefix();
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::HubClientInfo, InterfaceRole::Receiver);
        let get_one = Arc::clone(&handler);
        let put_one = handler;

        self.router = self.router.route(
            &receiver_path(prefix.as_deref(), "/hubclientinfo/{country_code}/{party_id}"),
            get(
                async move |auth: Auth,
                            ids: Ids,
                            Path((country_code, party_id)): Path<(String, String)>|
                            -> Result<_, OcpiErrorResponse> {
                    let request_ids = ids.0.clone();
                    let context = context_of(auth, ids, Routing(None), &ModuleId::HubClientInfo)?;
                    let Owner(party) = Owner::from_path(&country_code, &party_id)
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids.clone()))?;
                    get_one
                        .client_info(party, context)
                        .await
                        .map(|info| OcpiReply::ok(info).with_ids(request_ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids))
                },
            )
            .put(
                async move |auth: Auth,
                            ids: Ids,
                            Path((country_code, party_id)): Path<(String, String)>,
                            OcpiJson(info): OcpiJson<crate::v2_3_0::hub_client_info::ClientInfo>|
                            -> Result<_, OcpiErrorResponse> {
                    let request_ids = ids.0.clone();
                    let context = context_of(auth, ids, Routing(None), &ModuleId::HubClientInfo)?;
                    let Owner(party) = Owner::from_path(&country_code, &party_id)
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids.clone()))?;
                    put_one
                        .put_client_info(party, info, context)
                        .await
                        .map(|created| {
                            OcpiReply::<()>::no_content()
                                .with_http_status(status_of(created))
                                .with_ids(request_ids.clone())
                        })
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids))
                },
            ),
        );
        self
    }

    /// Mounts the Commands Sender interface: the callback the Receiver POSTs its result to.
    ///
    /// The path is `{commands}/{command}/{unique_id}`, which is the shape the specification's own
    /// example uses (`.../commands/RESERVE_NOW/1234`). Build the matching `response_url` with
    /// [`CallbackUrls::command_result`] so the two can never drift apart.
    ///
    /// > *This URL might contain a unique ID to be able to distinguish between StartSession
    /// > requests.*
    #[must_use]
    pub fn commands_sender<H: CommandsSender>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Commands, InterfaceRole::Sender);
        self.router = self.router.route(
            "/commands/{command}/{unique_id}",
            post(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path((_command, unique_id)): Path<(String, String)>,
                            OcpiJson(result): OcpiJson<crate::v2_3_0::commands::CommandResult>|
                            -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, routing, &ModuleId::Commands)?;
                    let ids = context.ids.clone();
                    handler
                        .command_result(unique_id, result, context)
                        .await
                        .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            ),
        );
        self
    }

    /// Mounts the Charging Profiles Receiver interface: `GET`/`PUT`/`DELETE` on a session.
    #[must_use]
    pub fn charging_profiles_receiver<H: ChargingProfilesReceiver>(mut self, handler: H) -> Self {
        self.check_receiver_conflict(&ModuleId::ChargingProfiles);
        let prefix = self.receiver_prefix();
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::ChargingProfiles, InterfaceRole::Receiver);
        let get_one = Arc::clone(&handler);
        let put_one = Arc::clone(&handler);
        let delete_one = handler;

        self.router = self.router.route(
            &receiver_path(prefix.as_deref(), "/chargingprofiles/{session_id}"),
            get(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path(session_id): Path<String>,
                            Query(q): Query<ActiveProfileQuery>|
                            -> Result<_, OcpiErrorResponse> {
                    let request_ids = ids.0.clone();
                    let context = context_of(auth, ids, routing, &ModuleId::ChargingProfiles)?;
                    let response_url = q
                        .response_url
                        .ok_or_else(|| missing_query("response_url"))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids.clone()))?;
                    get_one
                        .active_charging_profile(session_id, q.duration.unwrap_or(0), response_url, context)
                        .await
                        .map(|r| OcpiReply::ok(r).with_ids(request_ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids))
                },
            )
            .put(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path(session_id): Path<String>,
                            OcpiJson(request): OcpiJson<
                    crate::v2_3_0::charging_profiles::SetChargingProfile,
                >|
                            -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, routing, &ModuleId::ChargingProfiles)?;
                    let ids = context.ids.clone();
                    put_one
                        .set_charging_profile(session_id, request, context)
                        .await
                        .map(|r| OcpiReply::ok(r).with_ids(ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                },
            )
            .delete(
                async move |auth: Auth,
                            ids: Ids,
                            routing: Routing,
                            Path(session_id): Path<String>,
                            Query(q): Query<ActiveProfileQuery>|
                            -> Result<_, OcpiErrorResponse> {
                    let request_ids = ids.0.clone();
                    let context = context_of(auth, ids, routing, &ModuleId::ChargingProfiles)?;
                    let response_url = q
                        .response_url
                        .ok_or_else(|| missing_query("response_url"))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids.clone()))?;
                    delete_one
                        .clear_charging_profile(session_id, response_url, context)
                        .await
                        .map(|r| OcpiReply::ok(r).with_ids(request_ids.clone()))
                        .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids))
                },
            ),
        );
        self
    }

    /// Mounts the Charging Profiles Sender interface: the three result callbacks and the
    /// CPO-initiated `PUT` of a changed active profile.
    ///
    /// See [`ChargingProfilesSender`] for why there are three callback paths rather than one, and
    /// [`CallbackUrls`] for building the `response_url`s that reach them.
    #[must_use]
    pub fn charging_profiles_sender<H: ChargingProfilesSender>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::ChargingProfiles, InterfaceRole::Sender);
        let active = Arc::clone(&handler);
        let set = Arc::clone(&handler);
        let clear = Arc::clone(&handler);
        let pushed = handler;

        self.router = self
            .router
            .route(
                "/chargingprofiles/result/active/{unique_id}",
                post(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(unique_id): Path<String>,
                                OcpiJson(result): OcpiJson<
                        crate::v2_3_0::charging_profiles::ActiveChargingProfileResult,
                    >|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::ChargingProfiles)?;
                        let ids = context.ids.clone();
                        active
                            .active_charging_profile_result(unique_id, result, context)
                            .await
                            .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/chargingprofiles/result/set/{unique_id}",
                post(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(unique_id): Path<String>,
                                OcpiJson(result): OcpiJson<
                        crate::v2_3_0::charging_profiles::ChargingProfileResult,
                    >|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::ChargingProfiles)?;
                        let ids = context.ids.clone();
                        set.charging_profile_result(unique_id, result, context)
                            .await
                            .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/chargingprofiles/result/clear/{unique_id}",
                post(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(unique_id): Path<String>,
                                OcpiJson(result): OcpiJson<
                        crate::v2_3_0::charging_profiles::ClearProfileResult,
                    >|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::ChargingProfiles)?;
                        let ids = context.ids.clone();
                        clear
                            .clear_profile_result(unique_id, result, context)
                            .await
                            .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/chargingprofiles/{session_id}",
                put(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(session_id): Path<String>,
                                OcpiJson(profile): OcpiJson<
                        crate::v2_3_0::charging_profiles::ActiveChargingProfile,
                    >|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::ChargingProfiles)?;
                        let ids = context.ids.clone();
                        pushed
                            .put_active_charging_profile(session_id, profile, context)
                            .await
                            .map(|()| OcpiReply::<()>::no_content().with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            );
        self
    }

    /// Mounts the Payments Sender interface: the terminals and financial advice this PTP owns.
    #[must_use]
    pub fn payments_sender<H: PaymentsSender>(mut self, handler: H) -> Self {
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Payments, InterfaceRole::Sender);
        let list = Arc::clone(&handler);
        let one = Arc::clone(&handler);
        let put_one = Arc::clone(&handler);
        let patch_one = Arc::clone(&handler);
        let activate = Arc::clone(&handler);
        let deactivate = Arc::clone(&handler);
        let fac_list = Arc::clone(&handler);
        let fac_one = handler;

        self.router = self
            .router
            .route(
                "/payments/terminals",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                PageParams(query): PageParams,
                                State(state): State<Arc<OcpiState>>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        list.terminals(query, context)
                            .await
                            .map(|page| page_reply(page, &state).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            // A static segment beats a capture at the same position, so this stays reachable
            // even though `/payments/terminals/{terminal_id}` also matches it.
            .route(
                "/payments/terminals/activate",
                post(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                body: axum::body::Bytes|
                                -> Result<_, OcpiErrorResponse> {
                        let request_ids = ids.0.clone();
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        // Deliberately not `OcpiPatch`: this is a POST carrying a partial
                        // object, not an RFC 7396 merge patch, so the rule that a PATCH must
                        // carry `last_updated` does not apply to it.
                        let terminal = partial_object(&body)
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids.clone()))?;
                        activate
                            .activate_terminal(terminal, context)
                            .await
                            .map(|t| OcpiReply::ok(t).with_ids(request_ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids))
                    },
                ),
            )
            .route(
                "/payments/terminals/{terminal_id}",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(terminal_id): Path<String>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        one.terminal(terminal_id, context)
                            .await
                            .map(|t| OcpiReply::ok(t).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                )
                .put(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(terminal_id): Path<String>,
                                OcpiJson(terminal): OcpiJson<crate::v2_3_0::payments::Terminal>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        put_one
                            .put_terminal(terminal_id, terminal, context)
                            .await
                            .map(|t| OcpiReply::ok(t).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                )
                .patch(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(terminal_id): Path<String>,
                                OcpiPatch(patch): OcpiPatch<crate::v2_3_0::payments::Terminal>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        patch_one
                            .patch_terminal(terminal_id, patch, context)
                            .await
                            .map(|t| OcpiReply::ok(t).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/payments/terminals/{terminal_id}/deactivate",
                post(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(terminal_id): Path<String>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        deactivate
                            .deactivate_terminal(terminal_id, context)
                            .await
                            .map(|t| OcpiReply::ok(t).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/payments/financial-advice-confirmations",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                PageParams(query): PageParams,
                                State(state): State<Arc<OcpiState>>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        fac_list
                            .financial_advice_confirmations(query, context)
                            .await
                            .map(|page| page_reply(page, &state).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                "/payments/financial-advice-confirmations/{id}",
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(id): Path<String>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        fac_one
                            .financial_advice_confirmation(id, context)
                            .await
                            .map(|f| OcpiReply::ok(f).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            );
        self
    }

    /// Mounts the Payments Receiver interface: the CPO's copy of the PTP's objects.
    ///
    /// Note the `POST`: unlike every other Receiver interface in OCPI these objects are created
    /// with `POST` and their URLs carry no owning party.
    #[must_use]
    pub fn payments_receiver<H: PaymentsReceiver>(mut self, handler: H) -> Self {
        self.check_receiver_conflict(&ModuleId::Payments);
        let prefix = self.receiver_prefix();
        let handler = Arc::new(handler);
        self.mounted.add(ModuleId::Payments, InterfaceRole::Receiver);
        let get_terminal = Arc::clone(&handler);
        let post_terminal = Arc::clone(&handler);
        let get_fac = Arc::clone(&handler);
        let post_fac = handler;

        self.router = self
            .router
            .route(
                &receiver_path(prefix.as_deref(), "/payments/terminals"),
                post(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                OcpiJson(terminal): OcpiJson<crate::v2_3_0::payments::Terminal>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        post_terminal
                            .post_terminal(terminal, context)
                            .await
                            .map(|t| OcpiReply::created(t).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                &receiver_path(prefix.as_deref(), "/payments/terminals/{terminal_id}"),
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(terminal_id): Path<String>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        get_terminal
                            .terminal(terminal_id, context)
                            .await
                            .map(|t| OcpiReply::ok(t).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                &receiver_path(prefix.as_deref(), "/payments/financial-advice-confirmations"),
                post(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                OcpiJson(confirmation): OcpiJson<
                        crate::v2_3_0::payments::FinancialAdviceConfirmation,
                    >|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        post_fac
                            .post_financial_advice_confirmation(confirmation, context)
                            .await
                            .map(|f| OcpiReply::created(f).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            )
            .route(
                &receiver_path(prefix.as_deref(), "/payments/financial-advice-confirmations/{id}"),
                get(
                    async move |auth: Auth,
                                ids: Ids,
                                routing: Routing,
                                Path(id): Path<String>|
                                -> Result<_, OcpiErrorResponse> {
                        let context = context_of(auth, ids, routing, &ModuleId::Payments)?;
                        let ids = context.ids.clone();
                        get_fac
                            .financial_advice_confirmation(id, context)
                            .await
                            .map(|f| OcpiReply::ok(f).with_ids(ids.clone()))
                            .map_err(|e| OcpiErrorResponse::new(e).with_ids(ids))
                    },
                ),
            );
        self
    }

    /// Finishes the router.
    ///
    /// The `/versions` and version-details endpoints are added automatically and describe exactly
    /// what was mounted.
    pub fn build(self) -> Router {
        let state = Arc::new(OcpiState {
            tokens: self.tokens,
            config: self.config,
            base_url: self.base_url,
            mounted: self.mounted,
            version: self.version,
        });
        let details_state = Arc::clone(&state);
        let versions_state = Arc::clone(&state);

        self.router
            .route(
                "/",
                get(async move |auth: Auth, ids: Ids| -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, Routing(None), &ModuleId::Versions)?;
                    let ids = context.ids.clone();
                    Ok(OcpiReply::ok(details_state.version_details()).with_ids(ids))
                }),
            )
            .route(
                "/versions",
                get(async move |auth: Auth, ids: Ids| -> Result<_, OcpiErrorResponse> {
                    let context = context_of(auth, ids, Routing(None), &ModuleId::Versions)?;
                    let ids = context.ids.clone();
                    let versions = vec![crate::v2_3_0::versions::Version::new(
                        versions_state.version.clone(),
                        versions_state.base_url.clone(),
                    )];
                    Ok(OcpiReply::ok(versions).with_ids(ids))
                }),
            )
            .with_state(state)
    }
}

/// The path a Receiver route is mounted at, applying [`ServerConfig::receiver_path_prefix`].
fn receiver_path(prefix: Option<&str>, suffix: &str) -> String {
    match prefix {
        Some(prefix) => format!("/{}{suffix}", prefix.trim_matches('/')),
        None => suffix.to_owned(),
    }
}

/// The `?duration=` and `?response_url=` parameters of the Charging Profiles Receiver interface.
///
/// > *NOTE: As it is not common to add a body to a GET request, all parameters are added to the
/// > URL.*
#[derive(Debug, serde::Deserialize)]
struct ActiveProfileQuery {
    #[serde(default)]
    duration: Option<u64>,
    #[serde(default)]
    response_url: Option<Url>,
}

/// Decodes a body that is an object with fields left out, as `POST .../terminals/activate` is.
///
/// This is not a merge patch — nothing is being merged — so it is decoded as a plain JSON object
/// and wrapped, rather than going through the [`OcpiPatch`] extractor and picking up the rule
/// that a `PATCH` must carry `last_updated`.
fn partial_object<T>(body: &[u8]) -> Result<crate::transport::Patch<T>, OcpiError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| OcpiError::MalformedJson(e.to_string()))?;
    if !value.is_object() {
        return Err(OcpiError::Decode {
            path: "/".to_owned(),
            message: "expected a JSON object".to_owned(),
        });
    }
    Ok(crate::transport::Patch::from_value(value))
}

/// The `2001` for a URL parameter the specification marks required.
fn missing_query(name: &str) -> OcpiError {
    OcpiError::Decode { path: format!("?{name}"), message: format!("the {name} query parameter is required") }
}

/// Builds the `response_url`s that reach the callbacks this crate's router mounts.
///
/// The asynchronous halves of the Commands and Charging Profiles modules are the one place where
/// OCPI hands the URL shape to the implementation:
///
/// > *No structure defined. This is open to the eMSP to define, the URL is provided to the
/// > Receiver by the Sender.*
///
/// That freedom is easy to get wrong in a way that only shows up minutes later, when a Charge
/// Point answers into a 404. So the Sender-side mounts on [`OcpiRouter`] use fixed shapes, and
/// this type builds exactly those — pass the same `base_url` the router is published under and
/// the two cannot drift apart.
///
/// ```
/// use ocpi_kit::server::CallbackUrls;
/// use ocpi_kit::types::Url;
///
/// let urls = CallbackUrls::new(Url::new("https://msp.example.com/ocpi/emsp/2.3.0").unwrap());
/// assert_eq!(
///     urls.command_result("RESERVE_NOW", "1234").as_str(),
///     "https://msp.example.com/ocpi/emsp/2.3.0/commands/RESERVE_NOW/1234",
/// );
/// assert_eq!(
///     urls.clear_profile_result("5678").as_str(),
///     "https://msp.example.com/ocpi/emsp/2.3.0/chargingprofiles/result/clear/5678",
/// );
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallbackUrls {
    base: Url,
}

impl CallbackUrls {
    /// Callback URLs under `base_url`, which must be the URL this party's router is published at.
    #[must_use]
    pub const fn new(base_url: Url) -> Self {
        Self { base: base_url }
    }

    /// The base URL these callbacks are built from.
    #[must_use]
    pub const fn base(&self) -> &Url {
        &self.base
    }

    /// `{base}/commands/{command}/{unique_id}` — where a `CommandResult` is expected.
    ///
    /// Put this in the `response_url` of a
    /// [`Command`](crate::v2_3_0::commands::Command); it reaches
    /// [`CommandsSender::command_result`] with `unique_id`.
    #[must_use]
    pub fn command_result(&self, command: &str, unique_id: &str) -> Url {
        self.base.join("commands").join(command).join(unique_id)
    }

    /// `{base}/chargingprofiles/result/active/{unique_id}` — for an `ActiveChargingProfileResult`.
    #[must_use]
    pub fn active_charging_profile_result(&self, unique_id: &str) -> Url {
        self.base.join("chargingprofiles/result/active").join(unique_id)
    }

    /// `{base}/chargingprofiles/result/set/{unique_id}` — for a `ChargingProfileResult`.
    #[must_use]
    pub fn charging_profile_result(&self, unique_id: &str) -> Url {
        self.base.join("chargingprofiles/result/set").join(unique_id)
    }

    /// `{base}/chargingprofiles/result/clear/{unique_id}` — for a `ClearProfileResult`.
    #[must_use]
    pub fn clear_profile_result(&self, unique_id: &str) -> Url {
        self.base.join("chargingprofiles/result/clear").join(unique_id)
    }
}

/// The `?type=` query parameter that the Tokens module uses.
#[derive(Debug, serde::Deserialize)]
struct TokenTypeQuery {
    #[serde(rename = "type", default)]
    token_type: Option<crate::v2_3_0::tokens::TokenType>,
}

/// Decodes a request body that the specification marks optional.
///
/// An empty body, and a body that is an empty JSON object, both mean "not given".
fn decode_optional_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<Option<T>, OcpiError> {
    let trimmed = body.trim_ascii();
    if trimmed.is_empty() || trimmed == b"{}" {
        return Ok(None);
    }
    let mut de = serde_json::Deserializer::from_slice(trimmed);
    serde_path_to_error::deserialize(&mut de).map(Some).map_err(|e| OcpiError::Decode {
        path: format!("/{}", e.path()),
        message: e.into_inner().to_string(),
    })
}

fn status_of(created: super::traits::Created) -> http::StatusCode {
    http::StatusCode::from_u16(created.http_status()).unwrap_or(http::StatusCode::OK)
}

fn page_reply<T>(page: Page<T>, state: &Arc<OcpiState>) -> OcpiReply<Vec<T>> {
    let mut headers = http::HeaderMap::new();
    let meta = PageMeta { limit: Some(state.config.max_page_limit), ..page.meta };
    meta.write_to(&mut headers);
    OcpiReply::ok(page.items).with_headers(headers)
}

/// Builds the handler context, enforcing the `CREDENTIALS_TOKEN_A` scope rule.
fn context_of(
    Auth(peer): Auth,
    Ids(ids): Ids,
    Routing(routing): Routing,
    module: &ModuleId,
) -> Result<RequestContext, OcpiErrorResponse> {
    peer.check_scope(module).map_err(|e| OcpiErrorResponse::new(e).with_ids(ids.clone()))?;
    Ok(RequestContext { peer, ids, routing })
}

/// Builds the context for a client-owned-object URL, enforcing the ownership rule as well.
fn owned_context(
    auth: Auth,
    ids: Ids,
    routing: Routing,
    module: &ModuleId,
    country_code: &str,
    party_id: &str,
) -> Result<(RequestContext, PartyRef), OcpiErrorResponse> {
    let request_ids = ids.0.clone();
    let context = context_of(auth, ids, routing, module)?;
    let Owner(owner) = Owner::from_path(country_code, party_id)
        .map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids.clone()))?;
    context.peer.check_ownership(&owner).map_err(|e| OcpiErrorResponse::new(e).with_ids(request_ids))?;
    Ok((context, owner))
}

/// Decodes the body of `POST {commands}/{command}` into the right command object.
fn parse_command(name: &str, body: &[u8]) -> Result<crate::v2_3_0::commands::Command, OcpiError> {
    use crate::v2_3_0::commands::{
        CancelReservation, Command, CommandType, ReserveNow, StartSession, StopSession, UnlockConnector,
    };

    fn decode<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, OcpiError> {
        let mut de = serde_json::Deserializer::from_slice(body);
        serde_path_to_error::deserialize(&mut de).map_err(|e| OcpiError::Decode {
            path: format!("/{}", e.path()),
            message: e.into_inner().to_string(),
        })
    }

    match CommandType::from(name) {
        CommandType::CancelReservation => Ok(Command::CancelReservation(decode::<CancelReservation>(body)?)),
        CommandType::ReserveNow => Ok(Command::ReserveNow(Box::new(decode::<ReserveNow>(body)?))),
        CommandType::StartSession => Ok(Command::StartSession(Box::new(decode::<StartSession>(body)?))),
        CommandType::StopSession => Ok(Command::StopSession(decode::<StopSession>(body)?)),
        CommandType::UnlockConnector => Ok(Command::UnlockConnector(decode::<UnlockConnector>(body)?)),
        other => Err(OcpiError::NotFound(format!("no such command: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_decoded_by_the_name_in_the_url() {
        let body = br#"{"response_url":"https://msp.example.com/cb/1","session_id":"101"}"#;
        let command = parse_command("STOP_SESSION", body).unwrap();
        assert_eq!(command.command_type(), crate::v2_3_0::commands::CommandType::StopSession);
        assert!(parse_command("nltnm-CUSTOM", body).is_err());
        // A body that is not the command it claims to be reports where it went wrong.
        let err = parse_command("STOP_SESSION", b"{}").unwrap_err();
        assert_eq!(err.status_code(), crate::transport::StatusCode::INVALID_PARAMETERS);
    }
}
