//! A read-only conformance runner: drive a live peer and report what it does wrong.
//!
//! Every party in a roaming network has the same problem — the partner's implementation is not
//! quite the specification, and finding out *where* means reading their JSON by hand. This module
//! does it mechanically, and produces a report either side can act on.
//!
//! # It never changes anything
//!
//! The runner issues `GET` requests and exactly one deliberately-unauthenticated `GET`. It does
//! **not** register, does not POST credentials, does not write an object, and does not delete
//! one. A conformance check that mutates the peer it is checking is not a conformance check, and
//! running this against a production partner is safe.
//!
//! # What it checks
//!
//! Discovery and the transport rules first, because a peer that gets those wrong will fail
//! everything else for reasons that are hard to read:
//!
//! * `/versions` answers with a `1000` envelope carrying a plausible timestamp
//! * the versions offered are unique, and at least one is one this build speaks
//! * every advertised URL is absolute and passes the [`UrlPolicy`](crate::types::UrlPolicy)
//! * the version details list `credentials`, list no module that does not exist in that version,
//!   and list no `(module, role)` pair twice
//! * `X-Request-ID` and `X-Correlation-ID` come back on the response
//! * a request with no token, and one with a wrong token, are refused with `401`
//!
//! Then, for each Sender interface it can reach, that a page decodes, that its pagination headers
//! agree with each other, that a `limit` is never exceeded, and that the objects conform.
//!
//! ```no_run
//! use ocpi_kit::client::{Conformance, OcpiClient};
//! use ocpi_kit::transport::CredentialsToken;
//! use ocpi_kit::types::Url;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = OcpiClient::new()?;
//! let report = Conformance::new(
//!         Url::new("https://cpo.example.com/ocpi/versions")?,
//!         CredentialsToken::new("our-token-c")?,
//!     )
//!     .run(client.transport())
//!     .await;
//!
//! println!("{report}");
//! if report.has_failures() {
//!     // …open a ticket with the partner, quoting `report`
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Spec: 2.3.0 §version_information_endpoint, §transport_and_format_transport_and_format,
//! §status_codes_status_codes

use core::fmt;

use crate::transport::{
    CredentialsToken, OcpiError, OcpiRequest, Page, PageQuery, Quirks, RequestIds, StatusCode,
};
use crate::types::{DateTime, Url, Validate};
use crate::v2_3_0::versions::{Version, VersionDetails};
use crate::{InterfaceRole, ModuleId, VersionNumber};

use super::http::Transport;

/// How one check came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Outcome {
    /// The peer did what the specification says.
    Pass,
    /// The peer did something the specification does not require but that will cause trouble.
    Warn,
    /// The peer contradicts the specification.
    Fail,
    /// The check could not run — usually because the peer does not implement that module.
    Skipped,
}

impl Outcome {
    /// A single character for a compact report line.
    #[must_use]
    pub const fn glyph(self) -> char {
        match self {
            Self::Pass => '+',
            Self::Warn => '!',
            Self::Fail => 'x',
            Self::Skipped => '-',
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
            Self::Skipped => "SKIP",
        })
    }
}

/// One thing that was checked, and what the peer did.
#[derive(Clone, Debug)]
pub struct Check {
    /// Stable identifier, so a report can be diffed between runs.
    pub id: &'static str,
    /// What was checked, in one line.
    pub title: String,
    /// How it came out.
    pub outcome: Outcome,
    /// What the peer actually did.
    pub detail: String,
    /// The specification anchor this check comes from.
    pub spec: &'static str,
}

impl Check {
    fn new(
        id: &'static str,
        title: impl Into<String>,
        outcome: Outcome,
        detail: impl Into<String>,
        spec: &'static str,
    ) -> Self {
        Self { id, title: title.into(), outcome, detail: detail.into(), spec }
    }
}

/// Everything the runner found.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// The checks, in the order they ran.
    pub checks: Vec<Check>,
    /// The version that was selected, once discovery got that far.
    pub version: Option<VersionNumber>,
}

