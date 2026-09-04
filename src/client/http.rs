//! The request executor: auth, headers, retries, envelope decoding, tracing.

use http::{HeaderMap, HeaderValue, Method};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

use crate::ModuleId;
use crate::transport::headers::{APPLICATION_JSON, AUTHORIZATION};
use crate::transport::{
    CredentialsToken, OcpiError, OcpiResponse, Page, PageMeta, Quirks, RequestIds, RoutingHeaders,
};
use crate::types::{Url, UrlPolicy, Validate};

use super::{ClientConfig, RetryPolicy};

/// One outgoing OCPI request, before it is sent.
#[derive(Debug)]
pub struct OcpiRequest {
    /// The HTTP method.
    pub method: Method,
    /// The absolute URL to call.
    pub url: Url,
    /// The module the request addresses, which decides whether routing headers apply.
    pub module: ModuleId,
    /// The routing headers, when the module is a functional one.
    pub routing: Option<RoutingHeaders>,
    /// The request and correlation IDs.
    pub ids: RequestIds,
    /// The JSON body, already serialised.
    pub body: Option<Vec<u8>>,
}

impl OcpiRequest {
    /// A request with freshly generated IDs and no body.
    #[must_use]
    pub fn new(method: Method, url: Url, module: ModuleId) -> Self {
        Self { method, url, module, routing: None, ids: RequestIds::generate(), body: None }
    }

    /// Attaches routing headers, which are dropped for a configuration module.
    ///
    /// > *routing headers SHALL NOT be used with these modules*
    #[must_use]
    pub fn routed(mut self, routing: RoutingHeaders) -> Self {
        if self.module.is_functional() {
            self.routing = Some(routing);
        }
        self
    }

    /// Attaches the request and correlation IDs.
    #[must_use]
    pub fn with_ids(mut self, ids: RequestIds) -> Self {
        self.ids = ids;
        self
    }

    /// Serialises `body` as the request body.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::Decode`] if the value cannot be serialised.
    pub fn with_body<T: Serialize>(mut self, body: &T) -> Result<Self, OcpiError> {
        self.body = Some(
            serde_json::to_vec(body)
                .map_err(|e| OcpiError::Decode { path: "/".to_owned(), message: e.to_string() })?,
        );
        Ok(self)
    }

    /// Whether the specification permits retrying this request automatically.
    ///
    /// > *OCPI messages SHOULD NOT be queued. When a client does a POST, PUT or PATCH request and
    /// > that request fails or times out, the client should not queue the message and retry the
    /// > same message again later.*
    ///
    /// Only `GET` is retryable.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.method == Method::GET
    }

    fn header_map(&self, token: &CredentialsToken, quirks: &Quirks) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let auth = if quirks.send_unencoded_token {
            token.to_header_value_unencoded()
        } else {
            token.to_header_value()
        };
        if let Ok(value) = HeaderValue::from_str(&auth) {
            headers.insert(AUTHORIZATION, value);
        }
        self.ids.write_to(&mut headers);
        if let Some(routing) = &self.routing
            && !quirks.omit_routing_headers
        {
            routing.write_to(&mut headers);
        }
        if self.body.is_some() {
            headers.insert(http::header::CONTENT_TYPE, HeaderValue::from_static(APPLICATION_JSON));
        }
        headers
    }
}

/// Sends OCPI requests and decodes the envelope.
#[derive(Clone, Debug)]
pub struct Transport {
    http: reqwest::Client,
    config: ClientConfig,
}

impl Transport {
    /// Wraps a `reqwest` client.
    #[must_use]
    pub fn new(http: reqwest::Client, config: ClientConfig) -> Self {
        Self { http, config }
    }

    /// The URL policy every outgoing request is checked against.
    #[must_use]
    pub const fn url_policy(&self) -> &UrlPolicy {
        &self.config.url_policy
    }

