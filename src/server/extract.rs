//! Axum extractors for the things every OCPI request carries.

use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::response::IntoResponse;
use http::request::Parts;
use serde::de::DeserializeOwned;

use crate::transport::headers::{APPLICATION_JSON, AUTHORIZATION, header_str};
use crate::transport::{CredentialsToken, OcpiError, PageQuery, Patch, Quirks, RequestIds, RoutingHeaders};
use crate::types::PartyRef;

use super::auth::{AuthenticatedPeer, TokenStore};
use super::error::OcpiErrorResponse;

/// The pieces of a request a handler is given.
#[derive(Clone, Debug)]
pub struct RequestContext {
    /// The platform the request was authenticated as.
    pub peer: AuthenticatedPeer,
    /// The request and correlation IDs, to be echoed on the response.
    pub ids: RequestIds,
    /// The routing headers, when the module is a functional one and the peer sent them.
    pub routing: Option<RoutingHeaders>,
}

impl RequestContext {
    /// The routing headers for the response to this request.
    ///
    /// > *Direct response | Receiving platform provider to Requesting platform provider |
    /// > Requesting-party | Receiving-party*
    #[must_use]
    pub fn response_routing(&self, responder: &PartyRef) -> Option<RoutingHeaders> {
        self.routing.as_ref().map(|r| r.response_from(responder.clone()))
    }

    /// The party the request was addressed to, if the peer said.
    #[must_use]
    pub fn addressed_to(&self) -> Option<&PartyRef> {
        self.routing.as_ref().and_then(|r| r.to.as_ref())
    }

    /// The party the request came from, if the peer said.
    #[must_use]
    pub fn from_party(&self) -> Option<&PartyRef> {
        self.routing.as_ref().map(|r| &r.from)
    }
}

/// Extracts and authenticates the credentials token.
///
/// > *If the header is missing or the credentials token doesn't match any known party then the
/// > server SHALL respond with an HTTP `401 - Unauthorized` status code.*
///
/// The router provides the [`TokenStore`] through axum state.
///
/// Spec: 2.3.0 §transport_and_format_authorization_header
#[derive(Clone, Debug)]
pub struct Auth(pub AuthenticatedPeer);

impl<S> FromRequestParts<S> for Auth
where
    S: AuthState,
{
    type Rejection = OcpiErrorResponse;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let ids = RequestIds::from_headers_or_generate(&parts.headers);
        let header = header_str(&parts.headers, &AUTHORIZATION).ok_or_else(|| {
            OcpiErrorResponse::new(OcpiError::Unauthorized("no Authorization header".to_owned()))
                .with_ids(ids.clone())
        })?;

        let token =
            CredentialsToken::parse_header(header, state.quirks().accept_unencoded_token).map_err(|e| {
                OcpiErrorResponse::new(OcpiError::Unauthorized(e.to_string())).with_ids(ids.clone())
            })?;

        state.tokens().resolve(&token).map(Auth).ok_or_else(|| {
            // Deliberately the same message as a missing header: it says nothing about whether
            // the token merely expired or never existed.
            OcpiErrorResponse::new(OcpiError::Unauthorized(
                "the credentials token does not match any known party".to_owned(),
            ))
            .with_ids(ids)
        })
    }
}

/// The `X-Request-ID` and `X-Correlation-ID` of the request.
///
/// Missing IDs are generated rather than refused: they are required of the peer, but failing a
/// request over a missing debugging header would be worse than answering it and echoing what was
/// used.
#[derive(Clone, Debug)]
pub struct Ids(pub RequestIds);

impl<S: Send + Sync> FromRequestParts<S> for Ids {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(RequestIds::from_headers_or_generate(&parts.headers)))
    }
}

/// The `OCPI-to-*` and `OCPI-from-*` headers, when present.
///
/// Absent on a configuration module, and on the request half of an Open Routing Request.
#[derive(Clone, Debug)]
pub struct Routing(pub Option<RoutingHeaders>);