impl Report {
    /// How many checks came out a given way.
    #[must_use]
    pub fn count(&self, outcome: Outcome) -> usize {
        self.checks.iter().filter(|c| c.outcome == outcome).count()
    }

    /// Whether anything contradicted the specification.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.count(Outcome::Fail) > 0
    }

    /// The failing checks, for a caller that wants to act on them rather than print them.
    pub fn failures(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.outcome == Outcome::Fail)
    }

    fn push(&mut self, check: Check) {
        self.checks.push(check);
    }

    fn pass(&mut self, id: &'static str, title: &str, detail: impl Into<String>, spec: &'static str) {
        self.push(Check::new(id, title, Outcome::Pass, detail, spec));
    }

    fn fail(&mut self, id: &'static str, title: &str, detail: impl Into<String>, spec: &'static str) {
        self.push(Check::new(id, title, Outcome::Fail, detail, spec));
    }

    fn warn(&mut self, id: &'static str, title: &str, detail: impl Into<String>, spec: &'static str) {
        self.push(Check::new(id, title, Outcome::Warn, detail, spec));
    }

    fn skip(&mut self, id: &'static str, title: &str, detail: impl Into<String>, spec: &'static str) {
        self.push(Check::new(id, title, Outcome::Skipped, detail, spec));
    }

    /// Records `Pass` or `Fail` from a boolean, with the reason for each.
    fn assert(
        &mut self,
        id: &'static str,
        title: &str,
        ok: bool,
        detail: impl Into<String>,
        spec: &'static str,
    ) {
        let outcome = if ok { Outcome::Pass } else { Outcome::Fail };
        self.push(Check::new(id, title, outcome, detail, spec));
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for check in &self.checks {
            writeln!(f, "[{}] {:<10} {}", check.outcome.glyph(), check.id, check.title)?;
            if !check.detail.is_empty() {
                writeln!(f, "               {}", check.detail)?;
            }
            if check.outcome == Outcome::Fail || check.outcome == Outcome::Warn {
                writeln!(f, "               spec: {}", check.spec)?;
            }
        }
        writeln!(
            f,
            "\n{} passed, {} failed, {} warnings, {} skipped",
            self.count(Outcome::Pass),
            self.count(Outcome::Fail),
            self.count(Outcome::Warn),
            self.count(Outcome::Skipped),
        )
    }
}

/// The modules whose Sender interface the runner will pull one page from.
///
/// All of them are list endpoints whose objects this crate models, so a page can be decoded and
/// validated. Nothing here writes.
const PULLABLE: &[ModuleId] =
    &[ModuleId::Locations, ModuleId::Sessions, ModuleId::Cdrs, ModuleId::Tariffs, ModuleId::Tokens];

/// Drives a peer through the checks.
#[derive(Clone, Debug)]
pub struct Conformance {
    versions_url: Url,
    token: CredentialsToken,
    quirks: Quirks,
    page_limit: u64,
    max_clock_skew: core::time::Duration,
    check_auth: bool,
}

impl Conformance {
    /// A run against `versions_url`, authenticating with `token`.
    ///
    /// `token` should be the `CREDENTIALS_TOKEN_C` of an existing registration — the runner reads
    /// the peer's data, which requires being registered with it.
    #[must_use]
    pub fn new(versions_url: Url, token: CredentialsToken) -> Self {
        Self {
            versions_url,
            token,
            quirks: Quirks::default(),
            page_limit: 10,
            max_clock_skew: core::time::Duration::from_secs(300),
            check_auth: true,
        }
    }

    /// Applies the peer's known quirks, so the run reports real problems rather than known ones.
    #[must_use]
    pub fn with_quirks(mut self, quirks: Quirks) -> Self {
        self.quirks = quirks;
        self
    }

    /// How many objects to ask for per page. Small by default: this is a check, not a crawl.
    #[must_use]
    pub const fn with_page_limit(mut self, limit: u64) -> Self {
        self.page_limit = limit;
        self
    }

