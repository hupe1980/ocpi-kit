//! Pagination: the query parameters, the three response headers, and the crawl.
//!
//! Spec: 2.3.0 §transport_and_format_pagination

use core::fmt;

use http::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::types::{DateTime, Url};

use super::headers::{LINK, X_LIMIT, X_TOTAL_COUNT, header_str, header_u64, link_next, parse_link_next};

/// The query parameters of a paginated GET.
///
/// > *`date_from`: Only return objects that have `last_updated` after or equal to this Date/Time
/// > (inclusive). `date_to`: … up to this Date/Time, but not including (exclusive).*
///
/// The half-open interval is the point: *"when sequential requests to the same end-point are
/// done, the next interval will have no overlap and the `date_from` of the next interval is
/// simply the `date_to` of the previous interval."* [`PageQuery::next_interval`] does that.
///
/// Spec: 2.3.0 §transport_and_format_paginated_request
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PageQuery {
    /// Only objects with `last_updated` at or after this time. Inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_from: Option<DateTime>,
    /// Only objects with `last_updated` before this time. Exclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_to: Option<DateTime>,
    /// The offset of the first object returned. Absent means 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    /// The maximum number of objects to return. The server may return fewer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

impl PageQuery {
    /// An empty query: everything, from the beginning, at the server's own page size.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything updated at or after `from`.
    #[must_use]
    pub fn since(from: DateTime) -> Self {
        Self { date_from: Some(from), ..Self::default() }
    }

    /// Everything updated in `[from, to)`.
    #[must_use]
    pub fn between(from: DateTime, to: DateTime) -> Self {
        Self { date_from: Some(from), date_to: Some(to), ..Self::default() }
    }

    /// This query with an explicit page size.
    #[must_use]
    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// This query with an explicit offset.
    #[must_use]
    pub fn with_offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// The offset, applying the spec's default of 0.
    #[must_use]
    pub fn offset_or_default(&self) -> u64 {
        self.offset.unwrap_or(0)
    }

    /// The query for the next time interval, starting where this one ended.
    ///
    /// Returns `None` when this query has no `date_to` to continue from.
    #[must_use]
    pub fn next_interval(&self, new_end: DateTime) -> Option<Self> {
        let start = self.date_to?;
        Some(Self { date_from: Some(start), date_to: Some(new_end), offset: None, limit: self.limit })
    }

    /// The query string, with the parameters in the order the spec's examples use.
    ///
    /// Returns an empty string when nothing is set, so it can be appended unconditionally.
    #[must_use]
    pub fn to_query_string(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(offset) = self.offset {
            parts.push(format!("offset={offset}"));
        }
        if let Some(limit) = self.limit {
            parts.push(format!("limit={limit}"));
        }
        if let Some(from) = self.date_from {
            parts.push(format!("date_from={}", encode(&from.to_string())));
        }
        if let Some(to) = self.date_to {
            parts.push(format!("date_to={}", encode(&to.to_string())));
        }
        parts.join("&")
    }

    /// Applies this query to a base URL.
    #[must_use]
    pub fn apply_to(&self, base: &Url) -> Url {
        base.with_query(&self.to_query_string())
    }

    /// Clamps `limit` to `max`, as a peer's advertised `X-Limit` requires.
    #[must_use]
    pub fn clamped_to(mut self, max: u64) -> Self {
        self.limit = Some(self.limit.map_or(max, |l| l.min(max)));
        self
    }
}

/// Percent-encodes the characters a `DateTime` contributes that are unsafe in a query value.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                use core::fmt::Write as _;
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).as_bytes() {
                    let _ = write!(out, "%{byte:02X}");
                }
            }
        }
    }
    out
}

/// The three headers a paginated response carries.
///
/// Spec: 2.3.0 §transport_and_format_paginated_response
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PageMeta {
    /// The URL of the next page, present only when this is not the last page.
    pub next: Option<Url>,
    /// The total number of objects matching the query, excluding `limit` and `offset`.
    pub total_count: Option<u64>,
    /// The maximum number of objects the server will return.
    ///
    /// > *Note that this is an upper limit. If there are not enough remaining objects to return,
    /// > fewer objects than this upper limit number will be returned, X-Limit SHALL then still
    /// > show the upper limit, not the number of objects returned.*
    pub limit: Option<u64>,
}

impl PageMeta {
    /// Reads the pagination headers from a response.
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            next: header_str(headers, &LINK).and_then(parse_link_next),
            total_count: header_u64(headers, &X_TOTAL_COUNT),
            limit: header_u64(headers, &X_LIMIT),
        }
    }

    /// Writes the pagination headers into a response.
    pub fn write_to(&self, headers: &mut HeaderMap) {
        use http::HeaderValue;
        if let Some(next) = &self.next
            && let Ok(v) = HeaderValue::from_str(&link_next(next))
        {
            headers.insert(LINK, v);
        }
        if let Some(total) = self.total_count {
            headers.insert(X_TOTAL_COUNT, HeaderValue::from(total));
        }
        if let Some(limit) = self.limit {
            headers.insert(X_LIMIT, HeaderValue::from(limit));
        }
    }

    /// Whether there is another page to fetch.
    #[must_use]
    pub const fn has_next(&self) -> bool {
        self.next.is_some()
    }
}

