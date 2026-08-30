//! Crawling a paginated list endpoint, with the concurrency correction the spec asks for.

use std::collections::VecDeque;

use http::Method;
use serde::de::DeserializeOwned;

use crate::ModuleId;
use crate::transport::{CrawlAdjustment, OcpiError, OcpiRequest, PageMeta, RoutingHeaders, crawl_adjustment};
use crate::types::Url;

use super::http::Transport;
use super::peer::Peer;

/// An asynchronous crawl over every page of a list endpoint.
///
/// Following the `Link: <…>; rel="next"` header is only most of the job. The specification also
/// describes what to do when the result set changes underneath the crawl:
///
/// > *While a client crawls over the pages … a new object might be created on the server. The
/// > client detects this: the `X-Total-Count` will be higher on the next call. Even so, the client
/// > does not have to retry any requests when this happens because only the last page will be
/// > different.*
///
/// > *When there are for example 1000 objects matching a query … while crawling over the pages one
/// > of these objects is updated. The client detects this: `X-Total-Count` will be lower in the
/// > next request. It is advised to redo the previous GET with the `offset` lowered by 1 (if the
/// > `offset` was not 0) and after that continue crawling the 'next' page links.*
///
/// [`PageStream`] does both, and reports the correction it made through
/// [`PageStream::corrections`] so a pull that keeps shifting is visible rather than silent.
///
/// ```no_run
/// # use ocpi_kit::client::PageStream;
/// # use ocpi_kit::v2_3_0::locations::Location;
/// # async fn crawl(mut stream: PageStream<'_, Location>) -> Result<(), Box<dyn std::error::Error>> {
/// while let Some(location) = stream.next().await? {
///     println!("{}", location.id);
/// }
/// println!("{} objects over {} pages", stream.seen(), stream.pages_fetched());
/// # Ok(())
/// # }
/// ```
///
/// Spec: 2.3.0 §transport_and_format_paginated_response
pub struct PageStream<'a, T> {
    transport: &'a Transport,
    peer: &'a Peer,
    module: ModuleId,
    routing: RoutingHeaders,
    next: Option<Url>,
    buffer: VecDeque<T>,
    last_total: Option<u64>,
    last_offset: u64,
    pages: usize,
    seen: usize,
    corrections: usize,
    max_pages: usize,
}

/// The number of pages a crawl will fetch before giving up, unless configured otherwise.
///
/// A peer that answers every page with a `Link` to itself would otherwise loop forever.
pub const DEFAULT_MAX_PAGES: usize = 10_000;

impl<'a, T: DeserializeOwned> PageStream<'a, T> {
    /// Starts a crawl at `first`.
    #[must_use]
    pub fn new(
        transport: &'a Transport,
        peer: &'a Peer,
        module: ModuleId,
        routing: RoutingHeaders,
        first: Url,
    ) -> Self {
        Self {
            transport,
            peer,
            module,
            routing,
            next: Some(first),
            buffer: VecDeque::new(),
            last_total: None,
            last_offset: 0,
            pages: 0,
            seen: 0,
            corrections: 0,
            max_pages: DEFAULT_MAX_PAGES,
        }
    }

    /// Caps how many pages this crawl will fetch.
    #[must_use]
    pub const fn with_max_pages(mut self, max_pages: usize) -> Self {
        self.max_pages = max_pages;
        self
    }

    /// The next object, fetching another page when the buffer runs dry.
    ///
    /// # Errors
    ///
    /// Propagates transport, decoding and OCPI-level errors from the page fetch.
    pub async fn next(&mut self) -> Result<Option<T>, OcpiError> {
        loop {
            if let Some(item) = self.buffer.pop_front() {
                self.seen += 1;
                return Ok(Some(item));
            }
            let Some(url) = self.next.take() else { return Ok(None) };
            if self.pages >= self.max_pages {
                return Err(OcpiError::Transport(format!(
                    "pagination did not terminate after {} pages; the peer keeps returning a \
                     `Link` header",
                    self.max_pages
                )));
            }
            self.fetch(url).await?;
        }
    }

