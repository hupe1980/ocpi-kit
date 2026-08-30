```rust
let peer = Registration::new(versions_url, token_a)
    .discover(client.transport()).await?
    .select_best(client.transport()).await?;

// Refuse a peer that does not implement what you need — before anything is sent.
peer.require(&[(ModuleId::Locations, InterfaceRole::Sender)])?;

let peer = peer.register(client.transport(), &my_credentials).await?;

// Then pull, following every `Link: rel="next"`.
let mut locations = peer.locations(client.transport(), me).list(PageQuery::new())?;
while let Some(location) = locations.next().await? {
    println!("{} {}", location.id, location.name.as_deref().unwrap_or(""));
}
```
