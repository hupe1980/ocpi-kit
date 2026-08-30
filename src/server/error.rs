//! Turning an [`OcpiError`] into the HTTP response the specification prescribes.

use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode as HttpStatus};

use crate::transport::headers::APPLICATION_JSON;
use crate::transport::{OcpiError, OcpiResponse, RequestIds, RoutingHeaders};
use crate::types::Validate;

/// An OCPI response on its way out: the envelope, plus the headers that must accompany it.
///
/// Every OCPI response carries the request and correlation IDs it was called with:
///
/// > *Every request SHALL contain a unique request ID, the response to this request SHALL contain
/// > the same ID.*
///
/// so this type carries them rather than leaving it to each handler to remember.
#[derive(Debug)]
pub struct OcpiReply<T> {
    envelope: OcpiResponse<T>,
    http_status: HttpStatus,
    ids: Option<RequestIds>,
    routing: Option<RoutingHeaders>,
    extra: HeaderMap,
}

impl<T> OcpiReply<T> {
    /// A `1000 Generic success` reply with a payload, answered with HTTP 200.
    #[must_use]
    pub fn ok(data: T) -> Self {
        Self {
            envelope: OcpiResponse::success(data),
            http_status: HttpStatus::OK,
            ids: None,
            routing: None,
            extra: HeaderMap::new(),
        }
    }

    /// A `1000` reply for an object that was newly created, answered with HTTP 201.
    ///
    /// > *HTTP `201 - Created` when the object has been newly created in the server system.*
    #[must_use]
    pub fn created(data: T) -> Self {
        Self { http_status: HttpStatus::CREATED, ..Self::ok(data) }
    }

    /// A `1000` reply with no payload, for a PUT or PATCH whose response body is unspecified.
    ///
    /// > *We also advise that in such cases, platform sending the response leave the `data` field
    /// > unset in the response format.*
    #[must_use]
    pub fn no_content() -> Self {
        Self {
            envelope: OcpiResponse::success_empty(),
            http_status: HttpStatus::OK,
            ids: None,
            routing: None,
            extra: HeaderMap::new(),
        }
    }

    /// Attaches the request and correlation IDs of the request being answered.
    #[must_use]
    pub fn with_ids(mut self, ids: RequestIds) -> Self {
        self.ids = Some(ids);
        self
    }

    /// Attaches the routing headers of the response.
    ///
    /// Pass the headers already swapped for the response direction; see
    /// [`RoutingHeaders::response_from`].
    #[must_use]
    pub fn with_routing(mut self, routing: RoutingHeaders) -> Self {
        self.routing = Some(routing);
        self
    }

    /// Adds arbitrary response headers, such as the pagination trio or a `Location`.
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.extra.extend(headers);
        self
    }

    /// Overrides the HTTP status.
    #[must_use]
    pub const fn with_http_status(mut self, status: HttpStatus) -> Self {
        self.http_status = status;
        self
    }

    /// Sets the `status_message`.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.envelope.status_message = Some(message.into());
        self
    }

    /// The envelope that will be sent.
    #[must_use]
    pub const fn envelope(&self) -> &OcpiResponse<T> {
        &self.envelope
    }
}

impl<T: serde::Serialize> IntoResponse for OcpiReply<T> {
    fn into_response(self) -> Response {
        let mut headers = self.extra;
        if let Some(ids) = &self.ids {
            ids.write_to(&mut headers);
        }
        if let Some(routing) = &self.routing {
            routing.write_to(&mut headers);
        }
        headers.insert(http::header::CONTENT_TYPE, HeaderValue::from_static(APPLICATION_JSON));

        let body = match serde_json::to_vec(&self.envelope) {
            Ok(body) => body,
            Err(e) => {
                return internal_error(&format!("cannot serialise the response: {e}"), &headers);
            }
        };
        (self.http_status, headers, body).into_response()
    }
}

/// The HTTP response an [`OcpiError`] becomes.
///
/// The mapping is [`OcpiError::http_status`], which encodes the whole of §status_codes: only five
/// situations get an HTTP error, and everything that reached the OCPI layer is a `200 OK` with a
/// four-digit code in the body.
#[derive(Debug)]
pub struct OcpiErrorResponse {
    error: OcpiError,
    ids: Option<RequestIds>,
}

impl OcpiErrorResponse {
    /// Wraps an error.
    #[must_use]
    pub const fn new(error: OcpiError) -> Self {
        Self { error, ids: None }
    }

    /// Attaches the request and correlation IDs of the request being answered.
    #[must_use]
    pub fn with_ids(mut self, ids: RequestIds) -> Self {
        self.ids = Some(ids);
        self
    }

    /// The error being reported.
    #[must_use]
    pub const fn error(&self) -> &OcpiError {
        &self.error
    }
}

