//! `Quirks` — per-peer interoperability flags, each with the sentence that makes it necessary.
//!
//! Nobody publishes this knowledge. Every team that ships an OCPI integration rediscovers that
//! one partner does not Base64 its tokens, another puts a trailing slash on every endpoint URL,
//! and a third sends `"data": null` where the spec says a list. Encoding that as a documented,
//! testable per-peer profile is the difference between a library and a support ticket.
//!
//! Every flag names the spec text that makes it defensible. A flag with no such basis is a bug
//! in the peer that should be reported, not accommodated silently.

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

    /// Tolerate a trailing slash on discovered endpoint URLs and `Link` headers.
    ///
    /// The spec's own pagination examples show `…/cdrs/?offset=150`, while its endpoint examples
    /// show `…/cdrs`. Both occur in the wild; joining is done by
    /// [`Url::join`](crate::types::Url::join), which normalises either way.
    pub trailing_slash: bool,

    /// Treat an explicit `null` as an absent field.
    ///
    /// Default **on**: the spec itself advises accepting `data` *"being absent"* or *"present
    /// with any possible value"*, and peers extend that habit to object fields.
    ///
    /// Spec: 2.3.0 §transport_and_format_response_format
    pub null_means_absent: bool,

    /// Recognise the `#NA` sentinel in string fields.
    ///
    /// > *There are rare situation … where a certain field, that is required, cannot be filled.
    /// > In such cases, and only in such cases, it is allowed to set a string field to the value
    /// > `#NA`.*
    ///
    /// Default **on**: this is specified behaviour, not a deviation. The flag exists so a party
    /// that wants to reject `#NA` outright can.
    ///
    /// Spec: 2.3.0 §transport_and_format_not_available
    pub na_sentinel: bool,

    /// Match module identifiers ignoring ASCII case when reading version details.
    ///
    /// Default **on**. The Bookings module identifier is `Booking` — singular and mixed case,
    /// unlike every other module — and implementations differ on it.
    pub case_insensitive_module_ids: bool,

    /// Clamp an outgoing `limit` to this value.
    ///
    /// Some peers answer a large `limit` with an error instead of the smaller page the spec
    /// prescribes. When the peer's `X-Limit` is known, prefer that.
    pub max_page_limit: Option<u64>,

    /// Accept an over-long identifier without complaint.
    ///
    /// 2.1.1 gave `CDR.id` 36 characters; some vendor code emits 39 there because that is the
    /// 2.2.1 length. The value is preserved either way — this only silences the validator.
    pub lenient_id_length: bool,

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
            trailing_slash: true,
            null_means_absent: true,
            na_sentinel: true,
            case_insensitive_module_ids: true,
            max_page_limit: None,
            lenient_id_length: false,
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
        Self {
            trailing_slash: false,
            null_means_absent: false,
            na_sentinel: false,
            case_insensitive_module_ids: false,
            ..Self::default()
        }
    }

    /// Everything relaxed. For talking to a peer whose behaviour is not yet known.
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            accept_unencoded_token: true,
            send_unencoded_token: false,
            omit_routing_headers: false,
            trailing_slash: true,
            null_means_absent: true,
            na_sentinel: true,
            case_insensitive_module_ids: true,
            max_page_limit: None,
            lenient_id_length: true,
            lenient_content_type: true,
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
    pub fn with_max_page_limit(mut self, limit: u64) -> Self {
        self.max_page_limit = Some(limit);
        self
    }

    /// The page size to ask for, given what the caller wants.
    #[must_use]
    pub fn effective_limit(&self, requested: Option<u64>) -> Option<u64> {
        match (requested, self.max_page_limit) {
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
        assert!(!q.lenient_id_length);
        // These two are specified behaviour, so they are on by default.
        assert!(q.null_means_absent);
        assert!(q.na_sentinel);
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
        let q = Quirks::default().with_max_page_limit(100);
        assert_eq!(q.effective_limit(Some(2000)), Some(100));
        assert_eq!(q.effective_limit(Some(10)), Some(10));
        assert_eq!(q.effective_limit(None), Some(100));
        assert_eq!(Quirks::default().effective_limit(None), None);
    }

    #[test]
    fn the_strict_profile_turns_off_even_the_specified_leniencies() {
        let q = Quirks::strict();
        assert!(!q.null_means_absent && !q.na_sentinel && !q.trailing_slash);
    }
}
