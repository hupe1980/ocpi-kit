//! The OCPI response envelope, and the error type that maps onto it.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::types::{DateTime, Validate, Validator, Violations};

use super::status::{StatusClass, StatusCode};

/// The JSON object every OCPI response body is.
///
/// > *The content that is sent with all the response messages is an 'application/json' type and
/// > contains a JSON object with the following properties: `data`, `status_code`,
/// > `status_message`, `timestamp`.*
///
/// # `data` absent, `data: null`, and `data` present
///
/// The specification is deliberately relaxed here:
///
/// > *We advise that in cases where the specification does not explicitly specify what to put in
/// > the `data` field for the response to a certain request, the platform receiving the response
/// > accept both the `data` field being absent and the data field being present with any possible
/// > value. We also advise that in such cases, platform sending the response leave the `data`
/// > field unset.*
///
/// So `data` absent and `data: null` both deserialise to `None`, and a `None` is serialised by
/// leaving the key out.
///
/// ```
/// use ocpi_kit::transport::{OcpiResponse, StatusCode};
///
/// let json = r#"{"status_code":2001,"status_message":"Missing required field: type","timestamp":"2015-06-30T21:59:59Z"}"#;
/// let response: OcpiResponse<String> = serde_json::from_str(json).unwrap();
/// assert_eq!(response.status_code, StatusCode::INVALID_PARAMETERS);
/// assert!(response.data.is_none());
/// assert_eq!(serde_json::to_string(&response).unwrap(), json);
/// ```
///
/// Spec: 2.3.0 §transport_and_format_response_format
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
// `#[serde(default)]` on an `Option<T>` field would otherwise make serde demand `T: Default`,
// which no OCPI object implements — the point of `data` being absent is that there is nothing to
// default to.
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct OcpiResponse<T> {
    /// The response data, when the request succeeded and the endpoint documents a payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// How the request was handled.
    pub status_code: StatusCode,
    /// An optional status message which may help when debugging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    /// The time this message was generated.
    pub timestamp: DateTime,
}

impl<T> OcpiResponse<T> {
    /// A `1000 Generic success` response carrying `data`, timestamped now.
    #[must_use]
    pub fn success(data: T) -> Self {
        Self {
            data: Some(data),
            status_code: StatusCode::SUCCESS,
            status_message: None,
            timestamp: DateTime::now(),
        }
    }

    /// A `1000 Generic success` response with no payload, timestamped now.
    ///
    /// This is the shape the spec advises for a PUT or PATCH whose response body is not
    /// specified.
    #[must_use]
    pub fn success_empty() -> Self {
        Self {
            data: None,
            status_code: StatusCode::SUCCESS,
            status_message: None,
            timestamp: DateTime::now(),
        }
    }

    /// An error response with the given code and message, timestamped now.
    #[must_use]
    pub fn error(status_code: StatusCode, status_message: impl Into<String>) -> Self {
        Self {
            data: None,
            status_code,
            status_message: Some(status_message.into()),
            timestamp: DateTime::now(),
        }
    }

    /// Whether `status_code` is in the success range.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.status_code.is_success()
    }

    /// The payload, or the error the peer reported.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::Remote`] when the status code is not in the `1xxx` range, and
    /// [`OcpiError::MissingData`] when a successful response carried no payload.
    pub fn into_result(self) -> Result<T, OcpiError> {
        if !self.is_success() {
            return Err(OcpiError::Remote {
                status_code: self.status_code,
                status_message: self.status_message,
            });
        }
        self.data.ok_or(OcpiError::MissingData { status_code: self.status_code })
    }

    /// Applies `f` to the payload, keeping the envelope.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> OcpiResponse<U> {
        OcpiResponse {
            data: self.data.map(f),
            status_code: self.status_code,
            status_message: self.status_message,
            timestamp: self.timestamp,
        }
    }
}

impl<T> OcpiResponse<Vec<T>> {
    /// The payload of a list endpoint, treating an absent `data` as an empty list.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::Remote`] when the status code is not in the `1xxx` range.
    pub fn into_list(self) -> Result<Vec<T>, OcpiError> {
        if !self.is_success() {
            return Err(OcpiError::Remote {
                status_code: self.status_code,
                status_message: self.status_message,
            });
        }
        Ok(self.data.unwrap_or_default())
    }
}

impl<T: Validate> Validate for OcpiResponse<T> {
    fn validate_in(&self, v: &mut Validator) {
        v.field("data", &self.data);
        v.field("timestamp", &self.timestamp);
    }
}