    /// The configuration in use.
    #[must_use]
    pub const fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Sends a request and decodes the response envelope into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError`] for a refused URL, a transport failure, a body that is not the
    /// expected shape, or a non-success OCPI status code.
    pub async fn send<T: DeserializeOwned>(
        &self,
        request: &OcpiRequest,
        token: &CredentialsToken,
        quirks: &Quirks,
    ) -> Result<T, OcpiError> {
        let (response, _) = self.send_with_headers::<T>(request, token, quirks).await?;
        response.into_result()
    }

    /// Sends a request and returns one page of a list endpoint.
    ///
    /// # Errors
    ///
    /// As [`Transport::send`].
    pub async fn send_page<T: DeserializeOwned>(
        &self,
        request: &OcpiRequest,
        token: &CredentialsToken,
        quirks: &Quirks,
    ) -> Result<Page<T>, OcpiError> {
        let (response, headers) = self.send_with_headers::<Vec<T>>(request, token, quirks).await?;
        let meta = PageMeta::from_headers(&headers);
        Ok(Page { items: response.into_list()?, meta })
    }

    /// Sends a request and returns the envelope together with the response headers.
    ///
    /// # Errors
    ///
    /// As [`Transport::send`], except that a non-success status code is returned in the envelope
    /// rather than as an error.
    pub async fn send_with_headers<T: DeserializeOwned>(
        &self,
        request: &OcpiRequest,
        token: &CredentialsToken,
        quirks: &Quirks,
    ) -> Result<(OcpiResponse<T>, HeaderMap), OcpiError> {
        self.config.url_policy.check(&request.url).map_err(|e| OcpiError::UrlRefused {
            url: request.url.as_str().to_owned(),
            reason: e.to_string(),
        })?;

        let retries = if request.is_retryable() { self.config.retry.max_attempts } else { 1 };
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match self.attempt(request, token, quirks).await {
                Ok(result) => return Ok(result),
                Err(error) if attempt < retries && error.is_transient() => {
                    #[cfg(feature = "client")]
                    tracing::debug!(
                        attempt,
                        %error,
                        url = request.url.as_str(),
                        "retrying a GET after a transient failure",
                    );
                    tokio::time::sleep(
                        self.config.retry.delay_for(attempt, RetryPolicy::seed_from(&request.ids.request_id)),
                    )
                    .await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn attempt<T: DeserializeOwned>(
        &self,
        request: &OcpiRequest,
        token: &CredentialsToken,
        quirks: &Quirks,
    ) -> Result<(OcpiResponse<T>, HeaderMap), OcpiError> {
        // The span is attached with `Instrument` rather than entered with a guard: an
        // `Entered` guard held across an `.await` stays entered while the task is parked, so
        // whatever the executor polls next inherits this request's span. `Instrument` enters
        // and exits around each poll, which is the only correct form in an async fn.
        let span = tracing::info_span!(
            "ocpi.request",
            otel.kind = "client",
            http.request.method = %request.method,
            url.full = request.url.as_str(),
            ocpi.module = %request.module,
            ocpi.request_id = request.ids.request_id.as_str(),
            ocpi.correlation_id = request.ids.correlation_id.as_str(),
            ocpi.to = request.routing.as_ref().and_then(|r| r.to.as_ref()).map(ToString::to_string),
            ocpi.from = request.routing.as_ref().map(|r| r.from.to_string()),
            ocpi.status_code = tracing::field::Empty,
            http.response.status_code = tracing::field::Empty,
        );
        self.attempt_instrumented(request, token, quirks, span.clone()).instrument(span).await
    }

    async fn attempt_instrumented<T: DeserializeOwned>(
        &self,
        request: &OcpiRequest,
        token: &CredentialsToken,
        quirks: &Quirks,
        span: tracing::Span,
    ) -> Result<(OcpiResponse<T>, HeaderMap), OcpiError> {
        let mut builder = self
            .http
            .request(request.method.clone(), request.url.as_str())
            .timeout(self.config.timeout)
            .headers(request.header_map(token, quirks));
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }

        let response = builder.send().await.map_err(transport_error)?;
        let status = response.status();
        let headers = response.headers().clone();
        span.record("http.response.status_code", status.as_u16());

        let bytes = response.bytes().await.map_err(transport_error)?;

        // The five HTTP statuses the specification does use, before the OCPI layer is reached.
        if !status.is_success() {
            return Err(match status.as_u16() {
                400 => OcpiError::MalformedJson(preview(&bytes)),
                401 => OcpiError::Unauthorized(preview(&bytes)),
                404 => OcpiError::NotFound(request.url.as_str().to_owned()),
                405 => OcpiError::MethodNotAllowed(request.url.as_str().to_owned()),
                other => OcpiError::Transport(format!("HTTP {other}: {}", preview(&bytes))),
            });
        }

        let mut de = serde_json::Deserializer::from_slice(&bytes);
        let envelope: OcpiResponse<T> = serde_path_to_error::deserialize(&mut de).map_err(|e| {
            OcpiError::Decode { path: e.path().to_string(), message: e.into_inner().to_string() }
        })?;
        span.record("ocpi.status_code", envelope.status_code.get());
        Ok((envelope, headers))
    }
}

/// Validates an object before it goes on the wire, when the configuration asks for it.
///
/// # Errors
///
/// Returns [`OcpiError::Invalid`] listing every violation.
pub fn check_outgoing<T: Validate>(value: &T, config: &ClientConfig) -> Result<(), OcpiError> {
    if !config.validate_outgoing {
        return Ok(());
    }
    value.validate().map_err(OcpiError::Invalid)
}

/// Classifies a `reqwest` failure, keeping a timeout distinguishable from every other transport
/// failure.
///
/// `reqwest` knows which it was; recovering that from the message string — which is what a hub
/// would otherwise have to do to choose between `4002` and `4003` — is guesswork about another
/// crate's formatting.
fn transport_error(error: reqwest::Error) -> OcpiError {
    let message = strip_url(&error.to_string());
    if error.is_timeout() { OcpiError::Timeout(message) } else { OcpiError::Transport(message) }
}

/// A `reqwest` error message can contain the full URL, including a `response_url` that carries a
/// one-time token. Keep the cause, drop the URL.
fn strip_url(message: &str) -> String {
    match message.find(" for url (") {
        Some(at) => message[..at].to_owned(),
        None => message.to_owned(),
    }
}

fn preview(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.chars().count() <= 200 {
        return trimmed.to_owned();
    }
    format!("{}…", trimmed.chars().take(200).collect::<String>())
}

impl RetryPolicy {
    /// How long to wait before attempt `attempt` (1-based, so the first retry is attempt 2),
    /// for the request identified by `seed`.
    ///
    /// The delay is exponential — `initial_delay * 2^(attempt-1)`, capped at `max_delay` — with
    /// **equal jitter**: the value actually returned is drawn from the upper half of that
    /// interval, `[base/2, base]`.
    ///
    /// `seed` is what makes the jitter useful. A schedule computed from the attempt number alone
    /// is identical on every client in a fleet, so a peer that has just come back from an outage
    /// is hit by all of them at the same instant — the thundering herd that jitter exists to
    /// prevent. Pass something that differs per client and per request;
    /// [`RetryPolicy::seed_from`] derives one from the request's `X-Request-ID`, which is a
    /// freshly generated UUID and therefore already carries the entropy needed.
    #[must_use]
    pub fn delay_for(&self, attempt: u32, seed: u64) -> std::time::Duration {
        let exponent = attempt.saturating_sub(1).min(16);
        let factor = 1u32 << exponent;
        let base = self.initial_delay.saturating_mul(factor).min(self.max_delay);
        let half = base / 2;
        // A SplitMix64 finaliser over (seed, attempt): a good avalanche in a handful of
        // instructions, so two clients whose request ids differ in one bit wait very different
        // amounts, and no random number generator has to be threaded through the client.
        let spread = half
            .as_nanos()
            .try_into()
            .map_or(0, |span: u64| if span == 0 { 0 } else { mix64(seed ^ u64::from(attempt)) % span });
        half.saturating_add(std::time::Duration::from_nanos(spread)).min(self.max_delay)
    }

    /// A [`delay_for`](Self::delay_for) seed derived from a request id.
    #[must_use]
    pub fn seed_from(request_id: &str) -> u64 {
        // FNV-1a: no dependency, and every byte of the UUID reaches the result.
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in request_id.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

/// The SplitMix64 finalising mix, used to turn a seed into well-distributed jitter.
const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_get_may_be_retried() {
        let url = Url::new("https://e.com/a").unwrap();
        let get = OcpiRequest::new(Method::GET, url.clone(), ModuleId::Cdrs);
        let put = OcpiRequest::new(Method::PUT, url.clone(), ModuleId::Cdrs);
        let post = OcpiRequest::new(Method::POST, url, ModuleId::Cdrs);
        assert!(get.is_retryable());
        assert!(!put.is_retryable(), "the spec forbids queueing and retrying writes");
        assert!(!post.is_retryable());
    }

    #[test]
    fn routing_headers_are_dropped_for_configuration_modules() {
        let url = Url::new("https://e.com/a").unwrap();
        let routing = RoutingHeaders::new(
            crate::types::PartyRef::new("NL", "TNM").unwrap(),
            crate::types::PartyRef::new("DE", "ABC").unwrap(),
        );
        let functional =
            OcpiRequest::new(Method::GET, url.clone(), ModuleId::Locations).routed(routing.clone());
        assert!(functional.routing.is_some());
        let configuration = OcpiRequest::new(Method::GET, url, ModuleId::Credentials).routed(routing);
        assert!(configuration.routing.is_none());
    }

    #[test]
    fn the_authorization_header_follows_the_peers_quirks() {
        let token = CredentialsToken::new("example-token").unwrap();
        let request = OcpiRequest::new(Method::GET, Url::new("https://e.com/a").unwrap(), ModuleId::Cdrs);

        let modern = request.header_map(&token, &Quirks::default());
        assert_eq!(modern.get(AUTHORIZATION).unwrap(), "Token ZXhhbXBsZS10b2tlbg==");

        let legacy = request.header_map(&token, &Quirks::for_version(&crate::VersionNumber::V2_1_1));
        assert_eq!(legacy.get(AUTHORIZATION).unwrap(), "Token example-token");
    }

    #[test]
    fn a_body_sets_the_content_type_and_nothing_else_does() {
        let url = Url::new("https://e.com/a").unwrap();
        let empty = OcpiRequest::new(Method::GET, url.clone(), ModuleId::Cdrs)
            .header_map(&CredentialsToken::new("t").unwrap(), &Quirks::default());
        assert!(empty.get(http::header::CONTENT_TYPE).is_none());

        let with_body = OcpiRequest::new(Method::PUT, url, ModuleId::Cdrs)
            .with_body(&serde_json::json!({"a": 1}))
            .unwrap()
            .header_map(&CredentialsToken::new("t").unwrap(), &Quirks::default());
        assert_eq!(with_body.get(http::header::CONTENT_TYPE).unwrap(), "application/json");
    }

    #[test]
    fn retry_delays_grow_and_stay_under_the_cap() {
        let policy = RetryPolicy::default();
        let seed = RetryPolicy::seed_from("6d2b1b3a-0f8f-4e7e-9d3f-1a2b3c4d5e6f");
        assert!(policy.delay_for(2, seed) > policy.delay_for(1, seed));
        assert!(policy.delay_for(20, seed) <= policy.max_delay);
        // Equal jitter: never below half the exponential base, never above it.
        for attempt in 1..8 {
            let base = policy.initial_delay.saturating_mul(1 << (attempt - 1)).min(policy.max_delay);
            let delay = policy.delay_for(attempt, seed);
            assert!(
                delay >= base / 2 && delay <= base,
                "attempt {attempt}: {delay:?} not in half of {base:?}"
            );
        }
    }

    #[test]
    fn two_clients_retrying_the_same_endpoint_do_not_wait_the_same_time() {
        // The whole point of jitter: a schedule computed from the attempt number alone would be
        // identical everywhere, and a peer coming back from an outage would be hit by the whole
        // fleet at once.
        let policy = RetryPolicy::default();
        let delays: std::collections::HashSet<_> =
            (0..64).map(|i| policy.delay_for(2, RetryPolicy::seed_from(&format!("request-{i}")))).collect();
        assert!(delays.len() > 50, "only {} distinct delays across 64 requests", delays.len());
    }

    #[test]
    fn a_policy_that_does_not_retry_waits_for_nothing() {
        let policy = RetryPolicy::none();
        assert_eq!(policy.delay_for(3, 12345), std::time::Duration::ZERO);
    }

    #[test]
    fn transport_error_messages_do_not_leak_the_url() {
        let message = "error sending request for url (https://e.com/cb?token=secret)";
        assert_eq!(strip_url(message), "error sending request");
    }
}
