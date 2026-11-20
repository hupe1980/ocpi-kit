//! Where the client is allowed to connect: the redirect rule and the connect-time address check.
//!
//! OCPI makes a party fetch URLs its counterparty chose — a versions URL, every endpoint of a
//! version details answer, every `next` link, every command callback. [`UrlPolicy`] judges the
//! URL text. Two more layers make that judgement hold for the request that is actually sent:
//!
//! * **Every hop.** The HTTP client never follows a redirect on its own. [`Transport`] follows a
//!   redirect only for a `GET`, only within the origin it asked (or an `http` → `https` upgrade of
//!   it), only to a URL the policy accepts, and at most [`MAX_REDIRECTS`] times.
//! * **Every connection.** [`GuardedResolver`] sits in the connection path and drops every
//!   resolved address the policy does not permit. The addresses it checks are the addresses
//!   connected to, so a name that answers differently between a check and a connection
//!   (DNS rebinding) cannot slip through.
//!
//! [`Transport`]: super::Transport

use std::fmt;
use std::net::{IpAddr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::types::{Url, UrlPolicy};

/// How many redirects one request follows before the client gives up.
pub const MAX_REDIRECTS: usize = 5;

/// Resolves host names for the HTTP client and keeps only the addresses a [`UrlPolicy`] permits.
///
/// Installed by [`ClientConfig::http_builder`](super::ClientConfig::http_builder). A name with no
/// permitted address fails the connection with an error naming the host and what it resolved to,
/// which the client reports as [`OcpiError::UrlRefused`](crate::transport::OcpiError::UrlRefused).
///
/// A URL whose host is an IP literal never reaches a resolver; [`UrlPolicy::check`] covers it.
/// Behind an HTTP proxy the resolver sees the proxy, not the target: the proxy is then the place
/// to enforce egress rules.
#[derive(Clone, Debug)]
pub struct GuardedResolver {
    policy: UrlPolicy,
    #[cfg(test)]
    fixed: Option<std::collections::HashMap<String, Vec<IpAddr>>>,
}

impl GuardedResolver {
    /// A resolver that asks the system resolver and filters by `policy`.
    #[must_use]
    pub fn new(policy: UrlPolicy) -> Self {
        Self {
            policy,
            #[cfg(test)]
            fixed: None,
        }
    }

    /// A resolver answering from a table instead of the network, for tests.
    #[cfg(test)]
    pub(crate) fn fixed(policy: UrlPolicy, table: &[(&str, IpAddr)]) -> Self {
        let mut map = std::collections::HashMap::<String, Vec<IpAddr>>::new();
        for (name, ip) in table {
            map.entry((*name).to_owned()).or_default().push(*ip);
        }
        Self { policy, fixed: Some(map) }
    }

    async fn lookup(&self, host: &str) -> std::io::Result<Vec<IpAddr>> {
        #[cfg(test)]
        if let Some(table) = &self.fixed {
            return table
                .get(host)
                .cloned()
                .ok_or_else(|| std::io::Error::other(format!("{host}: no such name")));
        }
        Ok(tokio::net::lookup_host((host, 0)).await?.map(|a| a.ip()).collect())
    }
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let this = self.clone();
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let resolved = this.lookup(&host).await?;
            let (permitted, refused): (Vec<IpAddr>, Vec<IpAddr>) =
                resolved.into_iter().partition(|ip| this.policy.permits_address(*ip));
            if permitted.is_empty() {
                return Err(Box::new(AddressRefused { host, addresses: refused }) as _);
            }
            // Port 0 is replaced by the URL's port (or the scheme's default) by the connector.
            let addrs: Addrs = Box::new(permitted.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

/// A host name resolved only to addresses the [`UrlPolicy`] refuses.
#[derive(Debug)]
pub(crate) struct AddressRefused {
    pub(crate) host: String,
    addresses: Vec<IpAddr>,
}

impl fmt::Display for AddressRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown: Vec<String> = self.addresses.iter().take(4).map(ToString::to_string).collect();
        write!(
            f,
            "{} resolves only to addresses the URL policy refuses ({}{}): not globally reachable",
            self.host,
            shown.join(", "),
            if self.addresses.len() > 4 { ", …" } else { "" }
        )
    }
}

impl std::error::Error for AddressRefused {}

/// Why a redirect was not followed: the target as it may be shown (no query, no fragment — the
/// peer chose them, and they can carry anything) and the rule that refused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RedirectRefused {
    pub(crate) target: String,
    pub(crate) reason: String,
}

