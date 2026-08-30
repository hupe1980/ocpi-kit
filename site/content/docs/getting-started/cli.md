+++
title = "The CLI"
weight = 30
description = "Validate payloads, inspect a peer, crawl a module, price a CDR and check conformance from a terminal."
+++

```console
cargo install ocpi-kit --features cli
```

The `ocpi` binary is the crate's own functionality on a command line — useful for checking a
partner's payloads, debugging a handshake, or settling an invoice dispute without writing code.

## `validate` — is this file conformant?

```console
$ ocpi validate --as location location.json
$ ocpi validate --as cdr --version 2.2.1 cdr.json
$ cat location.json | ocpi validate --as location -
```

Reports every violation with a JSON Pointer, so the output pastes straight into a support ticket.
`--as` accepts `location`, `evse`, `connector`, `session`, `cdr`, `tariff`, `token`,
`credentials` and `version-details`; `--version` accepts `2.1.1`, `2.2.1` and `2.3.0`.

## `versions` — what does this peer speak?

```console
$ export OCPI_TOKEN=…
$ ocpi versions https://cpo.example.com/ocpi/versions
```

Fetches the peer's version list and the details of the newest version this build has in common
with it, printing every endpoint, its role and its URL. This is the fastest way to find out why a
handshake is failing.

## `pull` — crawl a Sender list endpoint

```console
$ ocpi pull locations https://cpo.example.com/ocpi/versions
$ ocpi pull cdrs https://cpo.example.com/ocpi/versions --from 2024-03-01T00:00:00Z
$ ocpi pull payment-terminals https://ptp.example.com/ocpi/versions
```

Follows every `Link: rel="next"` header to the end and prints the objects, one JSON document per
line, so the output pipes into `jq`.

Modules: `locations`, `sessions`, `cdrs`, `tariffs`, `tokens`, `hub-client-info`,
`payment-terminals` and `financial-advice-confirmations`. The last two are the Payments module's
two list endpoints — it declares one `ModuleID` and then addresses both through separate endpoint
URLs, which version discovery cannot express, so which of the two you want has to be said here.
See [Spec errata](@/docs/reference/errata.md).

## `price` — what should this session have cost?

```console
$ ocpi price cdr.json --tariff tariff.json --time-zone Europe/Berlin
```

Prices the CDR against the given Tariffs and prints the auditable breakdown, then compares the
total with the one the CDR itself claims — the invoice check, in one command. `--tariff` may be
given several times. `--no-step-size` bills measured quantities exactly, as OCPI 3.0 will.

**It exits non-zero when the CDR does not check out**, so this is a pipeline step rather than
something a person has to read. A note is enough on its own: a CDR whose Charging Periods span a
price change can total correctly by luck and still be malformed, and the breakdown says which
period and which dimension. See [Tariffs](@/docs/layers/tariffs.md).

## `convert` — move an object between versions

```console
$ ocpi convert --as cdr --from 2.2.1 --to 2.3.0 cdr.json
```

Prints the converted object on stdout and everything that did not survive the crossing on stderr,
one `pointer<TAB>reason` per line, so a lossy conversion is visible in a pipeline rather than
silent.

## `conformance` — check a live peer, without changing it

```console
$ ocpi conformance https://cpo.example.com/ocpi/versions
```

Drives the peer through discovery, the transport rules, authentication and one page of every
Sender interface it offers, and prints a report naming the specification anchor behind each check.
Read-only, and non-zero on failure so it drops into CI as a partner-integration gate. See
[Conformance](@/docs/layers/conformance.md).

## `serve-mock` — a conformant peer to integrate against

```console
$ ocpi serve-mock                                   # a CPO on 127.0.0.1:8080, one of each object
$ ocpi serve-mock --role emsp --version 2.2.1
$ ocpi serve-mock --bind 0.0.0.0:8080 --base-url https://mock.internal/ocpi/cpo/2.3.0
```

The other side of an integration, on a socket: an endpoint that paginates, applies `date_from`,
refuses a write under the wrong party, answers `2004` for an unknown token and rejects a `PATCH`
with no `last_updated`. It runs [`testkit::MockPeer`](@/docs/layers/testkit.md), which the test
suite holds to this crate's own [conformance runner](@/docs/layers/conformance.md).

`--version 2.2.1` serves 2.2.1 bytes from the same handlers —
[one model, every version](@/docs/concepts/versions.md).

It mounts the five object modules on both interfaces plus `credentials`. Commands, Charging
Profiles and Payments are deliberately absent; version discovery advertises exactly what is
mounted.

```console
$ ocpi serve-mock &
$ ocpi --insecure conformance http://127.0.0.1:8080/versions --token test-token-c
…
45 passed, 0 failed, 0 warnings, 5 skipped
```

## `schema` — JSON Schema for non-Rust partners

```console
$ ocpi schema location --version 2.3.0 > location.schema.json
```

## A note on `--insecure`

Every subcommand that fetches a URL refuses plain HTTP, loopback and private network addresses by
default. `Credentials.url`, `Endpoint.url` and every `response_url` come from a peer; a tool that
fetches them unconditionally is an SSRF proxy. `--insecure` lifts the restriction for local
testing. See [Security](@/docs/reference/security.md).