/// One page of a list endpoint: the objects and the metadata the headers carry.
#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    /// The objects on this page.
    pub items: Vec<T>,
    /// The pagination headers that came with them.
    pub meta: PageMeta,
}

impl<T> Page<T> {
    /// A page with no next link, for a server returning everything at once.
    #[must_use]
    pub fn single(items: Vec<T>) -> Self {
        let total = items.len() as u64;
        Self { items, meta: PageMeta { next: None, total_count: Some(total), limit: None } }
    }

    /// Whether there is another page to fetch.
    #[must_use]
    pub const fn has_next(&self) -> bool {
        self.meta.has_next()
    }
}

/// What a client should do after a page whose `X-Total-Count` moved.
///
/// > *NOTE: Some query parameters can cause concurrency problems. … While crawling over the pages
/// > one of these objects is updated. The client detects this: `X-Total-Count` will be lower in
/// > the next request. It is advised to redo the previous GET with the `offset` lowered by 1 (if
/// > the `offset` was not 0) and after that continue crawling the 'next' page links.*
///
/// Spec: 2.3.0 §transport_and_format_paginated_response
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrawlAdjustment {
    /// The count is stable or grew; follow the next link as normal.
    ///
    /// > *the client does not have to retry any requests when this happens because only the last
    /// > page will be different.*
    Continue,
    /// The count shrank; re-fetch at this offset before continuing.
    RefetchAt(u64),
}

/// Decides how to continue a crawl after seeing a new total count.
///
/// `previous_offset` is the offset of the page just fetched.
#[must_use]
pub fn crawl_adjustment(
    previous_total: Option<u64>,
    new_total: Option<u64>,
    previous_offset: u64,
) -> CrawlAdjustment {
    match (previous_total, new_total) {
        (Some(before), Some(now)) if now < before && previous_offset > 0 => {
            CrawlAdjustment::RefetchAt(previous_offset - 1)
        }
        _ => CrawlAdjustment::Continue,
    }
}

impl fmt::Display for PageQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let q = self.to_query_string();
        if q.is_empty() { f.write_str("(no filters)") } else { f.write_str(&q) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn dt(s: &str) -> DateTime {
        s.parse().unwrap()
    }

    #[test]
    fn query_strings_match_the_spec_examples() {
        let q = PageQuery::between(dt("2016-01-01T00:00:00Z"), dt("2016-12-31T23:59:59Z"));
        let base = Url::new("https://www.server.com/ocpi/cpo/2.3.0/cdrs/").unwrap();
        assert_eq!(
            q.apply_to(&base).as_str(),
            "https://www.server.com/ocpi/cpo/2.3.0/cdrs/\
             ?date_from=2016-01-01T00%3A00%3A00Z&date_to=2016-12-31T23%3A59%3A59Z"
        );
        assert_eq!(PageQuery::new().apply_to(&base), base, "an empty query changes nothing");
    }

    #[test]
    fn the_next_interval_starts_where_the_last_one_ended() {
        let first = PageQuery::between(dt("2016-01-01T00:00:00Z"), dt("2016-02-01T00:00:00Z"));
        let second = first.next_interval(dt("2016-03-01T00:00:00Z")).unwrap();
        assert_eq!(second.date_from, first.date_to, "half-open intervals do not overlap");
        assert_eq!(second.date_to, Some(dt("2016-03-01T00:00:00Z")));
        assert!(
            PageQuery::since(dt("2016-01-01T00:00:00Z")).next_interval(dt("2016-02-01T00:00:00Z")).is_none()
        );
    }

    #[test]
    fn limits_are_clamped_to_what_the_peer_advertises() {
        assert_eq!(PageQuery::new().with_limit(2000).clamped_to(100).limit, Some(100));
        assert_eq!(PageQuery::new().with_limit(50).clamped_to(100).limit, Some(50));
        assert_eq!(PageQuery::new().clamped_to(100).limit, Some(100));
    }

    #[test]
    fn page_meta_round_trips_through_headers() {
        let meta = PageMeta {
            next: Some(Url::new("https://e.com/cdrs/?offset=150&limit=50").unwrap()),
            total_count: Some(1234),
            limit: Some(50),
        };
        let mut headers = HeaderMap::new();
        meta.write_to(&mut headers);
        assert_eq!(
            headers.get(LINK).unwrap(),
            HeaderValue::from_static(r#"<https://e.com/cdrs/?offset=150&limit=50>; rel="next""#)
        );
        assert_eq!(PageMeta::from_headers(&headers), meta);
    }

    #[test]
    fn a_shrinking_total_count_rewinds_the_crawl_by_one() {
        assert_eq!(crawl_adjustment(Some(1000), Some(999), 150), CrawlAdjustment::RefetchAt(149));
        // Growing is fine: only the last page differs.
        assert_eq!(crawl_adjustment(Some(1000), Some(1001), 150), CrawlAdjustment::Continue);
        // "if the offset was not 0"
        assert_eq!(crawl_adjustment(Some(1000), Some(999), 0), CrawlAdjustment::Continue);
        assert_eq!(crawl_adjustment(None, Some(999), 150), CrawlAdjustment::Continue);
    }

    #[test]
    fn a_last_page_has_no_next_link() {
        let page = Page::single(vec![1, 2, 3]);
        assert!(!page.has_next());
        assert_eq!(page.meta.total_count, Some(3));
    }
}
