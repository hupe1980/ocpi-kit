//! A toolkit for the [OCPI](https://evroaming.org/ocpi/) (Open Charge Point Interface) protocol
//! used for EV roaming between Charge Point Operators, e-Mobility Service Providers and roaming
//! hubs.
//!
//! # What is here
//!
//! | Layer | Feature | What it gives you |
//! |---|---|---|
//! | [`types`] | *(always)* | `CiString`, `DateTime`, `Number`, `Url`, `Extensions`, validation |
//! | [`v2_3_0`] | `v2_3_0` | the OCPI 2.3.0 wire model, all ten modules |
//! | [`v2_2_1`] | `v2_2_1` | the OCPI 2.2.1 wire model |
//! | [`v2_1_1`] | `v2_1_1` | the OCPI 2.1.1 wire model |
//! | [`convert`] | `convert` | `Upgrade`/`Downgrade` between versions, with loss accounting |
//! | [`transport`] | `transport` | envelope, headers, credentials tokens, pagination, routing, PATCH |
//! | [`client`] | `client` | an async client over `reqwest`, with the registration handshake |
//! | [`server`] | `server` | an `axum` router driven by one trait per module and interface |
//! | [`hub`] | `hub` | routing, broadcast push, open routing, GET All, version bridging |
//! | [`tariffs`] | `tariffs` | an auditable pricing engine over CDRs and Sessions |
//! | [`testkit`] | `testkit` | sample objects, in-memory stores and a conformant mock peer |
//!
//! # Four properties worth knowing about
//!
//! **Money is never a float.** Every `number` in every object is a [`types::Number`], an exact
//! decimal. No public field of any OCPI object in this crate is an `f32` or `f64`, and the
//! modules where money is computed deny floats by lint.
//!
//! **Nothing a peer sent is thrown away.** Undocumented JSON fields land in
//! [`types::Extensions`] and are written back verbatim; an open-enum value this crate does not
//! know keeps its text in a `Custom` variant. A hub built on `ocpi-kit` forwards a vendor
//! extension it has never seen without damaging it, which is what OCPI 2.3.0's extensibility
//! chapter asks for.
//!
//! **Parsing and conformance are separate questions.** A peer that overruns a `string(45)`
//! cannot make a whole page of Locations undecodable; the value arrives, and
//! [`types::Validate::validate`] reports it with a JSON Pointer. See [`types::validate`].
//!
//! **The peer's OCPI version is not your problem.** [`client`], [`server`] and [`hub`] speak the
//! canonical [`v2_3_0`] model and translate at the wire, so a 2.2.1 peer — most of the market —
//! reads and writes as 2.3.0 objects, and anything that cannot cross is reported rather than
//! dropped. See [`convert`].
//!
//! # Getting started
//!
// The example needs the 2.3.0 model, which is a default feature but can be switched off; the
// fence becomes `ignore` rather than the example disappearing, so the docs read the same either
// way.
#![cfg_attr(feature = "v2_3_0", doc = "```rust")]
#![cfg_attr(not(feature = "v2_3_0"), doc = "```rust,ignore")]
//! use ocpi_kit::types::Validate;
//! use ocpi_kit::v2_3_0::locations::Location;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let json = std::fs::read_to_string("fixtures/2.3.0/location_example.json")?;
//! let location: Location = serde_json::from_str(&json)?;
//!
//! assert_eq!(location.country_code.as_str(), "BE");
//! location.validate()?; // every length limit and cross-field rule of the spec
//! # Ok(())
//! # }
//! ```
//!
//! # Spec traceability
//!
//! Every public item carries a `Spec: <version> §<anchor>` line naming the AsciiDoc anchor in
//! the OCPI source it implements, so a reviewer — or a partner's compliance team — can go from
//! a Rust type straight to the sentence that defines it.
//!
//! # Further reading
//!
//! The [guide](https://hupe1980.github.io/ocpi-kit/docs/) covers the concepts behind these APIs —
//! the parse/validate/construct rule, open enums, extensions, version bridging — plus per-layer
//! walkthroughs, the interop quirks registry and the specification errata.
//!
//! OCPI is a protocol owned and maintained by the [EVRoaming Foundation](https://evroaming.org/).
//! This project is not affiliated with the EVRoaming Foundation.

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod types;

#[cfg(feature = "v2_3_0")]
#[cfg_attr(docsrs, doc(cfg(feature = "v2_3_0")))]
pub mod v2_3_0;

#[cfg(feature = "v2_2_1")]
#[cfg_attr(docsrs, doc(cfg(feature = "v2_2_1")))]
pub mod v2_2_1;

#[cfg(feature = "v2_1_1")]
#[cfg_attr(docsrs, doc(cfg(feature = "v2_1_1")))]
pub mod v2_1_1;

#[cfg(feature = "convert")]
#[cfg_attr(docsrs, doc(cfg(feature = "convert")))]
pub mod convert;

#[cfg(feature = "transport")]
#[cfg_attr(docsrs, doc(cfg(feature = "transport")))]
pub mod transport;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

#[cfg(feature = "server")]
#[cfg_attr(docsrs, doc(cfg(feature = "server")))]
pub mod server;

#[cfg(feature = "hub")]
#[cfg_attr(docsrs, doc(cfg(feature = "hub")))]
pub mod hub;

#[cfg(feature = "tariffs")]
#[cfg_attr(docsrs, doc(cfg(feature = "tariffs")))]
pub mod tariffs;

#[cfg(feature = "testkit")]
#[cfg_attr(docsrs, doc(cfg(feature = "testkit")))]
pub mod testkit;

mod version;

pub use version::{InterfaceRole, ModuleId, VersionNumber};

/// The version of OCPI this crate treats as canonical: every other version is described as a
/// delta from it.
#[cfg(feature = "v2_3_0")]
pub const CANONICAL_VERSION: VersionNumber = VersionNumber::V2_3_0;