impl<S: Send + Sync> FromRequestParts<S> for Routing {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(RoutingHeaders::from_headers(&parts.headers)))
    }
}

/// The pagination query parameters.
#[derive(Clone, Debug)]
pub struct Page(pub PageQuery);

impl<S: PagePolicy> FromRequestParts<S> for Page {
    type Rejection = OcpiErrorResponse;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(query) = Query::<PageQuery>::from_request_parts(parts, state).await.map_err(|e| {
            OcpiErrorResponse::new(OcpiError::Decode { path: "?".to_owned(), message: e.body_text() })
        })?;
        // The handler is given a limit it can honour literally. A peer asking for `?limit=100000`
        // gets the server's own maximum instead, which is the number `X-Limit` on the way back
        // says it would get — a cap that is only advertised is not a cap, and the alternative is
        // that every handler has to remember to clamp.
        Ok(Self(query.clamped_to(state.max_page_limit())))
    }
}

/// The `{country_code}/{party_id}` pair of a client-owned-object URL.
///
/// Extract this together with [`Auth`] and call
/// [`AuthenticatedPeer::check_ownership`](super::auth::AuthenticatedPeer::check_ownership): a
/// platform may only write under its own party.
#[derive(Clone, Debug)]
pub struct Owner(pub PartyRef);

impl Owner {
    /// Builds an owner from the two path segments.
    ///
    /// # Errors
    ///
    /// Returns [`OcpiError::NotFound`] when the segments are not a usable party reference, which
    /// is the same answer as "no such object" and reveals nothing.
    pub fn from_path(country_code: &str, party_id: &str) -> Result<Self, OcpiError> {
        PartyRef::new(country_code, party_id)
            .map(Self)
            .map_err(|e| OcpiError::NotFound(format!("{country_code}/{party_id}: {e}")))
    }
}

/// A JSON body, decoded with the path to any offending value.
///
/// A body that is not valid JSON is an HTTP 400; a body that is valid JSON but not the object the
/// endpoint expects is a `2001` in a 200. That is exactly the line the specification draws:
///
/// > *The transport layer ends after a message is correctly parsed into a (semantically
/// > unvalidated) JSON structure. When a message does not contain a valid JSON string, the HTTP
/// > error `400 - Bad request` MUST be returned.*
#[derive(Clone, Debug)]
pub struct OcpiJson<T>(pub T);

impl<T, S> FromRequest<S> for OcpiJson<T>
where
    T: DeserializeOwned,
    S: ContentTypePolicy,
{
    type Rejection = OcpiErrorResponse;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let ids = RequestIds::from_headers_or_generate(request.headers());

        if !state.accepts_content_type(request.headers()) {
            return Err(OcpiErrorResponse::new(OcpiError::Decode {
                path: "Content-Type".to_owned(),
                message: format!(
                    "Content-Type SHALL be set to {APPLICATION_JSON} for any request that \
                     contains a message body"
                ),
            })
            .with_ids(ids));
        }

        let bytes = axum::body::Bytes::from_request(request, state)
            .await
            .map_err(|e| OcpiErrorResponse::new(OcpiError::Transport(e.body_text())).with_ids(ids.clone()))?;

        // Distinguish "not JSON at all" (HTTP 400) from "not this object" (2001 in a 200).
        if serde_json::from_slice::<serde::de::IgnoredAny>(&bytes).is_err() {
            return Err(OcpiErrorResponse::new(OcpiError::MalformedJson(
                "the request body is not valid JSON".to_owned(),
            ))
            .with_ids(ids));
        }

        let mut de = serde_json::Deserializer::from_slice(&bytes);
        serde_path_to_error::deserialize(&mut de).map(OcpiJson).map_err(|e| {
            OcpiErrorResponse::new(OcpiError::Decode {
                path: format!("/{}", e.path()),
                message: e.into_inner().to_string(),
            })
            .with_ids(ids)
        })
    }
}

