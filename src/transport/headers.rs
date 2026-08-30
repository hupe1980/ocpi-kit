//! The HTTP headers OCPI defines, as typed values.
//!
//! > *NOTE: HTTP header names are case-insensitive*
//!
//! Every name here is a [`http::HeaderName`] constant, so case is handled by the `http` crate and
//! never by string comparison.

use core::fmt;

use http::{HeaderMap, HeaderName, HeaderValue};

use crate::types::{PartyRef, Url};

/// `Authorization` — the credentials token. See [`CredentialsToken`](super::CredentialsToken).
pub const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");

/// `X-Request-ID` — unique per request; the response repeats it.
///
/// > *Every request SHALL contain a unique request ID, the response to this request SHALL contain
/// > the same ID.*
pub const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// `X-Correlation-ID` — unique per logical exchange; survives hub forwarding.
///
/// > *Every request/response SHALL contain a unique correlation ID, every response to this
/// > request SHALL contain the same ID.*
pub const X_CORRELATION_ID: HeaderName = HeaderName::from_static("x-correlation-id");

/// `OCPI-to-party-id` — party ID of the connected party this message is to be sent to.
pub const OCPI_TO_PARTY_ID: HeaderName = HeaderName::from_static("ocpi-to-party-id");
/// `OCPI-to-country-code` — country code of the party this message is to be sent to.
pub const OCPI_TO_COUNTRY_CODE: HeaderName = HeaderName::from_static("ocpi-to-country-code");
/// `OCPI-from-party-id` — party ID of the party this message is sent from.
pub const OCPI_FROM_PARTY_ID: HeaderName = HeaderName::from_static("ocpi-from-party-id");
/// `OCPI-from-country-code` — country code of the party this message is sent from.
pub const OCPI_FROM_COUNTRY_CODE: HeaderName = HeaderName::from_static("ocpi-from-country-code");

/// `Link` — the link to the next page of a paginated GET.
pub const LINK: HeaderName = HeaderName::from_static("link");
/// `X-Total-Count` — the total number of objects matching a paginated query.
pub const X_TOTAL_COUNT: HeaderName = HeaderName::from_static("x-total-count");
/// `X-Limit` — the maximum number of objects the server will return per page.
pub const X_LIMIT: HeaderName = HeaderName::from_static("x-limit");

/// `Location` — where a newly POSTed CDR can be retrieved.
pub const LOCATION: HeaderName = HeaderName::from_static("location");

/// The `Content-Type` OCPI bodies use.
///
/// > *The HTTP header: Content-Type SHALL be set to `application/json` for any request that
/// > contains a message body: POST, PUT and PATCH.*
pub const APPLICATION_JSON: &str = "application/json";

/// The pair of IDs every OCPI request and response carries.
///
/// > *For debugging issues, OCPI implementations are required to include unique IDs via HTTP
/// > headers in every request/response.*
///
/// The distinction matters at a hub:
///
/// > *When a Hub forwards a request to a party, the request to this party SHALL contain a **new**
/// > unique value in the X-Request-ID HTTP header, not a copy … the request SHALL contain the
/// > **same** X-Correlation-ID HTTP header.*
///
/// [`RequestIds::forwarded`] does exactly that, so a hub cannot get it backwards.
///
/// ```
/// use ocpi_kit::transport::RequestIds;
///
/// let incoming = RequestIds::generate();
/// let forwarded = incoming.forwarded();
/// assert_ne!(incoming.request_id, forwarded.request_id);
/// assert_eq!(incoming.correlation_id, forwarded.correlation_id);
/// ```
///
/// Spec: 2.3.0 §transport_and_format_unique_messageg_ids
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RequestIds {
    /// Unique per request hop.
    pub request_id: String,
    /// Unique per logical exchange, preserved across hub hops.
    pub correlation_id: String,
}

impl RequestIds {
    /// A fresh pair of UUIDs.
    ///
    /// > *It is advised to used GUID/UUID as values for X-Request-ID and X-Correlation-ID.*
    #[must_use]
    pub fn generate() -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            correlation_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// The IDs a hub must use when forwarding this request: a new request ID, the same
    /// correlation ID.
    #[must_use]
    pub fn forwarded(&self) -> Self {
        Self { request_id: uuid::Uuid::new_v4().to_string(), correlation_id: self.correlation_id.clone() }
    }

    /// Reads the pair from a header map, generating whichever half is missing.
    ///
    /// A missing ID is a spec violation on the peer's side, but refusing the request over it
    /// would be worse than carrying on with a generated one; the server echoes what it used.
    #[must_use]
    pub fn from_headers_or_generate(headers: &HeaderMap) -> Self {
        Self {
            request_id: header_str(headers, &X_REQUEST_ID)
                .map_or_else(|| uuid::Uuid::new_v4().to_string(), ToOwned::to_owned),
            correlation_id: header_str(headers, &X_CORRELATION_ID)
                .map_or_else(|| uuid::Uuid::new_v4().to_string(), ToOwned::to_owned),
        }
    }

    /// Reads the pair from a header map, or `None` if either is absent.
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        Some(Self {
            request_id: header_str(headers, &X_REQUEST_ID)?.to_owned(),
            correlation_id: header_str(headers, &X_CORRELATION_ID)?.to_owned(),
        })
    }

    /// Writes both headers into `headers`, replacing any existing values.
    pub fn write_to(&self, headers: &mut HeaderMap) {
        if let Ok(v) = HeaderValue::from_str(&self.request_id) {
            headers.insert(X_REQUEST_ID, v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.correlation_id) {
            headers.insert(X_CORRELATION_ID, v);
        }
    }
}

