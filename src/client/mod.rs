//! An async OCPI client: registration handshake, typed module clients, paginated crawls.
//!
//! ```no_run
//! use ocpi_kit::client::{OcpiClient, Registration};
//! use ocpi_kit::transport::{CredentialsToken, PageQuery};
//! use ocpi_kit::types::{PartyRef, Url};
//! use ocpi_kit::{InterfaceRole, ModuleId};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = OcpiClient::new()?;
//! let me = PartyRef::new("NL", "TNM")?;
//!
//! // The registration handshake, in the order the specification defines it.
//! let peer = Registration::new(
//!         Url::new("https://cpo.example.com/ocpi/versions")?,
//!         CredentialsToken::new("token-a-received-out-of-band")?,
//!     )
//!     .discover(client.transport()).await?
//!     .select_best(client.transport()).await?;
//!
//! // Refuse to register with a peer that does not implement what we need — before POSTing.
//! peer.require(&[(ModuleId::Locations, InterfaceRole::Sender)])?;
//!
//! let peer = peer.register(client.transport(), &my_credentials()).await?;
//!
//! // Bind the transport, the peer and the party you are calling as — once, not per call.
//! let cpo = client.peer(&peer, me);
//!
//! // Then pull, following every `Link: rel="next"`.
//! let mut locations = cpo.locations().list(PageQuery::new())?;
//! while let Some(location) = locations.next().await? {
//!     println!("{} {}", location.id, location.name.as_deref().unwrap_or(""));
//! }
//! # Ok(())
//! # }
//! # fn my_credentials() -> ocpi_kit::v2_3_0::credentials::Credentials { unimplemented!() }
//! ```
//!
//! # What this client does that a hand-rolled one usually does not
//!
//! * **It refuses to connect where it should not.** `Credentials.url`, `Endpoint.url`, every
//!   `next` link and every `response_url` are chosen by the counterparty; a client that fetches
//!   them unconditionally is an SSRF proxy. A [`UrlPolicy`] (HTTPS only, no private networks by
//!   default) is enforced three times: on the URL, on every redirect hop — a `GET` may follow
//!   at most five redirects within its own origin, a write none — and on the address actually
//!   connected to, so a host name that resolves to a private address, or that changes its answer
//!   between a check and a connection, is refused too. See [`ClientConfig::http_builder`].
//! * **It validates what it sends.** [`ClientConfig::validate_outgoing`] is on by default, so a
//!   non-conformant object is caught here rather than at the partner's support desk.
//! * **It only retries what it may.** *"OCPI messages SHOULD NOT be queued. When a client does a
//!   POST, PUT or PATCH request and that request fails or times out, the client should not queue
//!   the message and retry."* Only `GET` is retried.
//! * **It never logs the token.** The `tracing` spans carry the request and correlation IDs and
//!   the routing parties; [`CredentialsToken`](crate::transport::CredentialsToken) redacts
//!   itself in any case.

mod conformance;
mod egress;
mod http;
mod modules;
mod paging;
mod peer;
mod registration;
mod resync;

pub use conformance::{Check, Conformance, Outcome, Report};
pub use egress::{GuardedResolver, MAX_REDIRECTS};
pub use http::{OcpiRequest, Transport, check_outgoing};
#[cfg(feature = "payments")]
#[cfg_attr(docsrs, doc(cfg(feature = "payments")))]
pub use modules::PaymentsClient;
pub use modules::{
    CdrsClient, ChargingProfilesClient, CommandsClient, HubClientInfoClient, LocationsReceiver,
    LocationsSender, ModuleClient, PeerSession, SessionsReceiver, SessionsSender, TariffsReceiver,
    TariffsSender, TokensReceiver, TokensSender, correlated_ids,
};
pub use paging::{DEFAULT_MAX_PAGES, PageStream};
pub use peer::{Peer, PeerBuilder};
pub use registration::{Discovered, PeerState, Registration, Selected};
pub use resync::{Resync, ResyncPlan};

use std::time::Duration;

use crate::types::UrlPolicy;

/// How the client behaves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ClientConfig {
    /// What this client is willing to send a request to. Defaults to HTTPS, no private networks.
    pub url_policy: UrlPolicy,
    /// How long one request may take.
    pub timeout: Duration,
    /// How `GET` requests are retried. Writes are never retried.
    pub retry: RetryPolicy,
    /// Whether objects are validated before being sent. Defaults to `true`.
    ///
    /// This is what makes "construct strictly" hold in practice: the infallible `From<&str>`
    /// conversions the builders use are lenient, so the guarantee lives here, at the wire, where
    /// it also catches the cross-field rules no constructor could.
    pub validate_outgoing: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            url_policy: UrlPolicy::default(),
            timeout: Duration::from_secs(30),
            retry: RetryPolicy::default(),
            validate_outgoing: true,
        }
    }
}

impl ClientConfig {
    /// A configuration for talking to a peer on localhost, as an integration test does.
    #[must_use]
    pub fn for_testing() -> Self {
        Self { url_policy: UrlPolicy::permissive(), ..Self::default() }
    }

    /// Sets the URL policy.
    #[must_use]
    pub fn with_url_policy(mut self, policy: UrlPolicy) -> Self {
        self.url_policy = policy;
        self
    }