    /// How far the peer's clock may be from ours before the timestamp check complains.
    #[must_use]
    pub const fn with_max_clock_skew(mut self, skew: core::time::Duration) -> Self {
        self.max_clock_skew = skew;
        self
    }

    /// Whether to send the two deliberately-rejected requests that check authentication.
    ///
    /// On by default. Turn it off against a peer whose intrusion detection would rather not see
    /// a failed authentication from a partner.
    #[must_use]
    pub const fn with_auth_checks(mut self, check: bool) -> Self {
        self.check_auth = check;
        self
    }

    /// Runs every check and returns the report.
    ///
    /// Never returns an error: a peer that cannot be reached at all is itself a finding, and it
    /// is recorded as one.
    pub async fn run(&self, transport: &Transport) -> Report {
        let mut report = Report::default();

        let Some(versions) = self.check_versions(transport, &mut report).await else {
            return report;
        };
        let Some((version, details)) = self.check_details(transport, &mut report, &versions).await else {
            return report;
        };
        report.version = Some(version.clone());

        Self::check_endpoints(transport, &mut report, &version, &details);
        if self.check_auth {
            self.check_authentication(transport, &mut report, &details).await;
        }
        self.check_modules(transport, &mut report, &details).await;

        report
    }

    /// `GET /versions` and everything that can be said about the answer.
    async fn check_versions(&self, transport: &Transport, report: &mut Report) -> Option<Vec<Version>> {
        const SPEC: &str = "2.3.0 §version_information_endpoint";
        let request = OcpiRequest::new(http::Method::GET, self.versions_url.clone(), ModuleId::Versions)
            .with_ids(RequestIds::generate());

        let (envelope, headers) =
            match transport.send_with_headers::<Vec<Version>>(&request, &self.token, &self.quirks).await {
                Ok(pair) => pair,
                Err(e) => {
                    report.fail("versions.get", "GET /versions answers", e.to_string(), SPEC);
                    return None;
                }
            };

        report.assert(
            "versions.status",
            "GET /versions returns status_code 1000",
            envelope.status_code == StatusCode::SUCCESS,
            format!("got {}", envelope.status_code),
            "2.3.0 §status_codes_1xxx_success",
        );

        Self::check_echoed_ids(report, &request.ids, &headers);
        self.check_timestamp(report, envelope.timestamp);

        let Some(versions) = envelope.data else {
            report.fail(
                "versions.data",
                "GET /versions carries a data field",
                "the envelope has no `data`, so no version could be read",
                SPEC,
            );
            return None;
        };

        report.assert(
            "versions.nonempty",
            "at least one version is offered",
            !versions.is_empty(),
            format!("{} offered", versions.len()),
            SPEC,
        );

        let mut numbers: Vec<String> = versions.iter().map(|v| v.version.to_string()).collect();
        numbers.sort();
        let unique = {
            let mut d = numbers.clone();
            d.dedup();
            d.len() == numbers.len()
        };
        report.assert("versions.unique", "each version is listed once", unique, numbers.join(", "), SPEC);

        for version in &versions {
            if let Err(e) = transport.url_policy().check(&version.url) {
                report.fail(
                    "versions.url",
                    "every advertised version URL is usable",
                    format!("{}: {e}", version.version),
                    "2.3.0 §types_url_type",
                );
            }
        }

        let common: Vec<&Version> = versions.iter().filter(|v| v.version.is_supported()).collect();
        if common.is_empty() {
            report.fail(
                "versions.common",
                "the peer offers a version this build speaks",
                format!(
                    "peer offers {}; this build speaks {}",
                    numbers.join(", "),
                    VersionNumber::supported().iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
                ),
                SPEC,
            );
            return None;
        }
        report.pass(
            "versions.common",
            "the peer offers a version this build speaks",
            common.iter().map(|v| v.version.to_string()).collect::<Vec<_>>().join(", "),
            SPEC,
        );

        Some(versions)
    }

