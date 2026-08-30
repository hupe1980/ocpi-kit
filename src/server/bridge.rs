//! Serving an older OCPI version from handlers written against the canonical one.
//!
//! [`OcpiRouter`](super::OcpiRouter) takes the version it publishes as an argument; this
//! middleware is what makes that argument mean something. On a router published as 2.2.1 it
//! upgrades each request body into the canonical [`v2_3_0`](crate::v2_3_0) model before a handler
//! sees it, and downgrades the `data` of each response on the way out, so a platform serving both
//! versions writes its handlers once.
//!
//! It is installed only when the version differs, so a canonical router pays nothing, and it acts
//! only on the endpoints whose object actually changed — [`ObjectKind::for_endpoint`] returns
//! `None` for the rest. A version this build has no conversions for is refused by
//! [`OcpiRouter::build`](super::OcpiRouter::build) at start-up.
//!
//! Spec: 2.3.0 §transport_and_format_interface_endpoints, §version_information_endpoint

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode as HttpStatus;

use crate::convert::wire::{ObjectKind, Payload};
use crate::transport::OcpiError;
use crate::{InterfaceRole, ModuleId, VersionNumber};

use super::error::OcpiErrorResponse;
use super::router::OcpiState;

/// The largest body this middleware will buffer in order to translate it.
///
/// A page of Locations is the biggest thing OCPI sends and is comfortably inside this; a body
/// past it is refused rather than held in memory, which is the same bound `axum` would apply.
const MAX_BRIDGED_BODY: usize = 8 * 1024 * 1024;

/// Translates request and response bodies between the version this router publishes and the
/// canonical model its handlers are written against.
pub(crate) async fn translate(State(state): State<Arc<OcpiState>>, request: Request, next: Next) -> Response {
    let theirs = state.version().clone();
    let path = request.uri().path().to_owned();
    let Some((module, interface, below)) = state.endpoint_of(&path) else {
        // `/versions` and the version details are the same objects in every version this bridges,
        // and an unmounted path is about to become a 404 either way.
        return next.run(request).await;
    };

    let request = match rewrite_request(request, &theirs, &module, interface, &below).await {
        Ok(request) => request,
        Err(error) => return OcpiErrorResponse::new(error).into_response(),
    };
    let response = next.run(request).await;
    match rewrite_response(response, &theirs, &module, interface, &below).await {
        Ok(response) => response,
        Err(error) => OcpiErrorResponse::new(error).into_response(),
    }
}

async fn rewrite_request(
    request: Request,
    theirs: &VersionNumber,
    module: &ModuleId,
    interface: InterfaceRole,
    below: &str,
) -> Result<Request, OcpiError> {
    let Some(kind) = ObjectKind::for_endpoint(module, interface, below, Payload::Request) else {
        return Ok(request);
    };
    // A merge patch is not an object and cannot be decoded, converted and re-encoded. It does not
    // have to be: everything the two versions agree about crosses unchanged, and a patch that
    // writes a field they do not is refused with the recovery the specification prescribes.
    if request.method() == http::Method::PATCH {
        return check_patch(request, theirs, kind).await;
    }
    let (parts, body) = request.into_parts();
    let bytes = read(body).await?;
    if bytes.is_empty() {
        return Ok(Request::from_parts(parts, Body::from(bytes)));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| OcpiError::MalformedJson(e.to_string()))?;
    let converted = kind
        .bridge(theirs, &crate::CANONICAL_VERSION, value)
        .map_err(|e| OcpiError::Decode { path: "/".to_owned(), message: e.to_string() })?;
    let bytes = serde_json::to_vec(&converted.value).map_err(|e| OcpiError::MalformedJson(e.to_string()))?;
    Ok(Request::from_parts(parts, Body::from(bytes)))
}

