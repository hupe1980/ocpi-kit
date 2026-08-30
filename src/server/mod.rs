//! An OCPI server: one trait per module and interface, mounted onto an `axum::Router`.
//!
//! ```no_run
//! use std::sync::Arc;
//! use ocpi_kit::server::{InMemoryTokenStore, OcpiRouter};
//! use ocpi_kit::types::Url;
//! use ocpi_kit::VersionNumber;
//!
//! # async fn serve(
//! #     credentials: impl ocpi_kit::server::CredentialsHandler,
//! #     locations: impl ocpi_kit::server::LocationsSender,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let tokens = Arc::new(InMemoryTokenStore::new());
//!
//! let app = OcpiRouter::new(
//!         VersionNumber::V2_3_0,
//!         Url::new("https://cpo.example.com/ocpi/cpo/2.3.0")?,
//!         tokens,
//!     )
//!     .credentials(credentials)
//!     .locations_sender(locations)
//!     .build();
//!
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
//! axum::serve(listener, app).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # What the router takes care of
//!
//! * **The status code rules.** Only five situations get an HTTP error status; everything that
//!   reached the OCPI layer is a `200 OK` with a four-digit code in the body. A handler returns
//!   [`OcpiError`](crate::transport::OcpiError) and the mapping happens once, correctly.
//! * **Authentication, and the `CREDENTIALS_TOKEN_A` scope.** A bootstrap token used on any
//!   module other than `credentials` and `versions` gets a 401, as the specification requires.
//! * **Ownership of client-owned objects.** A platform writing under a `country_code`/`party_id`
//!   that is not one of its own roles gets a 404 — *"this way blocking client access to objects
//!   that do not belong to them"* — and the handler is never called.
//! * **`X-Request-ID` and `X-Correlation-ID`.** Echoed on every response, generated when the peer
//!   forgot them.
//! * **Version details.** `/versions` and the version-details endpoint are generated from exactly
//!   what was mounted, so discovery cannot disagree with reality.
//! * **The PATCH rule.** A patch without `last_updated` never reaches a handler; it is the
//!   specification's own example of a `2001`.
//!
//! # What it deliberately leaves to you
//!
//! Persistence, and the two credentials 405 rules — only the implementation knows whether a peer
//! is already registered. [`PeerState`](crate::client::PeerState) has the predicates for those.

mod auth;
mod bridge;
mod error;
mod extract;
mod router;
mod traits;

pub use auth::{AuthenticatedPeer, InMemoryTokenStore, MountedModules, PeerRegistry, TokenStore};
pub use error::{HttpStatusCode, OcpiErrorResponse, OcpiReply, echo_ids};
pub use extract::{
    Auth, AuthState, ContentTypePolicy, Ids, OcpiJson, OcpiPatch, Owner, Page, PagePolicy, RequestContext,
    Routing, accepts_json, reject,
};
pub use router::{CallbackUrls, OcpiRouter, OcpiState, ServerConfig};
pub use traits::{
    CdrsReceiver, CdrsSender, ChargingProfilesReceiver, ChargingProfilesSender, CommandsReceiver,
    CommandsSender, Created, CredentialsHandler, Handled, HubClientInfoReceiver, HubClientInfoSender,
    LocationsReceiver, LocationsSender, PaymentsReceiver, PaymentsSender, SessionsReceiver, SessionsSender,
    TariffsReceiver, TariffsSender, TokensReceiver, TokensSender,
};
