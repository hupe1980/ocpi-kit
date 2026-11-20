# Security policy

`ocpi-kit` handles other parties' credentials tokens and fetches URLs that other parties chose,
so a vulnerability here can reach beyond the deployment that runs it. Reports are welcome.

## Reporting a vulnerability

Please report privately through GitHub: **Security → Report a vulnerability** on
<https://github.com/hupe1980/ocpi-kit/security/advisories/new>. Do not open a public issue for a
suspected vulnerability.

Include the version, the feature set, and the smallest input or sequence of requests that shows
the problem. You will get an acknowledgement within a week; a fix for a confirmed issue is
released as a patch version with a GitHub security advisory and a `CHANGELOG.md` entry.

## In scope

- Requests the client can be made to send somewhere the `UrlPolicy` refuses (SSRF), including
  through redirects or name resolution.
- Credentials tokens exposed in logs, errors, `Debug` output or to a party that should not see them.
- A peer's document causing a panic, unbounded memory or CPU, or a wrong amount of money.
- The server accepting a write for an object the authenticated party does not own.

## Supported versions

The latest released minor version receives security fixes. Before 1.0, that is the only one.
