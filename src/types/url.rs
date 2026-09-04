//! `Url` — the OCPI `URL` type, and the policy that keeps a hub from becoming an SSRF proxy.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::validate::{Validate, Validator, ViolationCode};

/// The maximum length the spec gives for a `URL`.
///
/// Spec: 2.3.0 §types_url_type — *"An URL a string(255) type"*
pub const URL_MAX_LEN: usize = 255;

/// An OCPI `URL`.
///
/// The text is stored **exactly as received**. A URL is an identifier as much as a location: a
/// peer that registered `https://example.com/ocpi/cpo/2.3.0` must see that string again, not the
/// `https://example.com/ocpi/cpo/2.3.0` that a normalising parser would hand back with a
/// re-encoded path or an added trailing slash. [`Url::parse`] gives the parsed form when it is
/// needed for making a request.
///
/// ```
/// use ocpi_kit::types::Url;
///
/// let url = Url::new("https://example.com/ocpi/cpo/2.3.0/locations").unwrap();
/// assert_eq!(url.as_str(), "https://example.com/ocpi/cpo/2.3.0/locations");
/// assert_eq!(url.parse().unwrap().host_str(), Some("example.com"));
/// ```
///
/// Spec: 2.3.0 §types_url_type
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Url(String);

impl Url {
    /// Creates a `Url`, checking that it parses as an absolute URL and fits `string(255)`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidUrl`] if the text is not an absolute URL or is longer than 255
    /// characters.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidUrl> {
        let value = value.into();
        let parsed = url::Url::parse(&value).map_err(|e| InvalidUrl(format!("{value:?}: {e}")))?;
        if parsed.cannot_be_a_base() {
            return Err(InvalidUrl(format!("{value:?}: not an absolute http(s) URL")));
        }
        let len = value.chars().count();
        if len > URL_MAX_LEN {
            return Err(InvalidUrl(format!("URL is {len} characters, the limit is {URL_MAX_LEN}")));
        }
        Ok(Self(value))
    }

    /// Creates a `Url` without checking anything. Used by `Deserialize`.
    pub fn new_lenient(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The URL exactly as received or constructed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes this value and yields the inner text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    /// Parses the URL.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidUrl`] if the stored text is not a URL. This can only happen for values
    /// that came off the wire, since [`Url::new`] checks up front.
    pub fn parse(&self) -> Result<url::Url, InvalidUrl> {
        url::Url::parse(&self.0).map_err(|e| InvalidUrl(format!("{:?}: {e}", self.0)))
    }

    /// This URL with an **already-encoded path** appended, keeping exactly one `/` between the
    /// two.
    ///
    /// The argument is written into the URL verbatim, so it may contain `/` and carry several
    /// segments. That is what a hub needs — it holds the path an incoming request arrived with,
    /// already encoded by the party that sent it — and it is the wrong tool for an object id.
    /// Use [`join_segment`](Self::join_segment) for anything that came out of a JSON document.
    ///
    /// Endpoint URLs discovered from a peer sometimes carry a trailing slash and sometimes do
    /// not; this joins correctly either way and never emits a double slash or a trailing one.
    ///
    /// ```
    /// use ocpi_kit::types::Url;
    /// let base = Url::new("https://example.com/ocpi/cpo/2.3.0/locations/").unwrap();
    /// assert_eq!(base.join("NL").join("TNM").as_str(),
    ///            "https://example.com/ocpi/cpo/2.3.0/locations/NL/TNM");
    /// ```
    #[must_use]
    pub fn join(&self, path: &str) -> Self {
        let base = self.0.trim_end_matches('/');
        let path = path.trim_start_matches('/').trim_end_matches('/');
        if path.is_empty() {
            return Self(base.to_owned());
        }
        Self(format!("{base}/{path}"))
    }

    /// This URL with **one path segment** appended, percent-encoded.
    ///
    /// An OCPI identifier is a `CiString(36)`, and the specification puts no restriction on which
    /// characters it may contain. Interpolating one into a URL verbatim therefore lets the *value*
    /// change the URL's structure: a `token_uid` of `../credentials` addresses a different
    /// endpoint, and one containing `?` starts a query string. Both are reachable from data a peer
    /// sent, so every id this crate puts in a path goes through here.
    ///
    /// The encoding is RFC 3986 `pchar`: unreserved characters, sub-delims and `:`/`@` are left
    /// alone — which keeps the `*` of `BE*BEC*E041503001` readable — and everything else,
    /// including `/`, `?`, `#`, `%`, space and every non-ASCII byte, is percent-encoded.
    ///
    /// ```
    /// use ocpi_kit::types::Url;
    /// let base = Url::new("https://example.com/ocpi/cpo/2.3.0/tokens").unwrap();
    /// assert_eq!(base.join_segment("BE*BEC*E041503001").as_str(),
    ///            "https://example.com/ocpi/cpo/2.3.0/tokens/BE*BEC*E041503001");
    /// assert_eq!(base.join_segment("../credentials").as_str(),
    ///            "https://example.com/ocpi/cpo/2.3.0/tokens/..%2Fcredentials");
    /// ```
    #[must_use]
    pub fn join_segment(&self, segment: &str) -> Self {
        self.join(&encode_path_segment(segment))
    }

    /// This URL with a query string appended verbatim, using `?` or `&` as appropriate.
    ///
    /// The argument is not encoded; build it with [`with_param`](Self::with_param) or
    /// [`encode_query_component`] unless it is already a query string.
    #[must_use]
    pub fn with_query(&self, query: &str) -> Self {
        if query.is_empty() {
            return self.clone();
        }
        let sep = if self.0.contains('?') { '&' } else { '?' };
        Self(format!("{}{sep}{query}", self.0))
    }

    /// This URL with one `name=value` query parameter appended, with the value percent-encoded.
    ///
    /// ```
    /// use ocpi_kit::types::Url;
    /// let url = Url::new("https://example.com/ocpi/cpo/2.3.0/tokens/012").unwrap();
    /// assert_eq!(url.with_param("type", "APP_USER").as_str(),
    ///            "https://example.com/ocpi/cpo/2.3.0/tokens/012?type=APP_USER");
    /// ```
    #[must_use]
    pub fn with_param(&self, name: &str, value: &str) -> Self {
        self.with_query(&format!("{name}={}", encode_query_component(value)))
    }

    /// Whether this URL is acceptable under `policy`.
    ///
    /// # Errors
    ///
    /// Returns the reason the URL was refused.
    pub fn check(&self, policy: &UrlPolicy) -> Result<(), UrlRefused> {
        policy.check(self)
    }
}