    /// `GET` the version-details URL of the newest common version.
    async fn check_details(
        &self,
        transport: &Transport,
        report: &mut Report,
        versions: &[Version],
    ) -> Option<(VersionNumber, VersionDetails)> {
        const SPEC: &str = "2.3.0 §version_information_endpoint_version_details";

        let best = versions
            .iter()
            .filter(|v| v.version.is_supported())
            .max_by(|a, b| a.version.cmp_by_release(&b.version))?;

        let request = OcpiRequest::new(http::Method::GET, best.url.clone(), ModuleId::Versions)
            .with_ids(RequestIds::generate());
        let (envelope, headers) =
            match transport.send_with_headers::<VersionDetails>(&request, &self.token, &self.quirks).await {
                Ok(pair) => pair,
                Err(e) => {
                    report.fail(
                        "details.get",
                        "the version-details endpoint answers",
                        format!("{}: {e}", best.url.as_str()),
                        SPEC,
                    );
                    return None;
                }
            };

        Self::check_echoed_ids(report, &request.ids, &headers);

        let Some(details) = envelope.data else {
            report.fail("details.data", "version details carry a data field", "no `data`", SPEC);
            return None;
        };

        report.assert(
            "details.version",
            "the details name the version they were fetched for",
            details.version == best.version,
            format!("asked for {}, got {}", best.version, details.version),
            SPEC,
        );

        Some((best.version.clone(), details))
    }

    /// Everything that can be checked about the endpoint list without calling any of it.
    fn check_endpoints(
        transport: &Transport,
        report: &mut Report,
        version: &VersionNumber,
        details: &VersionDetails,
    ) {
        const SPEC: &str = "2.3.0 §version_information_endpoint_endpoint_class";

        report.assert(
            "endpoints.nonempty",
            "the version details list at least one endpoint",
            !details.endpoints.is_empty(),
            format!("{} listed", details.endpoints.len()),
            SPEC,
        );

        report.assert(
            "endpoints.credentials",
            "the credentials module is offered",
            details.credentials_url().is_some(),
            "every implementation must have a credentials endpoint",
            "2.3.0 §credentials_credentials_module",
        );

        let mut pairs: Vec<(String, InterfaceRole)> =
            details.endpoints.iter().map(|e| (e.identifier.to_string(), e.role)).collect();
        pairs.sort();
        let duplicate = pairs.windows(2).find(|w| w[0] == w[1]).map(|w| w[0].clone());
        report.assert(
            "endpoints.unique",
            "no module and role pair is listed twice",
            duplicate.is_none(),
            duplicate.map_or_else(String::new, |(m, r)| format!("{m}/{r} appears more than once")),
            SPEC,
        );

        for endpoint in &details.endpoints {
            if !endpoint.identifier.exists_in(version) {
                report.warn(
                    "endpoints.known",
                    "every advertised module exists in this version",
                    format!("`{}` is not a module of OCPI {version}", endpoint.identifier),
                    "2.3.0 §version_information_endpoint_moduleid_enum",
                );
            }
            if let Err(e) = endpoint.url.parse() {
                report.fail(
                    "endpoints.absolute",
                    "every endpoint URL is absolute",
                    format!("{}/{}: {e}", endpoint.identifier, endpoint.role),
                    "2.3.0 §types_url_type",
                );
            }
            // An endpoint a client will refuse to call is as unusable as one that 404s, and the
            // reason is invisible from the peer's side: a version-details document generated
            // from an internal address behind a reverse proxy publishes `http://10.0.0.5:8080/…`
            // to the whole network, and every partner's SSRF guard — including this crate's —
            // declines it.
            if let Err(e) = transport.url_policy().check(&endpoint.url) {
                report.fail(
                    "endpoints.reachable",
                    "every endpoint URL is one a partner may call",
                    format!("{}/{}: {e}", endpoint.identifier, endpoint.role),
                    "2.3.0 §version_information_endpoint_endpoint_class",
                );
            }
        }

        if let Err(violations) = details.validate() {
            for v in &violations {
                report.warn(
                    "details.conform",
                    "the version details conform",
                    format!("{}: {}", v.pointer, v.message),
                    SPEC,
                );
            }
        }
    }

