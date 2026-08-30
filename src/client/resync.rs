//! Getting back in sync after a connection loss.
//!
//! > *OCPI messages SHOULD NOT be queued. When a client does a POST, PUT or PATCH request and that
//! > request fails or times out, the client should not queue the message and retry the same
//! > message again later. When the connection is re-established, it is up to the target-server of
//! > a connection to GET the current status from the source-server to get back to a synchronized
//! > state.*
//!
//! So the recovery from an outage is not a retry queue — it is a **pull**, and this module builds
//! the query for it.
//!
//! The other half of the advice is about not stampeding:
//!
//! > *It is therefore advised to clients pulling lists from a server to do this on a relative low
//! > polling interval: think in hours, not minutes, and to introduce some splay (randomize the
//! > length of the poll interface a bit).*
//!
//! Spec: 2.3.0 §transport_and_format_offline_behaviour, §transport_and_format_pull_and_push

use core::time::Duration;

use crate::transport::PageQuery;
use crate::types::DateTime;

/// How far back a resync reaches beyond the last successful pull.
///
/// A peer's clock and this one's are not identical, and an object can be written a moment before
/// its `last_updated` is read, so a resync that starts exactly where the last one ended can miss
/// an object. Fifteen minutes of overlap costs a handful of duplicate objects, which are
/// idempotent to apply, and closes the gap.
pub const DEFAULT_OVERLAP: Duration = Duration::from_mins(15);

/// The interval the specification recommends for routine polling.
///
/// > *think in hours, not minutes*
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_hours(4);

/// Builds the pull that brings a receiver back in sync.
///
/// ```
/// use ocpi_kit::client::Resync;
/// use ocpi_kit::types::DateTime;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let last_success: DateTime = "2024-03-01T10:00:00Z".parse()?;
/// let now: DateTime = "2024-03-01T14:00:00Z".parse()?;
///
/// let plan = Resync::new().plan(last_success, now);
/// // The window starts before the last success, so nothing written around the cut is missed.
/// assert!(plan.query.date_from.unwrap() < last_success);
/// assert_eq!(plan.query.date_to, Some(now));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Resync {
    overlap: Duration,
    poll_interval: Duration,
    splay_fraction: f32,
    page_limit: Option<u64>,
}

impl Default for Resync {
    fn default() -> Self {
        Self {
            overlap: DEFAULT_OVERLAP,
            poll_interval: DEFAULT_POLL_INTERVAL,
            splay_fraction: 0.2,
            page_limit: None,
        }
    }
}

impl Resync {
    /// A resync with the recommended defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How far back the window reaches beyond the last successful pull.
    #[must_use]
    pub const fn with_overlap(mut self, overlap: Duration) -> Self {
        self.overlap = overlap;
        self
    }

    /// The routine polling interval.
    #[must_use]
    pub const fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// How much of the interval the splay may add or remove, as a fraction. Clamped to `0.0..=1.0`.
    #[must_use]
    pub fn with_splay(mut self, fraction: f32) -> Self {
        self.splay_fraction = fraction.clamp(0.0, 1.0);
        self
    }

    /// Asks for a specific page size.
    #[must_use]
    pub const fn with_page_limit(mut self, limit: u64) -> Self {
        self.page_limit = Some(limit);
        self
    }

    /// The query that catches up everything changed since `last_success`, up to `now`.
    #[must_use]
    pub fn plan(&self, last_success: DateTime, now: DateTime) -> ResyncPlan {
        let overlap_seconds = i64::try_from(self.overlap.as_secs()).unwrap_or(i64::MAX);
        let from =
            DateTime::from_unix_timestamp(last_success.unix_timestamp().saturating_sub(overlap_seconds))
                .unwrap_or(last_success);
        let mut query = PageQuery::between(from, now);
        if let Some(limit) = self.page_limit {
            query = query.with_limit(limit);
        }
        ResyncPlan { query, next_poll_after: self.splayed_interval(now) }
    }