/// A JSON Merge Patch body.
///
/// Refuses a patch with no `last_updated`, which is the specification's own example of a `2001`.
#[derive(Clone, Debug)]
pub struct OcpiPatch<T>(pub Patch<T>);

impl<T, S> FromRequest<S> for OcpiPatch<T>
where
    T: Send,
    S: ContentTypePolicy,
{
    type Rejection = OcpiErrorResponse;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let ids = RequestIds::from_headers_or_generate(request.headers());
        let OcpiJson(value) = OcpiJson::<serde_json::Value>::from_request(request, state).await?;
        let patch = Patch::<T>::from_value(value);
        if patch.last_updated().is_none() {
            return Err(OcpiErrorResponse::new(OcpiError::Decode {
                path: "/last_updated".to_owned(),
                message: "a PATCH must carry `last_updated`".to_owned(),
            })
            .with_ids(ids));
        }
        Ok(Self(patch))
    }
}

/// What [`Auth`] needs from the router's state.
pub trait AuthState: Send + Sync {
    /// The token store to resolve credentials tokens against.
    fn tokens(&self) -> &dyn TokenStore;
    /// The interoperability profile to parse the `Authorization` header with.
    fn quirks(&self) -> &Quirks;
}

/// What [`Page`] needs from the router's state.
///
/// > *`X-Limit`: The maximum number of objects that the server can return.*
///
/// The header is a promise, so the extractor keeps it: a `limit` above this never reaches a
/// handler.
pub trait PagePolicy: Send + Sync {
    /// The largest page this server will return.
    fn max_page_limit(&self) -> u64;
}

/// What [`OcpiJson`] needs from the router's state.
pub trait ContentTypePolicy: Send + Sync {
    /// Whether a request with these headers carries an acceptable `Content-Type`.
    fn accepts_content_type(&self, headers: &http::HeaderMap) -> bool;
}

/// The default reading of the `Content-Type` rule.
///
/// > *The HTTP header: Content-Type SHALL be set to `application/json` for any request that
/// > contains a message body.*
///
/// `application/json; charset=utf-8` is accepted, because it says the same thing and is extremely
/// common. `lenient` additionally accepts an absent or unrelated type, for a peer that cannot be
/// persuaded to set it.
#[must_use]
pub fn accepts_json(headers: &http::HeaderMap, lenient: bool) -> bool {
    let Some(value) = headers.get(http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) else {
        return lenient;
    };
    let base = value.split(';').next().unwrap_or("").trim();
    base.eq_ignore_ascii_case(APPLICATION_JSON) || lenient
}

/// Renders a rejection, so a handler can return one directly.
#[must_use]
pub fn reject(error: OcpiError, ids: RequestIds) -> axum::response::Response {
    OcpiErrorResponse::new(error).with_ids(ids).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(content_type: &str) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_str(content_type).unwrap());
        headers
    }

    #[test]
    fn the_charset_parameter_is_accepted_but_a_wrong_type_is_not() {
        assert!(accepts_json(&headers_with("application/json"), false));
        assert!(accepts_json(&headers_with("application/json; charset=utf-8"), false));
        assert!(accepts_json(&headers_with("APPLICATION/JSON"), false));
        assert!(!accepts_json(&headers_with("text/plain"), false));
        assert!(!accepts_json(&http::HeaderMap::new(), false));
    }

    #[test]
    fn the_lenient_policy_accepts_anything() {
        assert!(accepts_json(&headers_with("text/plain"), true));
        assert!(accepts_json(&http::HeaderMap::new(), true));
    }

    #[test]
    fn an_owner_that_is_not_a_party_reference_is_a_404() {
        assert!(Owner::from_path("NL", "TNM").is_ok());
        let err = Owner::from_path("TOOLONG", "TNM").unwrap_err();
        assert_eq!(err.http_status(), 404);
    }
}