/// Percent-encodes one URL **path segment**.
///
/// Everything outside RFC 3986 `pchar` — unreserved, sub-delims, `:` and `@` — is encoded, so the
/// value cannot introduce a segment boundary, a query, a fragment or a stray `%`.
#[must_use]
pub fn encode_path_segment(value: &str) -> String {
    encode(value, |byte| {
        matches!(byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'.' | b'_' | b'~'
            | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
            | b':' | b'@')
    })
}

/// Percent-encodes one query-parameter name or value.
///
/// Stricter than [`encode_path_segment`]: only unreserved characters survive, so a value carrying
/// `&`, `=` or `+` cannot become a second parameter or a space.
#[must_use]
pub fn encode_query_component(value: &str) -> String {
    encode(value, |byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'))
}

fn encode(value: &str, keep: impl Fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if keep(byte) {
            out.push(byte as char);
        } else {
            use core::fmt::Write as _;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

impl Validate for Url {
    fn validate_in(&self, v: &mut Validator) {
        match url::Url::parse(&self.0) {
            Ok(u) if u.cannot_be_a_base() => {
                v.report(ViolationCode::IllegalCharacter, format!("{:?} is not an absolute URL", self.0));
            }
            Ok(_) => {}
            Err(e) => v.report(ViolationCode::IllegalCharacter, format!("{:?} is not a URL: {e}", self.0)),
        }
        let len = self.0.chars().count();
        if len > URL_MAX_LEN {
            v.report(ViolationCode::TooLong, format!("URL({URL_MAX_LEN}) holds {len} characters"));
        }
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl fmt::Debug for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}
impl AsRef<str> for Url {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl FromStr for Url {
    type Err = InvalidUrl;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}
// The infallible conversions are **lenient**, matching `Deserialize`; use `Url::new` or
// `str::parse` for the checked path.
impl From<&str> for Url {
    fn from(s: &str) -> Self {
        Self::new_lenient(s)
    }
}

impl From<String> for Url {
    fn from(s: String) -> Self {
        Self::new_lenient(s)
    }
}
impl From<url::Url> for Url {
    fn from(value: url::Url) -> Self {
        Self(value.to_string())
    }
}

impl Serialize for Url {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for Url {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Url {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "URL".into()
    }
    fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "format": "uri", "maxLength": URL_MAX_LEN })
    }
}

/// Why a URL was refused by a [`UrlPolicy`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlRefused(String);

impl fmt::Display for UrlRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "URL refused: {}", self.0)
    }
}
impl std::error::Error for UrlRefused {}

