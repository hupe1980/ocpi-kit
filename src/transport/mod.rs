//! Everything between the JSON and the HTTP: envelope, status codes, headers, credentials
//! tokens, pagination, routing, endpoint URLs, PATCH and per-peer quirks.
//!
//! This layer has no HTTP client and no async runtime. It is the shared vocabulary that
//! [`client`](crate::client), [`server`](crate::server) and [`hub`](crate::hub) are written in,
//! and it is deliberately usable on its own by anyone who wants to keep their own HTTP stack.
//!
//! # The rules this layer exists to enforce
//!
//! * **HTTP status codes are almost never how OCPI reports a problem.** Only five situations get
//!   an HTTP error; everything else is `200 OK` with a four-digit code in the body. See
//!   [`OcpiError::http_status`].
//! * **A hub renews `X-Request-ID` and preserves `X-Correlation-ID`.** [`RequestIds::forwarded`].
//! * **Routing headers belong on functional modules only.** [`RoutingHeaders::applies_to`].
//! * **`CREDENTIALS_TOKEN_A` is scoped to `credentials` and `versions`.** [`TokenRole::may_access`].
//! * **A PATCH must carry `last_updated`.** [`Patch::apply`].
//!
//! Spec: 2.3.0 §transport_and_format_transport_and_format, §status_codes_status_codes

pub mod auth;
pub mod endpoints;
pub mod envelope;
pub mod headers;
pub mod pagination;
pub mod patch;
pub mod quirks;
pub mod routing;
pub mod status;

pub use auth::{CredentialsToken, InvalidToken, TOKEN_PREFIX, TokenRole};
pub use endpoints::{ReceiverEndpoint, SenderEndpoint};
pub use envelope::{OcpiError, OcpiResponse};
pub use headers::{RequestIds, header_party, header_str, header_u64, link_next, parse_link_next};
pub use pagination::{CrawlAdjustment, Page, PageMeta, PageQuery, crawl_adjustment};
pub use patch::{Patch, PatchFallback, merge, patch_fallback};
pub use quirks::Quirks;
pub use routing::{RoutingHeaders, RoutingScenario};

#[cfg(feature = "client")]
pub use crate::client::OcpiRequest;
pub use status::{StatusClass, StatusCode};
