//! In-memory object stores, with pagination that behaves the way the specification describes.
//!
//! These are what the `testkit` exists for: a working Locations Sender in three lines, so a test
//! can be about the thing it is testing rather than about a fake database.
//!
//! The pagination is not a stub. It honours `date_from`/`date_to` as the half-open interval the
//! spec defines, applies `offset` and `limit`, orders oldest-first as the spec advises —
//! *"It is best practice to return the oldest objects first"* — and reports `X-Total-Count`
//! excluding `limit` and `offset`, which is the part implementations most often get wrong.

use std::sync::RwLock;

use crate::transport::{Page, PageMeta, PageQuery};
use crate::types::{DateTime, Url};

/// An object that a store can hold: it has an id and a `last_updated`.
pub trait Stored: Clone + Send + Sync + 'static {
    /// The id this object is keyed by, compared case-insensitively.
    fn key(&self) -> String;
    /// The `last_updated` the pagination filters on.
    fn last_updated(&self) -> DateTime;
}

/// A store of one object type.
#[derive(Debug, Default)]
pub struct InMemoryStore<T> {
    items: RwLock<Vec<T>>,
    max_page: usize,
}

impl<T: Stored> InMemoryStore<T> {
    /// An empty store with a page size of 100.
    #[must_use]
    pub fn new() -> Self {
        Self { items: RwLock::new(Vec::new()), max_page: 100 }
    }

    /// An empty store with a specific page size, for testing a crawl over several pages.
    #[must_use]
    pub fn with_page_size(max_page: usize) -> Self {
        Self { items: RwLock::new(Vec::new()), max_page: max_page.max(1) }
    }

    /// Inserts or replaces an object, keyed case-insensitively by its id.
    ///
    /// Returns whether the object was newly created, which is what decides between HTTP 201 and
    /// HTTP 200 on a `PUT`.
    pub fn put(&self, item: T) -> bool {
        let mut items = self.items.write().expect("store lock poisoned");
        let key = item.key().to_ascii_lowercase();
        if let Some(index) = items.iter().position(|existing| existing.key().to_ascii_lowercase() == key) {
            items[index] = item;
            false
        } else {
            items.push(item);
            true
        }
    }

    /// Fetches an object by id, comparing case-insensitively.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<T> {
        let items = self.items.read().expect("store lock poisoned");
        items.iter().find(|item| item.key().eq_ignore_ascii_case(key)).cloned()
    }

    /// Removes an object.
    pub fn remove(&self, key: &str) -> bool {
        let mut items = self.items.write().expect("store lock poisoned");
        let before = items.len();
        items.retain(|item| !item.key().eq_ignore_ascii_case(key));
        items.len() != before
    }

    /// How many objects are stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.read().expect("store lock poisoned").len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every object, oldest first.
    #[must_use]
    pub fn all(&self) -> Vec<T> {
        let mut items = self.items.read().expect("store lock poisoned").clone();
        items.sort_by_key(Stored::last_updated);
        items
    }

    /// One page, applying the query the way the specification defines it.
    ///
    /// `base` is this endpoint's own URL, used to build the `Link` header of the next page — which
    /// *"should also contain any filters present in the original request"*.
    #[must_use]
    pub fn page(&self, query: &PageQuery, base: &Url) -> Page<T> {
        let matching: Vec<T> = self
            .all()
            .into_iter()
            .filter(|item| {
                let updated = item.last_updated();
                // "date_from is inclusive and date_to exclusive"
                query.date_from.is_none_or(|from| updated >= from)
                    && query.date_to.is_none_or(|to| updated < to)
            })
            .collect();

        // "X-Total-Count: The total number of objects available … (including the given query
        //  parameters, for example: date_to and date_from but excluding limit and offset)"
        let total = matching.len() as u64;
        let offset = query.offset_or_default();
        let limit = query.limit.map_or(self.max_page as u64, |l| l.min(self.max_page as u64)).max(1);

        let start = usize::try_from(offset).unwrap_or(usize::MAX).min(matching.len());
        let end = start.saturating_add(usize::try_from(limit).unwrap_or(usize::MAX)).min(matching.len());
        let items = matching[start..end].to_vec();

        let next = if (end as u64) < total {
            let next_query = query.clone().with_offset(end as u64).with_limit(limit);
            Some(next_query.apply_to(base))
        } else {
            None
        };

        Page { items, meta: PageMeta { next, total_count: Some(total), limit: Some(self.max_page as u64) } }
    }
}

macro_rules! stored_for {
    ($($ty:ty),* $(,)?) => {$(
        impl Stored for $ty {
            fn key(&self) -> String {
                self.id.as_str().to_owned()
            }
            fn last_updated(&self) -> DateTime {
                self.last_updated
            }
        }
    )*};
}