    /// Two requests that must be refused: no token at all, and a token that is not ours.
    async fn check_authentication(
        &self,
        transport: &Transport,
        report: &mut Report,
        details: &VersionDetails,
    ) {
        const SPEC: &str = "2.3.0 §transport_and_format_authorization_header";

        let Some((module, url)) = PULLABLE
            .iter()
            .find_map(|m| details.url(m, InterfaceRole::Sender).map(|u| (m.clone(), u.clone())))
        else {
            report.skip(
                "auth.unauthenticated",
                "an unauthenticated request is refused",
                "the peer offers no Sender interface to try it on",
                SPEC,
            );
            return;
        };

        for (id, title, token) in [
            (
                "auth.empty",
                "a request with an empty token is refused with 401",
                CredentialsToken::new_lenient(String::new()),
            ),
            (
                "auth.wrong",
                "a request with a token that is not ours is refused with 401",
                CredentialsToken::new_lenient("ocpi-kit-conformance-not-a-real-token"),
            ),
        ] {
            let request = OcpiRequest::new(http::Method::GET, url.clone(), module.clone())
                .with_ids(RequestIds::generate());
            let outcome = transport.send::<serde_json::Value>(&request, &token, &self.quirks).await;
            match outcome {
                Err(OcpiError::Unauthorized(_)) => {
                    report.pass(id, title, "401, as required", SPEC);
                }
                Err(OcpiError::NotFound(_)) => {
                    // Also defensible: the peer refuses to say the endpoint exists at all.
                    report.pass(id, title, "404 — the peer will not confirm the endpoint exists", SPEC);
                }
                Ok(_) => report.fail(
                    id,
                    title,
                    "the peer answered 200 to an unauthenticated request, exposing its data",
                    SPEC,
                ),
                Err(other) => report.warn(id, title, format!("expected 401, got {other}"), SPEC),
            }
        }
    }

    /// One page from every Sender interface the peer offers.
    async fn check_modules(&self, transport: &Transport, report: &mut Report, details: &VersionDetails) {
        for module in PULLABLE {
            let Some(url) = details.url(module, InterfaceRole::Sender) else {
                report.skip(
                    "module.page",
                    &format!("{module} Sender returns a decodable page"),
                    "not offered by this peer",
                    "2.3.0 §transport_and_format_pagination",
                );
                continue;
            };
            self.check_one_module(transport, report, module, url).await;
        }
    }

