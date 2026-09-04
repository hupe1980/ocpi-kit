+++
title = "Conformance"
weight = 60
description = "Drive a live OCPI peer through the specification and report where it disagrees. Read-only."
+++

Every party in a roaming network has the same problem: the partner's implementation is not quite
the specification, and finding out *where* means reading their JSON by hand. `ocpi-kit` does it
mechanically and produces a report either side can act on.

```console
$ export OCPI_TOKEN=…            # the CREDENTIALS_TOKEN_C of an existing registration
$ ocpi conformance https://cpo.example.com/ocpi/versions
```

Nothing to point it at yet? [`ocpi serve-mock`](@/docs/getting-started/cli.md) runs a conformant
peer, and this crate's test suite requires that this runner finds nothing wrong with it.

```text
[+] versions.status   GET /versions returns status_code 1000
                      got 1000
[+] versions.common   the peer offers a version this build speaks
                      2.3.0, 2.2.1
[x] headers.request_id  X-Request-ID is echoed
                      absent from the response
                      spec: 2.3.0 §transport_and_format_request_id
[!] module.xlimit     locations sends an X-Limit header
                      absent, so a client cannot tell whether its limit was reduced
                      spec: 2.3.0 §transport_and_format_pagination
[-] module.page       cdrs Sender returns a decodable page
                      not offered by this peer

21 passed, 1 failed, 1 warnings, 4 skipped
```

The exit status is non-zero when anything **failed**, so it drops into CI as a partner-integration
gate. `--no-fail` reports without failing the build.

## It never changes anything

The runner issues `GET` requests and exactly two deliberately-unauthenticated ones. It does not
register, does not POST credentials, does not write an object, and does not delete one. A
conformance check that mutates the peer it is checking is not a conformance check — running this
against a production partner is safe.

`--no-auth-checks` skips even the two rejected requests, for a partner whose intrusion detection
would rather not see a failed authentication from you.

## What it checks

Discovery and the transport rules come first, because a peer that gets those wrong fails
everything downstream for reasons that are hard to read.

| Check | The rule |
|---|---|
| `versions.get` / `versions.status` / `versions.data` | `/versions` answers, with a `1000` envelope carrying a version list |
| `versions.nonempty` / `versions.unique` | at least one version, each listed once |
| `versions.url` | every advertised URL passes the `UrlPolicy` |
| `versions.common` | at least one version this build speaks |
| `details.get` / `details.data` / `details.version` | the details fetch, carry a payload, and name the version they were fetched for |
| `endpoints.nonempty` / `endpoints.credentials` | at least one endpoint, and the credentials module among them — every implementation must have it |
| `endpoints.unique` | no `(module, role)` pair listed twice |
| `endpoints.known` | no module advertised that does not exist in that version |
| `endpoints.absolute` | every endpoint URL is absolute |
| `endpoints.reachable` | every endpoint URL is one a partner may actually call |
| `details.conform` | the version-details object itself conforms |
| `headers.request_id` / `headers.correlation_id` | both echoed on the response |
| `headers.timestamp` | the envelope timestamp is within the clock skew you allow |
| `auth.empty` / `auth.wrong` / `auth.unauthenticated` | a missing, malformed or wrong credentials token is refused |
| `module.page` | each offered Sender interface returns a decodable page |
| `module.limit` / `module.xlimit` | the requested `limit` is never exceeded, and `X-Limit` agrees |
| `module.link` / `module.total` | `X-Total-Count` is sent, and `Link: rel="next"` appears exactly when it says there is more |
| `module.link_host` | the next link stays on the same host and passes the `UrlPolicy` |
| `module.objects` | the objects on the page conform |
| `module.offset` | `offset` is actually applied |
| `module.date_from` | `date_from` is actually applied |

Every check names the specification anchor it comes from, so a failing line pastes straight into a
ticket without anyone having to look up the clause.

### `endpoints.reachable`

The one failure a peer cannot see from its own side. A version-details document is generated from
whatever base URL the process was configured with, and behind a reverse proxy that is routinely an
internal address — so the peer publishes `http://10.0.0.5:8080/ocpi/cpo/2.3.0/locations` to the
whole network, every partner's SSRF guard declines it (including this crate's,
[by default](@/docs/reference/security.md)), and the peer's own tests pass because they run inside
the network where it resolves.

### The last two

Every other check is visible in a single response. These are not: a peer that ignores `offset`
answers each page correctly and with the same objects, so a client following `Link: rel="next"`
never terminates. A peer that ignores `date_from` returns everything it has every time — also
correct, and it turns a partner's incremental pull into a full one.

Each costs one extra `GET` per module and both stay read-only. The `date_from` probe asks for one
second *past* the newest object the peer just showed, because filtering at the newest timestamp
proves nothing when a page's objects share one.

## Outcomes

* **Pass** — the peer did what the specification says.
* **Fail** — the peer contradicts the specification. This is what sets the exit status.
* **Warn** — not required, but it will cause trouble: a missing `X-Total-Count`, an object with a
  length violation, a module id that is not in the version's table.
* **Skip** — the check could not run, usually because the peer does not implement that module.

## As a library

The runner is a library type, not just CLI code, so it can go in your own integration suite:

```rust
use ocpi_kit::client::{Conformance, OcpiClient};

let report = Conformance::new(versions_url, token)
    .with_quirks(partner_quirks)     // report real problems, not known ones
    .with_page_limit(50)
    .run(client.transport())
    .await;

for check in report.failures() {
    tracing::error!(id = check.id, spec = check.spec, "{}: {}", check.title, check.detail);
}
```

`Conformance::run` never returns an error — a peer that cannot be reached at all is itself a
finding, and is recorded as one.

## It is run against this crate's own server

`tests/end_to_end.rs` points the runner at an `OcpiRouter` over a real socket and asserts that
nothing fails. Every check the runner makes is a rule the router is supposed to follow, so a
failure means one of the two is wrong — and the report says which.