/// Why a string is not a usable OCPI `URL`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidUrl(String);

impl fmt::Display for InvalidUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid URL: {}", self.0)
    }
}
impl std::error::Error for InvalidUrl {}

/// What a party is willing to send a request to.
///
/// OCPI hands a server URLs that it is then expected to call: `Credentials.url`,
/// `Endpoint.url`, and every `response_url` in the Commands and Charging Profiles modules. A
/// party that fetches those without checking is a server-side request forgery proxy for anyone
/// it has registered with. The specification says nothing about this, so this crate ships a
/// default that says no to the things a CPO never legitimately needs to call.
///
/// # What this does not do
///
/// A `UrlPolicy` inspects the URL, and only the URL. It cannot see where a **host name**
/// resolves, so `https://ptp.example.com/cb` passes even when that name has an `A` record for
/// `169.254.169.254`, and a name that resolves differently between the check and the connection
/// defeats it outright (a DNS rebind). Closing that needs a resolver in the connection path,
/// which belongs to whatever HTTP client is doing the fetching rather than to a URL type.
///
/// So treat this as the first of two layers, not as the whole defence. In production, pair it
/// with [`with_allowed_hosts`](Self::with_allowed_hosts) — an explicit list per peer is not
/// subject to either problem — and with an egress policy on the network that refuses the link-
/// local and private ranges outright. The literal-IP rules below are what stops the careless
/// cases; the allow-list is what stops the deliberate ones.
///
/// ```
/// use ocpi_kit::types::{Url, UrlPolicy};
///
/// let policy = UrlPolicy::default();
/// assert!(policy.check(&Url::new("https://msp.example.com/cb/1").unwrap()).is_ok());
/// assert!(policy.check(&Url::new("http://msp.example.com/cb/1").unwrap()).is_err()); // not TLS
/// assert!(policy.check(&Url::new("https://127.0.0.1/cb").unwrap()).is_err());        // loopback
/// assert!(policy.check(&Url::new("file:///etc/passwd").unwrap()).is_err());          // scheme
/// ```
#[derive(Clone, Debug)]
pub struct UrlPolicy {
    /// Schemes that may be used. Defaults to `https` only.
    pub allowed_schemes: Vec<String>,
    /// Whether loopback, link-local, private and unspecified addresses may be targeted.
    ///
    /// Defaults to `false`. Set to `true` for local development and integration tests.
    pub allow_private_networks: bool,
    /// When non-empty, only these hosts (matched case-insensitively, plus their subdomains) may
    /// be targeted.
    pub allowed_hosts: Vec<String>,
}

impl Default for UrlPolicy {
    fn default() -> Self {
        Self {
            allowed_schemes: vec!["https".to_owned()],
            allow_private_networks: false,
            allowed_hosts: Vec::new(),
        }
    }
}

