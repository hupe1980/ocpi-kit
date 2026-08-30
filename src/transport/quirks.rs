//! `Quirks` — per-peer interoperability flags, each with the sentence that makes it necessary.
//!
//! Every OCPI integration rediscovers that one partner does not Base64 its tokens, another puts a
//! trailing slash on every endpoint URL, and a third sends `"data": null` where the spec says a
//! list. This is that knowledge as a documented, testable per-peer profile.
//!
//! Every flag names the spec text that makes it defensible. A flag with no such basis is a bug in
//! the peer that should be reported, not accommodated silently.
//!
//! # Every flag here changes behaviour
//!
//! A configuration field that does nothing is worse than a missing feature: somebody sets it,
//! believes the problem is handled, and ships. So this struct holds only flags that are read
//! somewhere, and `cargo run -p xtask -- dead-config` fails the build if one stops being.
//!
//! Several accommodations this crate *does* make are deliberately **not** flags, because they are
//! unconditional and there is no coherent way to turn them off:
//!
//! * A trailing slash on a discovered URL is normalised by
//!   [`Url::join`](crate::types::Url::join) whatever anyone configures.
//! * An explicit `null` decodes to `None` because that is what `Option<T>` means in serde.
//! * An over-long identifier is always accepted and always reported — that is the crate's
//!   governing rule, not a per-peer setting; see [`types::validate`](crate::types::validate).
//! * `#NA` is recognised by [`CiString::is_not_available`](crate::types::CiString::is_not_available)
//!   wherever a caller asks. Whether a `#NA` in a given field is acceptable is a question about
//!   that field, not about the peer, so it belongs at the call site.

use crate::VersionNumber;

/// Per-peer interoperability settings.
///
/// Defaults are the strict, specification-conformant reading. [`Quirks::for_version`] relaxes the
/// ones that a given OCPI version genuinely requires — a 2.1.1 peer has no routing headers at
/// all, so sending them is wrong, not lenient.
///
/// ```
/// use ocpi_kit::VersionNumber;
/// use ocpi_kit::transport::Quirks;
///
/// let modern = Quirks::for_version(&VersionNumber::V2_3_0);
/// assert!(!modern.send_unencoded_token);
///
/// let legacy = Quirks::for_version(&VersionNumber::V2_1_1);
/// assert!(legacy.send_unencoded_token, "2.1.1 peers do not Base64 the token");
/// assert!(legacy.omit_routing_headers, "2.1.1 has no routing headers");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Quirks {
    /// Accept an `Authorization` header whose token is not Base64-encoded.
    ///
    /// > *NOTE: Many OCPI 2.1.1 and 2.2 implementations do not Base64 encode the credentials
    /// > token when including it in the 'Authorization' header. Since OCPI 2.2-d2 the OCPI
    /// > specification documents clearly require Base64 encoding.*
    ///
    /// Spec: 2.3.0 §transport_and_format_authorization_header
    pub accept_unencoded_token: bool,

    /// Send the `Authorization` token without Base64, for a peer that cannot decode it.
    ///
    /// Same spec note as [`Quirks::accept_unencoded_token`].
    pub send_unencoded_token: bool,

    /// Omit the four `OCPI-*` routing headers entirely.
    ///
    /// OCPI 2.1.1 and older have no message routing, so sending the headers to such a peer is
    /// not "extra safety", it is unspecified data.
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing (introduced in 2.2)
    pub omit_routing_headers: bool,

    /// Match module identifiers ignoring ASCII case when reading version details.
    ///
    /// Default **on**. The Bookings module identifier is `Booking` — singular and mixed case,
    /// unlike every other module — and implementations differ on it.
    pub case_insensitive_module_ids: bool,

    /// Clamp an outgoing `limit` to what **the peer** tolerates.
    ///
    /// Some peers answer a large `limit` with an error instead of the smaller page the spec
    /// prescribes. When the peer's `X-Limit` is known, prefer that.
    ///
    /// Named for the side it describes: `ServerConfig::max_page_limit` is the cap *this* process
    /// puts on the pages it serves, and the two are opposite ends of the same wire.
    pub peer_max_page_limit: Option<u64>,

    /// Accept a `Content-Type` other than `application/json` on a request with a body.
    ///
    /// > *The HTTP header: Content-Type SHALL be set to `application/json` for any request that
    /// > contains a message body.*
    ///
    /// Default **on** for the `application/json; charset=utf-8` form, which is harmless and
    /// common; this flag additionally accepts an absent or unrelated type.
    pub lenient_content_type: bool,
}