    /// Sets the per-request timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The HTTP client this configuration needs, ready to customise and build.
    ///
    /// Carries the user agent, **no automatic redirects** (the client follows them itself,
    /// checking each hop) and a [`GuardedResolver`] that refuses every resolved address
    /// [`url_policy`](Self::url_policy) does not permit, at connect time. Start here when you
    /// need a proxy, custom TLS roots or a shared pool, and hand the result to
    /// [`OcpiClient::from_http`]:
    ///
    /// ```no_run
    /// use ocpi_kit::client::{ClientConfig, OcpiClient};
    ///
    /// # fn main() -> Result<(), reqwest::Error> {
    /// let config = ClientConfig::default();
    /// let http = config.http_builder().pool_max_idle_per_host(32).build()?;
    /// let client = OcpiClient::from_http(http, config);
    /// # Ok(()) }
    /// ```
    ///
    /// Behind an HTTP proxy the resolver sees the proxy rather than the target, so egress rules
    /// for the target are the proxy's to enforce. `ClientBuilder::resolve` overrides bypass the
    /// resolver for the names they pin; that is what they are for.
    pub fn http_builder(&self) -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .user_agent(concat!("ocpi-kit/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(GuardedResolver::new(self.url_policy.clone()))
    }

    /// Turns off validation of outgoing objects.
    ///
    /// Only reasonable when a peer is known to require something non-conformant, and then it is
    /// better to record the reason next to the call.
    #[must_use]
    pub const fn without_outgoing_validation(mut self) -> Self {
        self.validate_outgoing = false;
        self
    }
}

/// How a failed `GET` is retried.
///
/// Writes are never retried; see [`OcpiRequest::is_retryable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Total attempts, including the first. `1` disables retrying.
    pub max_attempts: u32,
    /// The delay before the first retry.
    pub initial_delay: Duration,
    /// The cap on the exponentially growing delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(10),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries.
    #[must_use]
    pub const fn none() -> Self {
        Self { max_attempts: 1, initial_delay: Duration::from_millis(0), max_delay: Duration::from_millis(0) }
    }
}

/// The entry point: an HTTP client plus the configuration every request uses.
#[derive(Clone, Debug)]
pub struct OcpiClient {
    transport: Transport,
}

impl OcpiClient {
    /// A client with the default configuration.
    ///
    /// # Errors
    ///
    /// Returns the `reqwest` error if the HTTP client cannot be built, which happens when the
    /// platform has no usable TLS backend.
    pub fn new() -> Result<Self, reqwest::Error> {
        Self::with_config(ClientConfig::default())
    }

    /// A client with a specific configuration.
    ///
    /// # Errors
    ///
    /// As [`OcpiClient::new`].
    pub fn with_config(config: ClientConfig) -> Result<Self, reqwest::Error> {
        let http = config.http_builder().build()?;
        Ok(Self { transport: Transport::new(http, config) })
    }

    /// A client over an existing `reqwest` client, for sharing a connection pool or using a proxy.
    ///
    /// Build `http` from [`ClientConfig::http_builder`]. A client built any other way keeps the
    /// URL check but loses the connect-time address check; and if it follows redirects itself
    /// (`reqwest`'s default), every response it reached by one is refused as
    /// [`UrlRefused`](crate::transport::OcpiError::UrlRefused) — after the request was sent.
    #[must_use]
    pub fn from_http(http: reqwest::Client, config: ClientConfig) -> Self {
        Self { transport: Transport::new(http, config) }
    }

    /// Everything one of your parties can call on one registered peer.
    ///
    /// The entry point for every module call. The transport, the peer and the party making the
    /// request do not change between calls, so they are bound once here rather than passed to
    /// each one.
    ///
    /// ```no_run
    /// # use ocpi_kit::client::{OcpiClient, Peer};
    /// # async fn f(peer: Peer) -> Result<(), Box<dyn std::error::Error>> {
    /// let client = OcpiClient::new()?;
    /// let cpo = client.peer(&peer, "NL/TNM".parse()?);
    /// let location = cpo.locations().location("LOC1").await?;
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub fn peer<'a>(&'a self, peer: &'a Peer, from: crate::types::PartyRef) -> PeerSession<'a> {
        PeerSession::new(&self.transport, peer, from)
    }

    /// The request executor, which the registration handshake takes.
    #[must_use]
    pub const fn transport(&self) -> &Transport {
        &self.transport
    }

    /// The configuration in use.
    #[must_use]
    pub const fn config(&self) -> &ClientConfig {
        self.transport.config()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_configuration_is_the_careful_one() {
        let config = ClientConfig::default();
        assert!(config.validate_outgoing, "a non-conformant object should not reach a partner");
        assert!(
            config.url_policy.check(&crate::types::Url::new("http://e.com/a").unwrap()).is_err(),
            "plain HTTP is refused by default"
        );
        assert_eq!(config.retry.max_attempts, 3);
    }

    #[test]
    fn the_testing_configuration_allows_localhost() {
        let config = ClientConfig::for_testing();
        assert!(
            config.url_policy.check(&crate::types::Url::new("http://127.0.0.1:8080/ocpi").unwrap()).is_ok()
        );
    }

    #[test]
    fn retrying_can_be_switched_off() {
        assert_eq!(RetryPolicy::none().max_attempts, 1);
    }
}