/// Lets a merge patch through when it means the same thing in both versions.
async fn check_patch(
    request: Request,
    theirs: &VersionNumber,
    kind: ObjectKind,
) -> Result<Request, OcpiError> {
    let (parts, body) = request.into_parts();
    let bytes = read(body).await?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| OcpiError::MalformedJson(e.to_string()))?;
    let fields: Vec<&str> =
        value.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
    if !kind.patch_crosses_unchanged(&fields) {
        return Err(OcpiError::Unsupported(format!(
            "this PATCH writes {fields:?}, and a {kind} does not carry {} the same way in OCPI \
             {theirs} as in OCPI {}; GET the object and PUT it back instead, which is the \
             recovery the specification prescribes for a refused PATCH",
            kind.divergent_fields().join(", "),
            crate::CANONICAL_VERSION,
        )));
    }
    Ok(Request::from_parts(parts, Body::from(bytes)))
}

async fn rewrite_response(
    response: Response,
    theirs: &VersionNumber,
    module: &ModuleId,
    interface: InterfaceRole,
    below: &str,
) -> Result<Response, OcpiError> {
    let Some(kind) = ObjectKind::for_endpoint(module, interface, below, Payload::Response) else {
        return Ok(response);
    };
    let (parts, body) = response.into_parts();
    let bytes = read(body).await?;
    if bytes.is_empty() {
        return Ok(Response::from_parts(parts, Body::from(bytes)));
    }
    let mut envelope: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        // Not an OCPI envelope — a 404 from `axum` itself, say. Leave it alone.
        Err(_) => return Ok(Response::from_parts(parts, Body::from(bytes))),
    };
    let Some(data) = envelope.get_mut("data").map(serde_json::Value::take) else {
        return Ok(Response::from_parts(parts, Body::from(bytes)));
    };
    let converted = kind
        .bridge(&crate::CANONICAL_VERSION, theirs, data)
        .map_err(|e| OcpiError::Transport(e.to_string()))?;
    envelope["data"] = converted.value;
    if let Some(note) = converted.lossy.to_status_message() {
        tracing::warn!(ocpi.peer_version = %theirs, ocpi.object = %kind, "{note}");
    }
    let bytes = serde_json::to_vec(&envelope).map_err(|e| OcpiError::Transport(e.to_string()))?;
    let mut parts = parts;
    parts.headers.remove(http::header::CONTENT_LENGTH);
    Ok(Response::from_parts(parts, Body::from(bytes)))
}

async fn read(body: Body) -> Result<axum::body::Bytes, OcpiError> {
    axum::body::to_bytes(body, MAX_BRIDGED_BODY).await.map_err(|e| {
        OcpiError::MalformedJson(format!(
            "could not read a body to translate it between OCPI versions ({}): {e}",
            HttpStatus::PAYLOAD_TOO_LARGE
        ))
    })
}

impl OcpiState {
    /// Which module and interface a request path addresses, and what follows the module segment.
    ///
    /// The router mounts a Receiver interface under
    /// [`ServerConfig::receiver_path_prefix`](super::ServerConfig::receiver_path_prefix) when it
    /// has one; where it does not, the deployment runs one router per role and only one interface
    /// of a module is mounted, so what is mounted settles it.
    pub(crate) fn endpoint_of(&self, path: &str) -> Option<(ModuleId, InterfaceRole, String)> {
        let mut segments = path.split('/').filter(|s| !s.is_empty()).peekable();
        let mut prefixed = false;
        if let Some(prefix) = self.config().receiver_path_prefix.as_deref()
            && segments.peek() == Some(&prefix)
        {
            segments.next();
            prefixed = true;
        }
        let module = ModuleId::from(segments.next()?);
        let below = segments.collect::<Vec<_>>().join("/");
        let interface = if prefixed {
            InterfaceRole::Receiver
        } else if self.mounted().contains(&module, InterfaceRole::Sender) {
            InterfaceRole::Sender
        } else {
            InterfaceRole::Receiver
        };
        Some((module, interface, below))
    }
}