/// Everything that can go wrong on an OCPI request, from either side of the wire.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OcpiError {
    /// The peer answered with a non-success OCPI status code.
    #[error("peer returned OCPI status {status_code}{}", format_message(.status_message.as_ref()))]
    Remote {
        /// The code the peer sent.
        status_code: StatusCode,
        /// The message the peer sent, if any.
        status_message: Option<String>,
    },

    /// A successful response did not carry the payload the endpoint documents.
    #[error("peer returned OCPI status {status_code} but no data")]
    MissingData {
        /// The code the peer sent.
        status_code: StatusCode,
    },

    /// The request body was not valid JSON, so it never reached the OCPI layer.
    ///
    /// > *When a message does not contain a valid JSON string, the HTTP error `400 - Bad request`
    /// > MUST be returned.*
    #[error("malformed JSON: {0}")]
    MalformedJson(String),

    /// The body was valid JSON but did not fit the OCPI object it was supposed to be.
    ///
    /// `path` is the JSON path to the offending value, which is what turns a support ticket into
    /// a one-line fix.
    #[error("cannot decode {path}: {message}")]
    Decode {
        /// JSON path to the value that did not decode.
        path: String,
        /// What went wrong there.
        message: String,
    },

    /// A decoded object broke rules of the specification.
    #[error("object does not conform to the specification: {0}")]
    Invalid(#[from] Violations),

    /// No credentials token, or one that matches no known party.
    ///
    /// > *If the header is missing or the credentials token doesn't match any known party then
    /// > the server SHALL respond with an HTTP `401 - Unauthorized` status code.*
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// `CREDENTIALS_TOKEN_A` was used on a module other than `credentials` or `versions`.
    ///
    /// > *the server SHALL respond with an HTTP `401 - Unauthorized` status code.*
    #[error("CREDENTIALS_TOKEN_A may only be used on the credentials and versions modules")]
    TokenAOutOfScope,

    /// A GET addressed a resource that does not exist.
    ///
    /// > *In case of a GET request, when the resource does NOT exist, the server SHOULD return a
    /// > HTTP `404 - Not Found`.*
    #[error("not found: {0}")]
    NotFound(String),

    /// The HTTP method is not allowed in the current registration state.
    ///
    /// The credentials module uses this: POST when already registered, PUT or DELETE when not.
    #[error("method not allowed: {0}")]
    MethodNotAllowed(String),

    /// The transport failed: connection refused, TLS failure, non-JSON body.
    ///
    /// A **timeout** is [`OcpiError::Timeout`] instead, because a hub has to answer the two
    /// differently and telling them apart by reading the message is not something a protocol
    /// implementation should be doing.
    #[error("transport error: {0}")]
    Transport(String),

    /// The peer did not answer within the configured timeout.
    ///
    /// A hub turns this into `4002 Timeout on forwarded request`, which is the code that exists
    /// precisely for it — and the reason this is a variant of its own rather than a
    /// [`Transport`](Self::Transport) whose message happens to contain the word.
    #[error("timed out: {0}")]
    Timeout(String),

    /// A hub was asked to route a request whose headers and method do not describe any of the
    /// scenarios the specification defines.
    ///
    /// The clearest example is a `GET` addressed to the hub's own party on a Receiver interface:
    /// the `OCPI-to-` headers say Broadcast Push, but *"GET SHALL NOT be used in combination
    /// with Broadcast Push"*, and the sender is told to use an Open Routing Request instead.
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing
    #[error("cannot route this request: {0}")]
    NotRoutable(String),

    /// This build cannot carry a document between the two OCPI versions involved.
    ///
    /// A client whose peer speaks a version this crate has no conversions for, or a merge patch
    /// that writes a field the two versions disagree about. It is a `3000` rather than a `2001`
    /// because nothing about the request is wrong: the software simply cannot do it.
    #[error("not supported by this build: {0}")]
    Unsupported(String),

    /// A URL was refused by the configured [`UrlPolicy`](crate::types::UrlPolicy).
    #[error("refused to call {url}: {reason}")]
    UrlRefused {
        /// The URL that was refused.
        url: String,
        /// Why it was refused.
        reason: String,
    },
}

fn format_message(message: Option<&String>) -> String {
    message.map_or_else(String::new, |m| format!(": {m}"))
}