impl fmt::Display for RequestIds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "request={} correlation={}", self.request_id, self.correlation_id)
    }
}

/// Reads a header as a string, ignoring values that are not valid UTF-8.
#[must_use]
pub fn header_str<'a>(headers: &'a HeaderMap, name: &HeaderName) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

/// Reads a header as an integer.
#[must_use]
pub fn header_u64(headers: &HeaderMap, name: &HeaderName) -> Option<u64> {
    header_str(headers, name)?.trim().parse().ok()
}

/// Reads the `country_code`/`party_id` pair under `country_header` and `party_header`.
///
/// Returns `None` unless both are present and well-formed, which is what an
/// [Open Routing Request](super::routing) looks like on the `to` side.
#[must_use]
pub fn header_party(
    headers: &HeaderMap,
    country_header: &HeaderName,
    party_header: &HeaderName,
) -> Option<PartyRef> {
    let country = header_str(headers, country_header)?;
    let party = header_str(headers, party_header)?;
    PartyRef::new(country, party).ok()
}

/// Builds a `Link: <url>; rel="next"` header value.
///
/// Spec: 2.3.0 §transport_and_format_pagination_examples
#[must_use]
pub fn link_next(url: &Url) -> String {
    format!("<{}>; rel=\"next\"", url.as_str())
}

/// Extracts the `rel="next"` URL from a `Link` header value.
///
/// Handles a header carrying several links, and tolerates the unquoted `rel=next` form that some
/// implementations emit.
///
/// ```
/// use ocpi_kit::transport::parse_link_next;
///
/// let header = r#"<https://e.com/cdrs/?offset=150&limit=50>; rel="next""#;
/// assert_eq!(parse_link_next(header).unwrap().as_str(), "https://e.com/cdrs/?offset=150&limit=50");
/// assert!(parse_link_next(r#"<https://e.com/a>; rel="prev""#).is_none());
/// ```
#[must_use]
pub fn parse_link_next(value: &str) -> Option<Url> {
    for entry in split_link_entries(value) {
        let mut parts = entry.split(';');
        let target = parts.next()?.trim();
        let url = target.strip_prefix('<')?.strip_suffix('>')?;
        for param in parts {
            let param = param.trim();
            let Some((key, val)) = param.split_once('=') else { continue };
            if key.trim().eq_ignore_ascii_case("rel") {
                let val = val.trim().trim_matches('"');
                if val.eq_ignore_ascii_case("next") {
                    return Some(Url::new_lenient(url));
                }
            }
        }
    }
    None
}

/// Splits a `Link` header on the commas that separate entries, ignoring commas inside `<...>`.
fn split_link_entries(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in value.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(value[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(value[start..].trim());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hub_renews_the_request_id_and_keeps_the_correlation_id() {
        let incoming = RequestIds::generate();
        let forwarded = incoming.forwarded();
        assert_ne!(incoming.request_id, forwarded.request_id);
        assert_eq!(incoming.correlation_id, forwarded.correlation_id);
    }

    #[test]
    fn headers_round_trip_through_a_header_map() {
        let ids = RequestIds::generate();
        let mut headers = HeaderMap::new();
        ids.write_to(&mut headers);
        assert_eq!(RequestIds::from_headers(&headers), Some(ids));
    }

    #[test]
    fn missing_ids_are_generated_rather_than_refused() {
        let ids = RequestIds::from_headers_or_generate(&HeaderMap::new());
        assert!(!ids.request_id.is_empty() && !ids.correlation_id.is_empty());
        assert_ne!(ids.request_id, ids.correlation_id);
    }

    #[test]
    fn link_headers_round_trip_and_tolerate_the_unquoted_form() {
        let url = Url::new("https://www.server.com/ocpi/cpo/2.3.0/cdrs/?offset=150&limit=50").unwrap();
        let header = link_next(&url);
        assert_eq!(
            header,
            r#"<https://www.server.com/ocpi/cpo/2.3.0/cdrs/?offset=150&limit=50>; rel="next""#
        );
        assert_eq!(parse_link_next(&header), Some(url.clone()));
        assert_eq!(
            parse_link_next(r"<https://e.com/a>; rel=next"),
            Some(Url::new_lenient("https://e.com/a"))
        );
    }

    #[test]
    fn link_parsing_picks_next_out_of_several_entries() {
        let header = r#"<https://e.com/a?x=1,2>; rel="prev", <https://e.com/b>; rel="next""#;
        assert_eq!(parse_link_next(header), Some(Url::new_lenient("https://e.com/b")));
        assert_eq!(parse_link_next("garbage"), None);
    }

    #[test]
    fn party_headers_need_both_halves() {
        let mut headers = HeaderMap::new();
        headers.insert(OCPI_TO_COUNTRY_CODE, HeaderValue::from_static("NL"));
        assert_eq!(header_party(&headers, &OCPI_TO_COUNTRY_CODE, &OCPI_TO_PARTY_ID), None);
        headers.insert(OCPI_TO_PARTY_ID, HeaderValue::from_static("TNM"));
        assert_eq!(
            header_party(&headers, &OCPI_TO_COUNTRY_CODE, &OCPI_TO_PARTY_ID),
            Some(PartyRef::new("NL", "TNM").unwrap())
        );
    }
}
