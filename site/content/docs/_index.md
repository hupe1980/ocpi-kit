+++
title = "Documentation"
description = "Guides and reference for ocpi-kit — the ideas behind the crate, a walkthrough of every layer, and the interop knowledge you need to talk to real OCPI peers."
sort_by = "weight"
template = "section.html"
page_template = "page.html"
+++

`ocpi-kit` implements [OCPI](https://evroaming.org/ocpi/) — the Open Charge Point Interface, the
protocol that carries EV roaming traffic between Charge Point Operators, e-Mobility Service
Providers and roaming hubs.

This guide explains the ideas the crate is built on and how each layer is meant to be used. For
item-by-item API detail, see [docs.rs/ocpi-kit](https://docs.rs/ocpi-kit).

New to the protocol? [OCPI in brief](@/docs/getting-started/ocpi-in-brief.md) covers the roles,
modules and the Sender/Receiver distinction the rest of this guide assumes.

## Where to start

* You want to talk to a peer → [Your first request](@/docs/getting-started/first-request.md)
* You want to expose an OCPI API → [Server](@/docs/layers/server.md)
* You are building a hub → [Hub](@/docs/layers/hub.md)
* You want to check an invoice → [Tariffs](@/docs/layers/tariffs.md)
* A partner's implementation is misbehaving → [Conformance](@/docs/layers/conformance.md)
* You just want a command line tool → [The CLI](@/docs/getting-started/cli.md)
* You want to know whether to trust any of this → [How it is verified](@/docs/reference/verification.md)