stored_for!(
    crate::v2_3_0::locations::Location,
    crate::v2_3_0::sessions::Session,
    crate::v2_3_0::cdrs::Cdr,
    crate::v2_3_0::tariffs::Tariff,
);

impl Stored for crate::v2_3_0::tokens::Token {
    fn key(&self) -> String {
        self.uid.as_str().to_owned()
    }
    fn last_updated(&self) -> DateTime {
        self.last_updated
    }
}

/// An in-memory store of Locations.
pub type InMemoryLocations = InMemoryStore<crate::v2_3_0::locations::Location>;
/// An in-memory store of Sessions.
pub type InMemorySessions = InMemoryStore<crate::v2_3_0::sessions::Session>;
/// An in-memory store of CDRs.
pub type InMemoryCdrs = InMemoryStore<crate::v2_3_0::cdrs::Cdr>;
/// An in-memory store of Tariffs.
pub type InMemoryTariffs = InMemoryStore<crate::v2_3_0::tariffs::Tariff>;
/// An in-memory store of Tokens, keyed by `uid`.
pub type InMemoryTokens = InMemoryStore<crate::v2_3_0::tokens::Token>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::sample;

    fn store_with(count: usize, page_size: usize) -> InMemoryLocations {
        let store = InMemoryLocations::with_page_size(page_size);
        for i in 0..count {
            let mut location = sample::location(&format!("LOC{i}")).unwrap();
            // Space the timestamps a minute apart so ordering is well defined.
            location.last_updated =
                DateTime::from_unix_timestamp(1_705_312_800 + i64::try_from(i).unwrap() * 60).unwrap();
            store.put(location);
        }
        store
    }

    fn base() -> Url {
        Url::new("https://cpo.example.com/ocpi/cpo/2.3.0/locations").unwrap()
    }

    #[test]
    fn put_reports_whether_the_object_was_created() {
        let store = InMemoryLocations::new();
        let location = sample::location("LOC1").unwrap();
        assert!(store.put(location.clone()), "the first PUT creates");
        assert!(!store.put(location), "the second replaces");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn ids_are_matched_case_insensitively_as_cistring_requires() {
        let store = InMemoryLocations::new();
        store.put(sample::location("LOC1").unwrap());
        assert!(store.get("loc1").is_some());
        assert!(store.remove("Loc1"));
        assert!(store.is_empty());
    }

    #[test]
    fn a_page_carries_a_next_link_until_the_last_one() {
        let store = store_with(25, 10);
        let first = store.page(&PageQuery::new(), &base());
        assert_eq!(first.items.len(), 10);
        assert_eq!(first.meta.total_count, Some(25));
        assert_eq!(first.meta.limit, Some(10), "X-Limit is the server maximum");
        let next = first.meta.next.expect("not the last page");
        assert!(next.as_str().contains("offset=10"), "{next}");

        let last = store.page(&PageQuery::new().with_offset(20), &base());
        assert_eq!(last.items.len(), 5);
        assert!(last.meta.next.is_none(), "the last page has no Link");
    }

    #[test]
    fn the_date_window_is_half_open() {
        let store = store_with(5, 100);
        let all = store.all();
        let second = all[1].last_updated;
        let fourth = all[3].last_updated;

        let page = store.page(&PageQuery::between(second, fourth), &base());
        // date_from inclusive, date_to exclusive: objects 1 and 2, not 3.
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].last_updated, second);
        assert_eq!(page.meta.total_count, Some(2), "the total reflects the filter");
    }

    #[test]
    fn sequential_intervals_do_not_overlap() {
        let store = store_with(6, 100);
        let all = store.all();
        let boundary = all[3].last_updated;
        let first = store.page(&PageQuery::between(all[0].last_updated, boundary), &base());
        let second = store.page(&PageQuery::since(boundary), &base());
        assert_eq!(first.items.len() + second.items.len(), 6, "every object appears exactly once");
    }

    #[test]
    fn the_total_count_excludes_limit_and_offset() {
        let store = store_with(25, 10);
        let page = store.page(&PageQuery::new().with_offset(10).with_limit(5), &base());
        assert_eq!(page.items.len(), 5);
        assert_eq!(page.meta.total_count, Some(25));
    }

    #[test]
    fn objects_come_back_oldest_first() {
        let store = store_with(3, 100);
        let page = store.page(&PageQuery::new(), &base());
        assert!(page.items[0].last_updated < page.items[1].last_updated);
        assert!(page.items[1].last_updated < page.items[2].last_updated);
    }
}