    async fn check_one_module(
        &self,
        transport: &Transport,
        report: &mut Report,
        module: &ModuleId,
        url: &Url,
    ) {
        const SPEC: &str = "2.3.0 §transport_and_format_pagination";
        let query = PageQuery::new().with_limit(self.page_limit);
        let request = OcpiRequest::new(http::Method::GET, query.apply_to(url), module.clone())
            .with_ids(RequestIds::generate());

        // Decoded as `Value` first: a page that fails to decode into the typed object is a
        // finding about the *objects*, not about pagination, and both are worth separating.
        let page = match transport.send_page::<serde_json::Value>(&request, &self.token, &self.quirks).await {
            Ok(page) => page,
            Err(e) => {
                report.fail(
                    "module.page",
                    &format!("{module} Sender returns a decodable page"),
                    e.to_string(),
                    SPEC,
                );
                return;
            }
        };

        report.pass(
            "module.page",
            &format!("{module} Sender returns a decodable page"),
            format!("{} object(s)", page.items.len()),
            SPEC,
        );

        let count = page.items.len() as u64;
        report.assert(
            "module.limit",
            &format!("{module} honours the requested limit"),
            count <= self.page_limit,
            format!("asked for at most {}, got {count}", self.page_limit),
            SPEC,
        );

        match page.meta.limit {
            Some(limit) => {
                report.assert(
                    "module.xlimit",
                    &format!("{module} reports X-Limit consistently"),
                    count <= limit,
                    format!("X-Limit: {limit}, body carried {count}"),
                    SPEC,
                );
            }
            None => report.warn(
                "module.xlimit",
                &format!("{module} sends an X-Limit header"),
                "absent, so a client cannot tell whether its limit was reduced",
                SPEC,
            ),
        }

        match page.meta.total_count {
            Some(total) => {
                let expects_next = total > count;
                report.assert(
                    "module.link",
                    &format!("{module} sends Link: rel=\"next\" exactly when there is more"),
                    expects_next == page.meta.next.is_some(),
                    format!(
                        "X-Total-Count: {total}, this page: {count}, next link: {}",
                        page.meta.next.as_ref().map_or("absent", |_| "present")
                    ),
                    SPEC,
                );
            }
            None => report.warn(
                "module.total",
                &format!("{module} sends an X-Total-Count header"),
                "absent, so a client cannot size the crawl",
                SPEC,
            ),
        }

        // The next link is a URL this client is about to call, on a peer's say-so.
        if let Some(next) = page.meta.next.as_ref() {
            let same_host = next.parse().ok().and_then(|n| n.host_str().map(str::to_owned))
                == url.parse().ok().and_then(|u| u.host_str().map(str::to_owned));
            report.assert(
                "module.link_host",
                &format!("{module}'s next link stays on the same host"),
                same_host && transport.url_policy().check(next).is_ok(),
                format!("{module} pages on to {next}"),
                SPEC,
            );
        }

        Self::check_objects(report, module, &page.items);
        self.check_offset(transport, report, module, url, &page).await;
        self.check_date_from(transport, report, module, url, &page).await;
    }

    /// Whether the peer actually applies `offset`.
    ///
    /// A peer that ignores it answers every page with the same objects, which is not a wrong
    /// answer so much as an endless one: a client following `Link: rel="next"` never terminates,
    /// and `DEFAULT_MAX_PAGES` is the only thing between it and a loop. Nothing in a single page
    /// reveals this, which is why it belongs in a conformance run rather than in a client.
    ///
    /// > *Example: With offset=0 and limit=10 the server shall return the first 10 records (if 10
    /// > objects match the request). Then the next page starts with offset=10.*
    async fn check_offset(
        &self,
        transport: &Transport,
        report: &mut Report,
        module: &ModuleId,
        url: &Url,
        first: &Page<serde_json::Value>,
    ) {
        const SPEC: &str = "2.3.0 §transport_and_format_pagination";
        let title = format!("{module} applies the offset parameter");
        if first.items.len() < 2 {
            report.skip("module.offset", &title, "fewer than two objects to distinguish", SPEC);
            return;
        }
        let query = PageQuery::new().with_offset(1).with_limit(1);
        let request = OcpiRequest::new(http::Method::GET, query.apply_to(url), module.clone())
            .with_ids(RequestIds::generate());
        match transport.send_page::<serde_json::Value>(&request, &self.token, &self.quirks).await {
            Err(e) => report.fail("module.offset", &title, e.to_string(), SPEC),
            Ok(second) => match second.items.first() {
                None => report.warn(
                    "module.offset",
                    &title,
                    "offset=1&limit=1 returned nothing, although the unfiltered page had at least two                      objects",
                    SPEC,
                ),
                Some(item) => report.assert(
                    "module.offset",
                    &title,
                    *item == first.items[1],
                    if *item == first.items[0] {
                        "offset=1 returned the object at offset 0; a crawl over this endpoint would                          never terminate"
                            .to_owned()
                    } else {
                        "offset=1 returned an object, though not the second of the first page —                          acceptable if the set changed between the two requests"
                            .to_owned()
                    },
                    SPEC,
                ),
            },
        }
    }