impl From<OcpiError> for OcpiErrorResponse {
    fn from(error: OcpiError) -> Self {
        Self::new(error)
    }
}

impl IntoResponse for OcpiErrorResponse {
    fn into_response(self) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(ids) = &self.ids {
            ids.write_to(&mut headers);
        }
        headers.insert(http::header::CONTENT_TYPE, HeaderValue::from_static(APPLICATION_JSON));

        let status = HttpStatus::from_u16(self.error.http_status()).unwrap_or(HttpStatus::OK);
        let envelope: OcpiResponse<()> = self.error.to_response();
        match serde_json::to_vec(&envelope) {
            Ok(body) => (status, headers, body).into_response(),
            Err(e) => internal_error(&format!("cannot serialise the error: {e}"), &headers),
        }
    }
}

impl IntoResponse for OcpiError {
    fn into_response(self) -> Response {
        OcpiErrorResponse::new(self).into_response()
    }
}

/// The last resort, when even serialising the envelope failed.
fn internal_error(message: &str, headers: &HeaderMap) -> Response {
    let mut headers = headers.clone();
    headers.remove(http::header::CONTENT_TYPE);
    (HttpStatus::INTERNAL_SERVER_ERROR, headers, message.to_owned()).into_response()
}

/// Validates an object a handler is about to return, when the server is configured to.
///
/// # Errors
///
/// Returns [`OcpiError::Invalid`], which becomes a `2001` in the envelope.
pub fn check_outgoing<T: Validate>(value: &T, enabled: bool) -> Result<(), OcpiError> {
    if !enabled {
        return Ok(());
    }
    value.validate().map_err(OcpiError::Invalid)
}

/// Copies the request and correlation IDs onto a response, generating what is missing.
#[must_use]
pub fn echo_ids(request_headers: &HeaderMap) -> RequestIds {
    RequestIds::from_headers_or_generate(request_headers)
}

/// Re-exported for handlers that build their own responses.
pub use http::StatusCode as HttpStatusCode;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::StatusCode;
    use crate::transport::headers::{X_CORRELATION_ID, X_REQUEST_ID};
    use axum::body::to_bytes;

    async fn body_of(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn a_successful_reply_is_a_1000_envelope() {
        let response = OcpiReply::ok(serde_json::json!({"id": "LOC1"})).into_response();
        assert_eq!(response.status(), HttpStatus::OK);
        assert_eq!(response.headers().get(http::header::CONTENT_TYPE).unwrap(), "application/json");
        let body = body_of(response).await;
        assert_eq!(body["status_code"], 1000);
        assert_eq!(body["data"]["id"], "LOC1");
    }

    #[tokio::test]
    async fn a_created_object_is_a_201_with_the_same_envelope() {
        let response = OcpiReply::created(serde_json::json!({})).into_response();
        assert_eq!(response.status(), HttpStatus::CREATED);
        assert_eq!(body_of(response).await["status_code"], 1000);
    }

    #[tokio::test]
    async fn an_unspecified_response_body_leaves_data_unset() {
        let response = OcpiReply::<()>::no_content().into_response();
        let body = body_of(response).await;
        assert_eq!(body["status_code"], 1000);
        assert!(body.get("data").is_none(), "the spec advises leaving `data` unset");
    }

    #[tokio::test]
    async fn errors_that_reached_the_ocpi_layer_are_http_200() {
        let response =
            OcpiError::Decode { path: "/evses/0/status".to_owned(), message: "unknown value".to_owned() }
                .into_response();
        assert_eq!(response.status(), HttpStatus::OK, "an HTTP error code MUST NOT be returned");
        let body = body_of(response).await;
        assert_eq!(body["status_code"], StatusCode::INVALID_PARAMETERS.get());
        assert!(body["status_message"].as_str().unwrap().contains("/evses/0/status"));
    }

    #[tokio::test]
    async fn the_five_transport_level_failures_keep_their_http_status() {
        for (error, expected) in [
            (OcpiError::MalformedJson("nope".into()), 400),
            (OcpiError::Unauthorized("no token".into()), 401),
            (OcpiError::TokenAOutOfScope, 401),
            (OcpiError::NotFound("/locations/1".into()), 404),
            (OcpiError::MethodNotAllowed("already registered".into()), 405),
        ] {
            let response = error.into_response();
            assert_eq!(response.status().as_u16(), expected);
        }
    }

    #[tokio::test]
    async fn the_request_and_correlation_ids_are_echoed() {
        let ids = RequestIds::generate();
        let response = OcpiReply::ok(1u8).with_ids(ids.clone()).into_response();
        assert_eq!(response.headers().get(X_REQUEST_ID).unwrap(), ids.request_id.as_str());
        assert_eq!(response.headers().get(X_CORRELATION_ID).unwrap(), ids.correlation_id.as_str());
    }
}
