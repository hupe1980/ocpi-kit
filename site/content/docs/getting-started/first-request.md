+++
title = "Your first request"
weight = 20
description = "Decode and validate an OCPI document, register with a peer, and serve an OCPI API."
+++

## Decode and validate a file

Nothing here needs a network or an async runtime — the default features are enough.

```rust
use ocpi_kit::types::Validate;
use ocpi_kit::v2_3_0::locations::Location;

let json = std::fs::read_to_string("location.json")?;
let location: Location = serde_json::from_str(&json)?;

assert_eq!(location.country_code.as_str(), "BE");

// Decoding said the shape is right. Validation says whether it is conformant.
match location.validate() {
    Ok(()) => println!("conformant"),
    Err(violations) => {
        for v in violations.iter() {
            println!("{} {:?}: {}", v.pointer, v.code, v.message);
        }
    }
}
```

The two questions are deliberately separate; see
[Parse, validate, construct](@/docs/concepts/parse-validate-construct.md).

## Register with a peer and pull

With the `client` feature:

```rust
use ocpi_kit::client::{OcpiClient, Registration};
use ocpi_kit::transport::{CredentialsToken, PageQuery};
use ocpi_kit::types::{PartyRef, Url};
use ocpi_kit::{InterfaceRole, ModuleId};

let client = OcpiClient::new()?;
let me = PartyRef::new("NL", "TNM")?;

let peer = Registration::new(
        Url::new("https://cpo.example.com/ocpi/versions")?,
        CredentialsToken::new("token-a-received-out-of-band")?,
    )
    .discover(client.transport()).await?     // GET /versions
    .select_best(client.transport()).await?; // GET the details of the newest common version

// Check before you commit: refuse a peer that does not implement what you need.
peer.require(&[(ModuleId::Locations, InterfaceRole::Sender)])?;

let peer = peer.register(client.transport(), &my_credentials).await?; // POST /credentials

let mut locations = peer.locations(client.transport(), me).list(PageQuery::new())?;
while let Some(location) = locations.next().await? {
    println!("{} {}", location.id, location.name.as_deref().unwrap_or(""));
}
```

`Registration` → `Discovered` → `Selected` → `Peer` is a typestate: the compiler will not let you
call a module endpoint before discovery, or keep using `CREDENTIALS_TOKEN_A` after registration.
See [Client](@/docs/layers/client.md).

## Serve an OCPI API

With the `server` feature you implement one trait per module and interface, and mount it:

```rust
use ocpi_kit::server::{InMemoryTokenStore, OcpiRouter};
use ocpi_kit::{types::Url, VersionNumber};

let app = OcpiRouter::new(
        VersionNumber::V2_3_0,
        Url::new("https://cpo.example.com/ocpi/cpo/2.3.0")?,
        std::sync::Arc::new(InMemoryTokenStore::new()),
    )
    .credentials(my_credentials_handler)
    .locations_sender(my_locations)
    .build();

axum::serve(tokio::net::TcpListener::bind("0.0.0.0:8080").await?, app).await?;
```

`/versions` and the version-details endpoint are generated from exactly what was mounted, so
discovery cannot disagree with reality. See [Server](@/docs/layers/server.md).