impl UrlPolicy {
    /// A policy that permits anything, for tests and for talking to a peer over plain HTTP on a
    /// trusted network.
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            allowed_schemes: vec!["https".to_owned(), "http".to_owned()],
            allow_private_networks: true,
            allowed_hosts: Vec::new(),
        }
    }

    /// Restricts this policy to `hosts` and their subdomains.
    #[must_use]
    pub fn with_allowed_hosts<I, S>(mut self, hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_hosts = hosts.into_iter().map(Into::into).collect();
        self
    }

    /// Allows plain `http` in addition to whatever is already allowed.
    #[must_use]
    pub fn allowing_http(mut self) -> Self {
        if !self.allowed_schemes.iter().any(|s| s == "http") {
            self.allowed_schemes.push("http".to_owned());
        }
        self
    }

    /// Allows targets on private and loopback networks.
    #[must_use]
    pub fn allowing_private_networks(mut self) -> Self {
        self.allow_private_networks = true;
        self
    }

    /// Checks `url` against this policy.
    ///
    /// # Errors
    ///
    /// Returns [`UrlRefused`] naming the rule that rejected the URL.
    pub fn check(&self, url: &Url) -> Result<(), UrlRefused> {
        let parsed = url.parse().map_err(|e| UrlRefused(e.to_string()))?;
        let scheme = parsed.scheme();
        if !self.allowed_schemes.iter().any(|s| s == scheme) {
            return Err(UrlRefused(format!(
                "scheme {scheme:?} is not allowed (allowed: {})",
                self.allowed_schemes.join(", ")
            )));
        }
        let Some(host) = parsed.host() else {
            return Err(UrlRefused("URL has no host".to_owned()));
        };
        if !self.allow_private_networks && is_private_host(&host) {
            return Err(UrlRefused(format!("{host} is on a private or loopback network")));
        }
        if !self.allowed_hosts.is_empty() {
            let host_text = host.to_string();
            let ok = self.allowed_hosts.iter().any(|allowed| {
                host_text.eq_ignore_ascii_case(allowed)
                    || host_text.len() > allowed.len()
                        && host_text.as_bytes()[host_text.len() - allowed.len() - 1] == b'.'
                        && host_text[host_text.len() - allowed.len()..].eq_ignore_ascii_case(allowed)
            });
            if !ok {
                return Err(UrlRefused(format!("host {host_text:?} is not in the allow-list")));
            }
        }
        Ok(())
    }
}

fn is_private_host(host: &url::Host<&str>) -> bool {
    use std::net::IpAddr;
    match host {
        url::Host::Ipv4(ip) => is_private_ip(&IpAddr::V4(*ip)),
        url::Host::Ipv6(ip) => is_private_ip(&IpAddr::V6(*ip)),
        url::Host::Domain(name) => {
            // Already lower-cased, so these are case-insensitive comparisons, not extension
            // checks; `.local` here is an mDNS suffix rather than a file extension.
            let lower = name.to_ascii_lowercase();
            lower == "localhost" || lower.ends_with(".localhost") || lower.strip_suffix(".local").is_some()
        }
    }
}

fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                // 100.64.0.0/10, carrier-grade NAT.
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // Unique local addresses fc00::/7 and link-local fe80::/10.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some_and(|v4| is_private_ip(&IpAddr::V4(v4)))
                // The deprecated IPv4-compatible form `::a.b.c.d`, which `to_ipv4_mapped` does
                // not cover, and the NAT64 well-known prefix 64:ff9b::/96 — both are ways of
                // spelling an IPv4 address that a naive check would wave through.
                || v6.segments()[..6] == [0, 0, 0, 0, 0, 0]
                    && v6.segments()[6] != 0
                    && is_private_ip(&IpAddr::V4(embedded_v4(v6)))
                || v6.segments()[..4] == [0x0064, 0xff9b, 0, 0]
                    && is_private_ip(&IpAddr::V4(embedded_v4(v6)))
        }
    }
}

