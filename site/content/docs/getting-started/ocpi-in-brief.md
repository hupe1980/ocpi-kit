+++
title = "OCPI in brief"
weight = 5
description = "The roles, modules and interfaces of the Open Charge Point Interface, and the vocabulary the rest of this guide assumes."
+++

If you already know what a Sender interface is, skip to
[Install and features](@/docs/getting-started/install.md). Everything below is the vocabulary the
rest of the guide assumes.

## The problem OCPI solves

A driver with a contract from one company wants to charge at a charge point owned by another. The
two companies have no shared database, so they need a protocol: to publish where the chargers are,
to authorise the driver, to report what happened, and to agree on what it cost.

That protocol is **OCPI**, published by the [EVRoaming Foundation](https://evroaming.org/ocpi/).

## Roles

| Role | Who it is |
|---|---|
| **CPO** — Charge Point Operator | Owns and operates charge points. Publishes Locations, produces Sessions and CDRs, sets Tariffs. |
| **eMSP** — e-Mobility Service Provider | Has the contract with the driver. Holds Tokens, consumes Locations, receives CDRs. |
| **Hub** | Sits between many CPOs and eMSPs so each connects once instead of *n* times. Routes and, when versions differ, translates. |
| **NAP / NSP / SCSP** | Data aggregators and adjacent roles, mostly read-only. |

A single platform often holds several roles, each with its own `country_code` and `party_id` — the
pair that identifies a party throughout the protocol, written `NL/TNM`.

## Modules

OCPI is organised into modules, each a REST resource with its own endpoint:

`credentials` and `versions` are the configuration modules — how two parties meet. `locations`,
`sessions`, `cdrs`, `tariffs`, `tokens`, `commands`, `chargingprofiles` and `hubclientinfo` are the
functional modules that carry the actual traffic. OCPI 2.3.0 core adds `invoicereconciliation`,
and its two **release branches** add `payments` and `Booking`.

A release branch is not a separate protocol: it is core plus one module, plus the fields that
module adds to core objects. This crate has one cargo feature per branch, and which release defines
what is [checked against pinned sources](@/docs/reference/traceability.md) rather than remembered —
the branches do move.

## Sender and Receiver

This is the distinction that trips up most newcomers, and it is the one `ocpi-kit` models most
explicitly.

Each module has up to two interfaces, and **which role implements which depends on the module**:

* The **Sender** owns the data and offers it. For Locations the CPO is the Sender: it is the CPO's
  charge points, so the CPO answers `GET /locations`.
* The **Receiver** is given data it does not own. For Locations the eMSP is the Receiver: the CPO
  `PUT`s Locations to it.

For Tokens it is the other way round — the eMSP owns the tokens, so the eMSP is the Sender. Never
assume "CPO means Sender".

The two interfaces also address objects differently, which is why they are separate types in this
crate:

```text
Sender    GET /locations/{location_id}
Receiver  PUT /locations/{country_code}/{party_id}/{location_id}
```

A Receiver path carries the *owner*, because the objects there belong to the client.

## Push and pull

A Sender may **push** changes to a Receiver as they happen (`PUT`/`PATCH`), and a Receiver may
**pull** the current state (`GET`, paginated). Most integrations do both: push for freshness, pull
to recover after an outage.

The specification is explicit that a failed push must **not** be queued and replayed. Recovery is a
pull. See [Client](@/docs/layers/client.md).

## Registration

Two parties exchange credentials once, out of band, then bootstrap:

1. They agree on `CREDENTIALS_TOKEN_A` by email or a portal.
2. The initiating party `GET`s the other's `/versions`, picks the newest common version, and
   fetches its endpoint list.
3. It `POST`s its own credentials — including `CREDENTIALS_TOKEN_B`, which the other party will use
   to call *it* — and receives `CREDENTIALS_TOKEN_C` in return.
4. `CREDENTIALS_TOKEN_A` may no longer be used.

`ocpi-kit` makes those states types, so the common mistakes are compile errors. See
[Client](@/docs/layers/client.md).

## The envelope

Every response body has the same shape, and the four-digit `status_code` inside it — not the HTTP
status — is how OCPI reports most problems:

```json
{
  "data": [ … ],
  "status_code": 1000,
  "status_message": "Success",
  "timestamp": "2024-03-01T10:00:00Z"
}
```

`1xxx` success, `2xxx` client error, `3xxx` server error, `4xxx` hub error. Only five situations
get an HTTP error status at all. See [Transport](@/docs/layers/transport.md).

## Versions in the wild

**2.2.1** is what most production traffic speaks today. **2.3.0** is current, adding Payments (on
its own release branch),
richer parking and tax modelling. **2.1.1** is legacy but still deployed, and differs enough to
need its own model. 3.0 is an unreleased draft.

`ocpi-kit` implements all three, and converts between them with a report of what a translation
cost. See [Versions and conversion](@/docs/concepts/versions.md).