impl Default for Quirks {
    fn default() -> Self {
        Self {
            accept_unencoded_token: false,
            send_unencoded_token: false,
            omit_routing_headers: false,
            case_insensitive_module_ids: true,
            peer_max_page_limit: None,
            lenient_content_type: false,
        }
    }
}

impl Quirks {
    /// The strict, fully conformant profile.
    ///
    /// Useful when running a conformance check: anything a peer gets wrong shows up as an error
    /// rather than being quietly accommodated.
    #[must_use]
    pub fn strict() -> Self {
        Self { case_insensitive_module_ids: false, ..Self::default() }
    }

    /// Everything relaxed. For talking to a peer whose behaviour is not yet known.
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            accept_unencoded_token: true,
            case_insensitive_module_ids: true,
            lenient_content_type: true,
            // Not relaxed even here: sending an unencoded token, or dropping the routing
            // headers, is not leniency towards the peer — it is this side becoming the
            // non-conformant one. `for_version` turns those on where the version requires it.
            send_unencoded_token: false,
            omit_routing_headers: false,
            peer_max_page_limit: None,
        }
    }

    /// The profile a peer speaking `version` needs.
    ///
    /// The two relaxations for 2.1.1 and 2.2 are not guesses: those versions predate the
    /// Base64 requirement and the routing headers respectively.
    #[must_use]
    pub fn for_version(version: &VersionNumber) -> Self {
        let mut quirks = Self::default();
        // "Many OCPI 2.1.1 and 2.2 implementations do not Base64 encode the credentials token."
        if matches!(
            version,
            VersionNumber::V2_0 | VersionNumber::V2_1 | VersionNumber::V2_1_1 | VersionNumber::V2_2
        ) {
            quirks.accept_unencoded_token = true;
            quirks.send_unencoded_token = true;
        }
        // Routing headers arrived in 2.2.
        if !version.has_routing_headers() {
            quirks.omit_routing_headers = true;
        }
        quirks
    }

    /// This profile with the peer's advertised `X-Limit` applied as the page-size cap.
    #[must_use]
    pub fn with_peer_max_page_limit(mut self, limit: u64) -> Self {
        self.peer_max_page_limit = Some(limit);
        self
    }

    /// The page size to ask for, given what the caller wants.
    #[must_use]
    pub fn effective_limit(&self, requested: Option<u64>) -> Option<u64> {
        match (requested, self.peer_max_page_limit) {
            (Some(r), Some(max)) => Some(r.min(max)),
            (Some(r), None) => Some(r),
            (None, max) => max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conformant_not_permissive() {
        let q = Quirks::default();
        assert!(!q.accept_unencoded_token, "2.2-d2 and later require Base64");
        assert!(!q.send_unencoded_token);
        assert!(!q.omit_routing_headers);
        assert!(!q.lenient_content_type);
        // The Bookings module identifier really is spelled two ways in the wild.
        assert!(q.case_insensitive_module_ids);
    }

    #[test]
    fn legacy_versions_get_exactly_the_relaxations_they_need() {
        let v211 = Quirks::for_version(&VersionNumber::V2_1_1);
        assert!(v211.accept_unencoded_token && v211.send_unencoded_token);
        assert!(v211.omit_routing_headers);

        let v22 = Quirks::for_version(&VersionNumber::V2_2);
        assert!(v22.accept_unencoded_token, "2.2 predates the Base64 requirement");
        assert!(!v22.omit_routing_headers, "2.2 introduced routing headers");

        let v221 = Quirks::for_version(&VersionNumber::V2_2_1);
        assert!(!v221.accept_unencoded_token);
        assert!(!v221.omit_routing_headers);
    }

    #[test]
    fn page_limits_are_clamped_to_what_the_peer_tolerates() {
        let q = Quirks::default().with_peer_max_page_limit(100);
        assert_eq!(q.effective_limit(Some(2000)), Some(100));
        assert_eq!(q.effective_limit(Some(10)), Some(10));
        assert_eq!(q.effective_limit(None), Some(100));
        assert_eq!(Quirks::default().effective_limit(None), None);
    }

    #[test]
    fn the_strict_profile_accommodates_nothing() {
        // What a conformance run wants: anything the peer gets wrong shows up as an error rather
        // than being quietly absorbed.
        let q = Quirks::strict();
        assert!(!q.case_insensitive_module_ids);
        assert!(!q.accept_unencoded_token && !q.lenient_content_type);
    }
}
