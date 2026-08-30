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
```

Follows every `Link: rel="next"` header to the end and prints the objects. Modules: `locations`,
`sessions`, `cdrs`, `tariffs`, `tokens`.

## `price` — what should this session have cost?

```console
$ ocpi price cdr.json --tariff tariff.json --time-zone Europe/Berlin
```

Prices the CDR against the given Tariffs and prints the auditable breakdown, then compares the
total with the one the CDR itself claims — the invoice check, in one command. `--tariff` may be
given several times. `--no-step-size` bills measured quantities exactly, as OCPI 3.0 will.

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

## `schema` — JSON Schema for non-Rust partners

```console
$ ocpi schema location --version 2.3.0 > location.schema.json
```

## A note on `--insecure`

Every subcommand that fetches a URL refuses plain HTTP, loopback and private network addresses by
default. `Credentials.url`, `Endpoint.url` and every `response_url` come from a peer; a tool that
fetches them unconditionally is an SSRF proxy. `--insecure` lifts the restriction for local
testing. See [Security](@/docs/reference/security.md).