/// Decides whether the client may follow a redirect from `current` to `location`, and where to.
///
/// `hops` is how many redirects this request has already followed.
pub(crate) fn redirect_target(
    policy: &UrlPolicy,
    method: &http::Method,
    current: &url::Url,
    location: &str,
    hops: usize,
) -> Result<url::Url, RedirectRefused> {
    let Ok(target) = current.join(location) else {
        return Err(RedirectRefused {
            target: shown(location),
            reason: "the peer redirected to something that is not a URL".to_owned(),
        });
    };
    let refuse = |reason: String| Err(RedirectRefused { target: redacted(&target), reason });
    if method != http::Method::GET {
        return refuse(format!(
            "the peer redirected a {method}; a write is never redirected, because it would be replayed or \
             turned into a different request"
        ));
    }
    if hops >= MAX_REDIRECTS {
        return refuse(format!("more than {MAX_REDIRECTS} redirects"));
    }
    if !same_origin(current, &target) {
        return refuse(format!(
            "the redirect leaves the origin {}; if the peer has moved, register the new URL instead",
            current.origin().ascii_serialization()
        ));
    }
    if let Err(e) = policy.check(&Url::from(target.clone())) {
        return refuse(e.to_string());
    }
    Ok(target)
}

/// Same scheme, host and port — or an `http` → `https` upgrade of the same host on default ports,
/// which a permissive development policy should keep following.
fn same_origin(from: &url::Url, to: &url::Url) -> bool {
    if from.origin() == to.origin() {
        return true;
    }
    from.scheme() == "http"
        && to.scheme() == "https"
        && from.host() == to.host()
        && from.port().is_none()
        && to.port().is_none()
}

/// A URL with its query and fragment removed, for an error message.
pub(crate) fn redacted(url: &url::Url) -> String {
    let mut shown = url.clone();
    shown.set_query(None);
    shown.set_fragment(None);
    shown.to_string()
}