    /// The query for a routine incremental pull, with no end bound.
    ///
    /// An open-ended window keeps picking up objects written while the crawl runs, which is what
    /// a steady-state poll wants; use [`Resync::plan`] when a closed interval matters.
    #[must_use]
    pub fn incremental(&self, last_success: DateTime) -> PageQuery {
        let overlap_seconds = i64::try_from(self.overlap.as_secs()).unwrap_or(i64::MAX);
        let from =
            DateTime::from_unix_timestamp(last_success.unix_timestamp().saturating_sub(overlap_seconds))
                .unwrap_or(last_success);
        let mut query = PageQuery::since(from);
        if let Some(limit) = self.page_limit {
            query = query.with_limit(limit);
        }
        query
    }

    /// The polling interval with splay applied, so a fleet of clients does not synchronise.
    ///
    /// The splay is derived from `seed` rather than from a random number generator, so a given
    /// client polls at a stable, uncorrelated offset and the behaviour is reproducible in tests.
    #[must_use]
    pub fn splayed_interval(&self, seed: DateTime) -> Duration {
        if self.splay_fraction <= f32::EPSILON {
            return self.poll_interval;
        }
        let span = self.poll_interval.mul_f32(self.splay_fraction);
        // Map the seed into [0, 2*span) deterministically.
        let modulus = span.as_secs().saturating_mul(2).max(1);
        let offset = seed.unix_timestamp().unsigned_abs() % modulus;
        self.poll_interval.saturating_add(Duration::from_secs(offset)).saturating_sub(span)
    }
}

/// What to pull, and when to pull again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResyncPlan {
    /// The query for the catch-up pull.
    pub query: PageQuery,
    /// How long to wait before the next routine poll.
    pub next_poll_after: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> DateTime {
        s.parse().unwrap()
    }

    #[test]
    fn the_window_overlaps_the_last_success() {
        let plan = Resync::new().plan(dt("2024-03-01T10:00:00Z"), dt("2024-03-01T14:00:00Z"));
        assert_eq!(plan.query.date_from, Some(dt("2024-03-01T09:45:00Z")));
        assert_eq!(plan.query.date_to, Some(dt("2024-03-01T14:00:00Z")));
    }

    #[test]
    fn an_incremental_poll_has_no_end_bound() {
        let query =
            Resync::new().with_overlap(Duration::from_secs(60)).incremental(dt("2024-03-01T10:00:00Z"));
        assert_eq!(query.date_from, Some(dt("2024-03-01T09:59:00Z")));
        assert_eq!(query.date_to, None);
    }

    #[test]
    fn the_default_interval_is_measured_in_hours_not_minutes() {
        // "think in hours, not minutes"
        assert!(DEFAULT_POLL_INTERVAL >= Duration::from_hours(1));
    }

    #[test]
    fn splay_stays_within_the_configured_fraction() {
        let resync = Resync::new().with_splay(0.2);
        let span = DEFAULT_POLL_INTERVAL.mul_f32(0.2);
        for seed in
            ["2024-03-01T14:00:00Z", "2024-03-01T14:00:01Z", "2024-06-17T03:41:59Z", "1970-01-01T00:00:00Z"]
        {
            let interval = resync.splayed_interval(dt(seed));
            assert!(interval >= DEFAULT_POLL_INTERVAL.checked_sub(span).unwrap(), "{seed}: {interval:?}");
            assert!(interval <= DEFAULT_POLL_INTERVAL + span, "{seed}: {interval:?}");
        }
    }

    #[test]
    fn splay_is_deterministic_and_can_be_switched_off() {
        let resync = Resync::new();
        let seed = dt("2024-03-01T14:00:00Z");
        assert_eq!(resync.splayed_interval(seed), resync.splayed_interval(seed));
        assert_eq!(Resync::new().with_splay(0.0).splayed_interval(seed), DEFAULT_POLL_INTERVAL);
    }

    #[test]
    fn two_clients_starting_at_different_moments_do_not_align() {
        let resync = Resync::new();
        let a = resync.splayed_interval(dt("2024-03-01T14:00:00Z"));
        let b = resync.splayed_interval(dt("2024-03-01T14:07:13Z"));
        assert_ne!(a, b);
    }
}