    /// Whether the peer actually applies `date_from`.
    ///
    /// > *`date_from`: Only return objects that have `last_updated` after or equal to this
    /// > Date/Time (inclusive).*
    ///
    /// A peer that ignores it turns every incremental pull into a full one. The cost is invisible
    /// — the data is correct — until a partner with a million CDRs wonders why a nightly sync
    /// takes six hours.
    async fn check_date_from(
        &self,
        transport: &Transport,
        report: &mut Report,
        module: &ModuleId,
        url: &Url,
        first: &Page<serde_json::Value>,
    ) {
        const SPEC: &str = "2.3.0 §transport_and_format_pagination";
        let title = format!("{module} applies the date_from filter");
        let Some(newest) = first.items.iter().filter_map(last_updated).max() else {
            report.skip("module.date_from", &title, "no object carried a usable last_updated", SPEC);
            return;
        };
        // One second *past* the newest object this peer just showed us. A peer that applies the
        // filter answers with nothing, or with whatever changed in between; a peer that ignores it
        // hands back the same page, every object of which is now demonstrably too old. Filtering
        // at `newest` itself would prove nothing when a page's objects share a timestamp, which
        // is the normal case for a bulk import.
        let Ok(after) = DateTime::from_unix_timestamp(newest.unix_timestamp() + 1) else {
            report.skip("module.date_from", &title, "the newest timestamp is at the end of time", SPEC);
            return;
        };
        let query = PageQuery::since(after).with_limit(self.page_limit);
        let request = OcpiRequest::new(http::Method::GET, query.apply_to(url), module.clone())
            .with_ids(RequestIds::generate());
        match transport.send_page::<serde_json::Value>(&request, &self.token, &self.quirks).await {
            Err(e) => report.fail("module.date_from", &title, e.to_string(), SPEC),
            Ok(filtered) => {
                let stale = filtered.items.iter().filter_map(last_updated).filter(|t| *t < after).count();
                let total = filtered.items.len();
                report.assert(
                    "module.date_from",
                    &title,
                    stale == 0,
                    if stale == 0 {
                        format!("asked for last_updated >= {after}, got {total} object(s), none older")
                    } else {
                        format!(
                            "asked for last_updated >= {after}, got {stale} of {total} object(s) older \
                             than that; a peer that ignores date_from turns every incremental pull \
                             into a full one"
                        )
                    },
                    SPEC,
                );
            }
        }
    }

    /// Re-decodes the page as the typed object and validates each one.
    fn check_objects(report: &mut Report, module: &ModuleId, items: &[serde_json::Value]) {
        macro_rules! typed {
            ($ty:ty) => {{
                let mut decoded = 0usize;
                let mut problems: Vec<String> = Vec::new();
                for (i, raw) in items.iter().enumerate() {
                    match serde_json::from_value::<$ty>(raw.clone()) {
                        Ok(object) => {
                            decoded += 1;
                            if let Err(violations) = object.validate() {
                                for v in violations.iter() {
                                    problems.push(format!("[{i}]{}: {}", v.pointer, v.message));
                                }
                            }
                        }
                        Err(e) => problems.push(format!("[{i}] does not decode: {e}")),
                    }
                }
                (decoded, problems)
            }};
        }

        let (decoded, problems) = match module {
            ModuleId::Locations => typed!(crate::v2_3_0::locations::Location),
            ModuleId::Sessions => typed!(crate::v2_3_0::sessions::Session),
            ModuleId::Cdrs => typed!(crate::v2_3_0::cdrs::Cdr),
            ModuleId::Tariffs => typed!(crate::v2_3_0::tariffs::Tariff),
            ModuleId::Tokens => typed!(crate::v2_3_0::tokens::Token),
            _ => return,
        };

        if problems.is_empty() {
            report.pass(
                "module.objects",
                &format!("{module} objects conform"),
                format!("{decoded} checked"),
                "2.3.0 §types_types",
            );
            return;
        }
        // Cap the detail: a peer with a systematic problem would otherwise print a page of it.
        let shown = problems.iter().take(10).cloned().collect::<Vec<_>>().join("; ");
        let suffix =
            if problems.len() > 10 { format!(" (and {} more)", problems.len() - 10) } else { String::new() };
        report.warn(
            "module.objects",
            &format!("{module} objects conform"),
            format!("{shown}{suffix}"),
            "2.3.0 §types_types",
        );
    }