fn shown(text: &str) -> String {
    let cut = text.find(['?', '#']).unwrap_or(text.len());
    text[..cut].chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    fn follow(
        method: http::Method,
        from: &str,
        location: &str,
        hops: usize,
    ) -> Result<String, RedirectRefused> {
        redirect_target(&UrlPolicy::default(), &method, &u(from), location, hops).map(|u| u.to_string())
    }

    #[test]
    fn a_trailing_slash_redirect_within_the_origin_is_followed() {
        assert_eq!(
            follow(http::Method::GET, "https://cpo.example/ocpi/versions", "/ocpi/versions/", 0).unwrap(),
            "https://cpo.example/ocpi/versions/"
        );
    }

    #[test]
    fn a_redirect_leaving_the_origin_is_refused_whatever_the_target() {
        for location in [
            "https://other.example/ocpi",
            "https://cpo.example:8443/ocpi",
            "http://169.254.169.254/latest/meta-data/",
            "http://cpo.example/ocpi",
        ] {
            let refused =
                follow(http::Method::GET, "https://cpo.example/ocpi/versions", location, 0).unwrap_err();
            assert!(refused.reason.contains("leaves the origin"), "{location}: {}", refused.reason);
        }
    }

    #[test]
    fn a_write_is_never_redirected() {
        for method in [http::Method::POST, http::Method::PUT, http::Method::PATCH, http::Method::DELETE] {
            let refused = follow(method.clone(), "https://cpo.example/a", "/b", 0).unwrap_err();
            assert!(refused.reason.contains("never redirected"), "{method}");
        }
    }

    #[test]
    fn redirects_are_bounded() {
        assert!(follow(http::Method::GET, "https://cpo.example/a", "/b", MAX_REDIRECTS - 1).is_ok());
        let refused = follow(http::Method::GET, "https://cpo.example/a", "/b", MAX_REDIRECTS).unwrap_err();
        assert!(refused.reason.contains("more than 5"), "{}", refused.reason);
    }

    #[test]
    fn a_same_host_upgrade_to_https_is_followed_under_a_policy_allowing_http() {
        let policy = UrlPolicy::permissive();
        let next = redirect_target(
            &policy,
            &http::Method::GET,
            &u("http://cpo.example/a"),
            "https://cpo.example/a",
            0,
        );
        assert_eq!(next.unwrap().as_str(), "https://cpo.example/a");
    }

    #[test]
    fn a_refusal_does_not_repeat_what_the_peer_put_in_the_query() {
        let refused = follow(
            http::Method::GET,
            "https://cpo.example/a",
            "https://other.example/x?token=secret#frag",
            0,
        )
        .unwrap_err();
        assert_eq!(refused.target, "https://other.example/x");
        assert!(!format!("{refused:?}").contains("secret"));
    }

    /// The connect-time check end to end: a public-looking name whose record points at loopback.
    #[tokio::test]
    async fn a_name_resolving_to_a_private_address_is_refused_at_connect_time() {
        use crate::client::{ClientConfig, OcpiRequest, Transport};
        use crate::transport::{CredentialsToken, OcpiError, Quirks};

        let listener = wiremock::MockServer::start().await;
        let port = listener.address().port();
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let url = Url::new(format!("http://peer.test:{port}/versions")).unwrap();
        let send = |policy: UrlPolicy| {
            let config = ClientConfig::default().with_url_policy(policy.clone());
            let http = config
                .http_builder()
                .dns_resolver(GuardedResolver::fixed(policy, &[("peer.test", loopback)]))
                .build()
                .unwrap();
            let request = OcpiRequest::new(http::Method::GET, url.clone(), crate::ModuleId::Credentials);
            async move {
                Transport::new(http, config)
                    .send::<serde_json::Value>(
                        &request,
                        &CredentialsToken::new("t").unwrap(),
                        &Quirks::default(),
                    )
                    .await
            }
        };

        // HTTP allowed, private networks not: the name passes the URL check, its address does not.
        let error = send(UrlPolicy::default().allowing_http()).await.expect_err("refused");
        let OcpiError::UrlRefused { url, reason } = &error else {
            panic!("expected UrlRefused, got {error:?}")
        };
        assert_eq!(url, "peer.test");
        assert!(reason.contains("127.0.0.1"), "{reason}");
        assert!(listener.received_requests().await.unwrap().is_empty(), "nothing reached the listener");

        // The same name under a policy allowing private networks connects (and gets a 404).
        let error = send(UrlPolicy::permissive()).await.expect_err("the listener has no routes");
        assert!(matches!(error, OcpiError::NotFound(_)), "{error:?}");
        assert_eq!(listener.received_requests().await.unwrap().len(), 1);
    }

    #[test]
    fn the_resolver_drops_refused_addresses_and_fails_when_none_remain() {
        let resolver = GuardedResolver::fixed(
            UrlPolicy::default(),
            &[
                ("peer.test", "127.0.0.1".parse().unwrap()),
                ("mixed.test", "10.0.0.1".parse().unwrap()),
                ("mixed.test", "8.8.8.8".parse().unwrap()),
            ],
        );
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let error = match resolver.resolve("peer.test".parse().unwrap()).await {
                Err(e) => e.to_string(),
                Ok(_) => panic!("a name resolving only to loopback must be refused"),
            };
            assert!(error.contains("peer.test") && error.contains("127.0.0.1"), "{error}");
            let Ok(addrs) = resolver.resolve("mixed.test".parse().unwrap()).await else {
                panic!("a public address remains");
            };
            let addrs: Vec<SocketAddr> = addrs.collect();
            assert_eq!(addrs, vec!["8.8.8.8:0".parse::<SocketAddr>().unwrap()]);
        });
    }
}