    /// Collects the whole list.
    ///
    /// # Errors
    ///
    /// Propagates any error from [`PageStream::next`].
    pub async fn collect_all(mut self) -> Result<Vec<T>, OcpiError> {
        let mut out = Vec::new();
        while let Some(item) = self.next().await? {
            out.push(item);
        }
        Ok(out)
    }

    /// How many pages have been fetched.
    #[must_use]
    pub const fn pages_fetched(&self) -> usize {
        self.pages
    }

    /// How many objects have been yielded.
    #[must_use]
    pub const fn seen(&self) -> usize {
        self.seen
    }

    /// How many times the crawl was rewound because `X-Total-Count` shrank.
    ///
    /// A non-zero count means objects were changing while the crawl ran.
    #[must_use]
    pub const fn corrections(&self) -> usize {
        self.corrections
    }

    /// The total the peer reported for the query, from the most recent page.
    #[must_use]
    pub const fn total_count(&self) -> Option<u64> {
        self.last_total
    }

    async fn fetch(&mut self, url: Url) -> Result<(), OcpiError> {
        let offset = offset_of(&url);
        let request =
            OcpiRequest::new(Method::GET, url.clone(), self.module.clone()).routed(self.routing.clone());
        let page = self.transport.send_page::<T>(&request, self.peer.token(), self.peer.quirks()).await?;
        self.pages += 1;

        match crawl_adjustment(self.last_total, page.meta.total_count, self.last_offset) {
            CrawlAdjustment::RefetchAt(new_offset) => {
                self.corrections += 1;
                tracing::debug!(
                    previous_total = self.last_total,
                    new_total = page.meta.total_count,
                    new_offset,
                    "X-Total-Count shrank mid-crawl; rewinding one object as the spec advises",
                );
                self.last_total = page.meta.total_count;
                self.next = Some(with_offset(&url, new_offset));
                self.last_offset = new_offset;
                Ok(())
            }
            CrawlAdjustment::Continue => {
                self.last_total = page.meta.total_count;
                self.last_offset = offset;
                self.buffer.extend(page.items);
                self.next = next_url(&page.meta);
                Ok(())
            }
        }
    }
}

fn next_url(meta: &PageMeta) -> Option<Url> {
    meta.next.clone()
}

/// Reads the `offset` query parameter of a page URL, defaulting to 0.
fn offset_of(url: &Url) -> u64 {
    url.as_str()
        .split_once('?')
        .map(|(_, query)| query)
        .and_then(|query| {
            query.split('&').find_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                (key == "offset").then(|| value.parse().ok())?
            })
        })
        .unwrap_or(0)
}

/// Replaces or adds the `offset` query parameter.
fn with_offset(url: &Url, offset: u64) -> Url {
    let (base, query) = match url.as_str().split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url.as_str(), None),
    };
    let mut parts: Vec<String> = query
        .map(|q| q.split('&').filter(|pair| !pair.starts_with("offset=")).map(ToOwned::to_owned).collect())
        .unwrap_or_default();
    parts.insert(0, format!("offset={offset}"));
    Url::new_lenient(format!("{base}?{}", parts.join("&")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_is_read_from_and_written_to_the_query() {
        let url = Url::new("https://e.com/cdrs?offset=150&limit=50").unwrap();
        assert_eq!(offset_of(&url), 150);
        assert_eq!(with_offset(&url, 149).as_str(), "https://e.com/cdrs?offset=149&limit=50");

        let bare = Url::new("https://e.com/cdrs").unwrap();
        assert_eq!(offset_of(&bare), 0);
        assert_eq!(with_offset(&bare, 10).as_str(), "https://e.com/cdrs?offset=10");
    }

    #[test]
    fn other_filters_survive_a_rewind() {
        // "The Link should also contain any filters present in the original request."
        let url =
            Url::new("https://e.com/cdrs?offset=100&limit=100&date_from=2016-01-01T00%3A00%3A00Z").unwrap();
        assert_eq!(
            with_offset(&url, 99).as_str(),
            "https://e.com/cdrs?offset=99&limit=100&date_from=2016-01-01T00%3A00%3A00Z"
        );
    }
}