    fn check_echoed_ids(report: &mut Report, sent: &RequestIds, headers: &http::HeaderMap) {
        const SPEC: &str = "2.3.0 §transport_and_format_request_id";
        let got = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();

        let request_id = got("x-request-id");
        report.assert(
            "headers.request_id",
            "X-Request-ID is echoed",
            request_id == sent.request_id.as_str(),
            if request_id.is_empty() {
                "absent from the response".to_owned()
            } else {
                format!("sent {}, got back {request_id}", sent.request_id)
            },
            SPEC,
        );

        let correlation_id = got("x-correlation-id");
        report.assert(
            "headers.correlation_id",
            "X-Correlation-ID is echoed",
            correlation_id == sent.correlation_id.as_str(),
            if correlation_id.is_empty() {
                "absent from the response".to_owned()
            } else {
                format!("sent {}, got back {correlation_id}", sent.correlation_id)
            },
            SPEC,
        );
    }

    fn check_timestamp(&self, report: &mut Report, timestamp: DateTime) {
        const SPEC: &str = "2.3.0 §transport_and_format_response_format";
        let now = DateTime::now();
        let skew = (now.unix_timestamp() - timestamp.unix_timestamp()).unsigned_abs();
        let allowed = self.max_clock_skew.as_secs();
        report.assert(
            "headers.timestamp",
            "the response timestamp is close to ours",
            skew <= allowed,
            format!("peer says {timestamp}, we say {now} — {skew}s apart, tolerance {allowed}s"),
            SPEC,
        );
    }
}

/// The `last_updated` of an object, whatever module it belongs to.
///
/// Every OCPI object that a Sender interface lists carries one; it is what `date_from` filters on.
fn last_updated(object: &serde_json::Value) -> Option<DateTime> {
    object.get("last_updated")?.as_str()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(outcome: Outcome) -> Check {
        Check::new("t", "t", outcome, "", "spec")
    }

    #[test]
    fn a_report_counts_by_outcome() {
        let report = Report {
            checks: vec![
                check(Outcome::Pass),
                check(Outcome::Pass),
                check(Outcome::Fail),
                check(Outcome::Warn),
                check(Outcome::Skipped),
            ],
            version: None,
        };
        assert_eq!(report.count(Outcome::Pass), 2);
        assert_eq!(report.count(Outcome::Fail), 1);
        assert!(report.has_failures());
        assert_eq!(report.failures().count(), 1);
    }

    #[test]
    fn a_clean_report_has_no_failures() {
        let report = Report { checks: vec![check(Outcome::Pass), check(Outcome::Warn)], version: None };
        assert!(!report.has_failures(), "a warning is not a failure");
    }

    #[test]
    fn the_summary_line_names_every_outcome() {
        let report = Report { checks: vec![check(Outcome::Pass), check(Outcome::Fail)], version: None };
        let text = report.to_string();
        assert!(text.contains("1 passed"), "{text}");
        assert!(text.contains("1 failed"), "{text}");
    }

    #[test]
    fn only_read_only_modules_are_pulled() {
        // A conformance run must not write, so the pull list holds no module whose Sender
        // interface is anything but a list of objects the peer owns.
        for module in PULLABLE {
            assert!(
                matches!(
                    module,
                    ModuleId::Locations
                        | ModuleId::Sessions
                        | ModuleId::Cdrs
                        | ModuleId::Tariffs
                        | ModuleId::Tokens
                ),
                "{module} is not a read-only Sender list endpoint"
            );
        }
    }

    #[test]
    fn outcomes_order_worst_last_for_sorting() {
        assert!(Outcome::Pass < Outcome::Warn);
        assert!(Outcome::Warn < Outcome::Fail);
    }
}