/// The IPv4 address carried in the low 32 bits of an IPv6 address.
fn embedded_v4(v6: &std::net::Ipv6Addr) -> std::net::Ipv4Addr {
    let o = v6.octets();
    std::net::Ipv4Addr::new(o[12], o[13], o[14], o[15])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identifier_cannot_change_the_shape_of_a_url() {
        let base = Url::new("https://e.com/ocpi/cpo/2.3.0/tokens").unwrap();
        // OCPI ids are CiString(36) with no character restriction, and a Token uid arrives from
        // a peer. Interpolated verbatim, these would address a different endpoint.
        assert_eq!(
            base.join_segment("../credentials").as_str(),
            "https://e.com/ocpi/cpo/2.3.0/tokens/..%2Fcredentials"
        );
        assert_eq!(
            base.join_segment("012?limit=1").as_str(),
            "https://e.com/ocpi/cpo/2.3.0/tokens/012%3Flimit=1"
        );
        assert_eq!(base.join_segment("a#b").as_str(), "https://e.com/ocpi/cpo/2.3.0/tokens/a%23b");
        assert_eq!(base.join_segment("100%").as_str(), "https://e.com/ocpi/cpo/2.3.0/tokens/100%25");
        // And the shapes real identifiers actually have survive unencoded.
        for id in ["BE*BEC*E041503001", "3256", "LOC1", "NL-TNM-C12345678-X", "a.b~c_d"] {
            assert_eq!(base.join_segment(id).as_str(), format!("https://e.com/ocpi/cpo/2.3.0/tokens/{id}"));
        }
    }

    #[test]
    fn a_query_value_cannot_add_a_parameter() {
        let url = Url::new("https://e.com/ocpi/cpo/2.3.0/tokens/012").unwrap();
        assert_eq!(
            url.with_param("type", "RFID&offset=9").as_str(),
            "https://e.com/ocpi/cpo/2.3.0/tokens/012?type=RFID%26offset%3D9"
        );
    }

    #[test]
    fn text_is_preserved_exactly() {
        // `url::Url` would normalise this to `https://example.com/`.
        let u = Url::new("https://example.com").unwrap();
        assert_eq!(u.as_str(), "https://example.com");
        assert_eq!(serde_json::to_string(&u).unwrap(), "\"https://example.com\"");
    }

    #[test]
    fn join_handles_trailing_slashes_either_way() {
        for base in ["https://e.com/l", "https://e.com/l/"] {
            let u = Url::new(base).unwrap();
            assert_eq!(u.join("NL").join("TNM").join("14").as_str(), "https://e.com/l/NL/TNM/14");
        }
    }

    #[test]
    fn with_query_picks_the_right_separator() {
        let u = Url::new("https://e.com/cdrs").unwrap();
        assert_eq!(u.with_query("limit=10").as_str(), "https://e.com/cdrs?limit=10");
        assert_eq!(
            u.with_query("limit=10").with_query("offset=5").as_str(),
            "https://e.com/cdrs?limit=10&offset=5"
        );
    }

    #[test]
    fn default_policy_blocks_the_ssrf_shapes() {
        let p = UrlPolicy::default();
        assert!(p.check(&Url::new("https://msp.example.com/cb").unwrap()).is_ok());
        for bad in [
            "http://msp.example.com/cb",
            "https://127.0.0.1/cb",
            "https://localhost/cb",
            "https://10.0.0.5/cb",
            "https://192.168.1.1/cb",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/cb",
            "https://[fd00::1]/cb",
            "https://[fe80::1]/cb",
            // The same metadata endpoint spelled three other ways.
            "https://[::ffff:169.254.169.254]/latest/meta-data",
            "https://[::169.254.169.254]/latest/meta-data",
            "https://[64:ff9b::169.254.169.254]/latest/meta-data",
        ] {
            assert!(p.check(&Url::new(bad).unwrap()).is_err(), "{bad} should be refused");
        }
        // A public address in any of those forms is still reachable.
        assert!(p.check(&Url::new("https://[64:ff9b::93.184.216.34]/cb").unwrap()).is_ok());
    }

    #[test]
    fn a_host_name_is_not_resolved_so_the_allow_list_is_the_real_defence() {
        // Documented limitation, asserted so it cannot regress into a false sense of safety:
        // the policy sees the URL, not where the name points.
        let p = UrlPolicy::default();
        assert!(p.check(&Url::new("https://metadata.example.com/latest").unwrap()).is_ok());
        let strict = p.with_allowed_hosts(["ptp.example.com"]);
        assert!(strict.check(&Url::new("https://metadata.example.com/latest").unwrap()).is_err());
    }

    #[test]
    fn host_allow_list_matches_subdomains_only_at_a_dot_boundary() {
        let p = UrlPolicy::default().with_allowed_hosts(["example.com"]);
        assert!(p.check(&Url::new("https://example.com/a").unwrap()).is_ok());
        assert!(p.check(&Url::new("https://ocpi.example.com/a").unwrap()).is_ok());
        assert!(p.check(&Url::new("https://notexample.com/a").unwrap()).is_err());
        assert!(p.check(&Url::new("https://example.com.evil.net/a").unwrap()).is_err());
    }

    #[test]
    fn over_long_urls_are_reported_not_dropped() {
        let long = format!("https://e.com/{}", "x".repeat(300));
        assert!(Url::new(&long).is_err());
        let lenient = Url::new_lenient(&long);
        assert_eq!(lenient.validate().unwrap_err().as_slice()[0].code, ViolationCode::TooLong);
    }
}