impl OcpiError {
    /// The OCPI status code this error should be reported as.
    #[must_use]
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Remote { status_code, .. } | Self::MissingData { status_code } => *status_code,
            Self::MalformedJson(_) | Self::Decode { .. } | Self::Invalid(_) | Self::NotRoutable(_) => {
                StatusCode::INVALID_PARAMETERS
            }
            Self::Unauthorized(_)
            | Self::TokenAOutOfScope
            | Self::NotFound(_)
            | Self::MethodNotAllowed(_) => StatusCode::CLIENT_ERROR,
            Self::Transport(_) | Self::Timeout(_) | Self::UrlRefused { .. } | Self::Unsupported(_) => {
                StatusCode::SERVER_ERROR
            }
        }
    }

    /// The HTTP status code this error should be answered with.
    ///
    /// This encodes the whole of §status_codes: the only cases that get an HTTP error are the
    /// ones the spec explicitly names. **Everything that reached the OCPI layer is HTTP 200 with
    /// a `2xxx`/`3xxx`/`4xxx` `status_code` in the body.**
    ///
    /// | Situation | HTTP |
    /// |---|---|
    /// | body is not valid JSON | `400` |
    /// | missing or unknown credentials token | `401` |
    /// | `CREDENTIALS_TOKEN_A` outside `credentials`/`versions` | `401` |
    /// | GET of a resource that does not exist | `404` |
    /// | credentials POST when registered, PUT/DELETE when not | `405` |
    /// | anything else | `200` |
    ///
    /// Spec: 2.3.0 §status_codes_status_codes
    #[must_use]
    pub const fn http_status(&self) -> u16 {
        match self {
            Self::MalformedJson(_) => 400,
            Self::Unauthorized(_) | Self::TokenAOutOfScope => 401,
            Self::NotFound(_) => 404,
            Self::MethodNotAllowed(_) => 405,
            _ => 200,
        }
    }

    /// Whether retrying the same request could plausibly succeed.
    ///
    /// The spec forbids automatically retrying a write:
    ///
    /// > *OCPI messages SHOULD NOT be queued. When a client does a POST, PUT or PATCH request and
    /// > that request fails or times out, the client should not queue the message and retry the
    /// > same message again later.*
    ///
    /// So this is only ever consulted for GETs; see
    /// [`RetryPolicy`](crate::client::RetryPolicy).
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Transport(_) | Self::Timeout(_) => true,
            Self::Remote { status_code, .. } => {
                matches!(status_code.class(), StatusClass::ServerError | StatusClass::HubError)
            }
            _ => false,
        }
    }

    /// Renders this error as the envelope a server should send back.
    #[must_use]
    pub fn to_response<T>(&self) -> OcpiResponse<T> {
        OcpiResponse::error(self.status_code(), self.to_string())
    }
}

impl fmt::Display for OcpiResponse<()> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.status_code)?;
        if let Some(m) = &self.status_message {
            write!(f, ": {m}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_and_absent_data_both_mean_none() {
        let absent: OcpiResponse<String> =
            serde_json::from_str(r#"{"status_code":1000,"timestamp":"2015-06-30T21:59:59Z"}"#).unwrap();
        let null: OcpiResponse<String> =
            serde_json::from_str(r#"{"data":null,"status_code":1000,"timestamp":"2015-06-30T21:59:59Z"}"#)
                .unwrap();
        assert_eq!(absent.data, None);
        assert_eq!(null.data, None);
        // Round-tripping a `null` normalises it to an absent key, as the spec advises.
        assert_eq!(
            serde_json::to_string(&null).unwrap(),
            r#"{"status_code":1000,"timestamp":"2015-06-30T21:59:59Z"}"#
        );
    }

    #[test]
    fn a_list_endpoint_treats_absent_data_as_empty() {
        let r: OcpiResponse<Vec<String>> =
            serde_json::from_str(r#"{"status_code":1000,"timestamp":"2015-06-30T21:59:59Z"}"#).unwrap();
        assert_eq!(r.into_list().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn into_result_surfaces_the_peers_error() {
        let r: OcpiResponse<String> = serde_json::from_str(
            r#"{"status_code":2001,"status_message":"Missing required field: type","timestamp":"2015-06-30T21:59:59Z"}"#,
        )
        .unwrap();
        let err = r.into_result().unwrap_err();
        assert_eq!(err.status_code(), StatusCode::INVALID_PARAMETERS);
        assert!(err.to_string().contains("Missing required field"), "{err}");
        assert!(!err.is_transient());
    }

    #[test]
    fn http_status_mapping_matches_the_spec_table() {
        assert_eq!(OcpiError::MalformedJson("x".into()).http_status(), 400);
        assert_eq!(OcpiError::Unauthorized("x".into()).http_status(), 401);
        assert_eq!(OcpiError::TokenAOutOfScope.http_status(), 401);
        assert_eq!(OcpiError::NotFound("x".into()).http_status(), 404);
        assert_eq!(OcpiError::MethodNotAllowed("x".into()).http_status(), 405);
        // Everything that reached the OCPI layer is a 200 with an OCPI status code in the body.
        assert_eq!(OcpiError::Decode { path: "/evses/0".into(), message: "nope".into() }.http_status(), 200);
        let unroutable = OcpiError::NotRoutable("GET is not a Broadcast Push".into());
        assert_eq!(unroutable.http_status(), 200);
        assert_eq!(unroutable.status_code(), StatusCode::INVALID_PARAMETERS);
        assert!(!unroutable.is_transient());
        assert_eq!(OcpiError::Transport("timeout".into()).http_status(), 200);
        assert_eq!(
            OcpiError::Remote { status_code: StatusCode::HUB_ERROR, status_message: None }.http_status(),
            200
        );
    }

    #[test]
    fn server_and_hub_errors_are_transient_client_errors_are_not() {
        let transient =
            OcpiError::Remote { status_code: StatusCode::CONNECTION_PROBLEM, status_message: None };
        assert!(transient.is_transient());
        let permanent = OcpiError::Remote { status_code: StatusCode::UNKNOWN_TOKEN, status_message: None };
        assert!(!permanent.is_transient());
        assert!(OcpiError::Transport("reset".into()).is_transient());
    }
}
