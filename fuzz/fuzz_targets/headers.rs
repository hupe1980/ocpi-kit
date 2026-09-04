//! Parse the HTTP headers a peer controls.
//!
//! `Authorization`, `Link`, and the three pagination headers. Every one of them is read before the
//! request is authenticated, which makes them the earliest peer-controlled parser in the stack.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ocpi_kit::transport::{CredentialsToken, PageMeta, PageQuery};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };

    for lenient in [true, false] {
        let _ = CredentialsToken::parse_header(text, lenient);
    }
    let _ = ocpi_kit::transport::headers::parse_link_next(text);

    let mut headers = http::HeaderMap::new();
    if let Ok(value) = http::HeaderValue::from_str(text) {
        for name in [
            http::HeaderName::from_static("link"),
            http::HeaderName::from_static("x-total-count"),
            http::HeaderName::from_static("x-limit"),
            http::HeaderName::from_static("x-request-id"),
            http::HeaderName::from_static("x-correlation-id"),
        ] {
            headers.insert(name, value.clone());
        }
    }
    let meta = PageMeta::from_headers(&headers);
    let mut out = http::HeaderMap::new();
    meta.write_to(&mut out);

    // And the query string, which comes back off a peer's `Link` header.
    let query: PageQuery = serde_urlencoded_lite(text);
    let _ = query.to_query_string();
});

/// A `PageQuery` from whatever key/value pairs the input happens to contain.
fn serde_urlencoded_lite(text: &str) -> PageQuery {
    let mut query = PageQuery::new();
    for pair in text.split('&') {
        let Some((key, value)) = pair.split_once('=') else { continue };
        match key {
            "offset" => query.offset = value.parse().ok(),
            "limit" => query.limit = value.parse().ok(),
            "date_from" => query.date_from = value.parse().ok(),
            "date_to" => query.date_to = value.parse().ok(),
            _ => {}
        }
    }
    query
}
