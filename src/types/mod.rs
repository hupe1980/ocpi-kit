//! The scalar and leaf types the whole specification is built from.
//!
//! These are version-neutral: OCPI 2.1.1, 2.2.1 and 2.3.0 agree on what a `CiString`, a
//! `DateTime`, a `number` and a `DisplayText` are. Types that *changed* between versions —
//! `Price`, `Role`, `Tariff` — live in the per-version modules instead.
//!
//! Nothing here needs an async runtime or an HTTP stack, so this layer compiles for
//! `wasm32-unknown-unknown` and can back browser tooling and edge workers.
//!
//! # The three rules
//!
//! 1. **Parse permissively.** `Deserialize` never fails because a peer overran a length limit.
//! 2. **Validate explicitly.** [`Validate::validate`] reports every deviation, with a JSON
//!    Pointer to it.
//! 3. **Construct strictly.** `new` and `FromStr` refuse to build a non-conformant value, so
//!    what this crate emits is conformant.
//!
//! See [`validate`] for why.

pub mod cistring;
pub mod datetime;
pub mod display_text;
pub mod extensions;
pub mod ids;
pub mod local;
pub mod number;
pub mod open_enum;
pub mod string;
pub mod url;
pub mod validate;

mod text;

pub use cistring::CiString;
pub use datetime::{DateTime, InvalidDateTime};
pub use display_text::DisplayText;
pub use extensions::Extensions;
pub use ids::{
    ContractId, ContractIdParts, CountryCode, CountryCodeExt, Currency, EvseId, EvseIdParts, InvalidPartyRef,
    PartyId, PartyRef,
};
pub use local::{InvalidLocalDate, InvalidLocalTime, LocalDate, LocalParts, LocalTime};
pub use number::{InvalidNumber, Number};
pub use open_enum::UnknownVariant;
pub use string::OcpiString;
pub use text::{InvalidString, StringKind};
pub use url::{
    InvalidUrl, URL_MAX_LEN, Url, UrlPolicy, UrlRefused, encode_path_segment, encode_query_component,
};
pub use validate::{Validate, Validator, Violation, ViolationCode, Violations};

#[doc(hidden)]
pub use open_enum::{validate_closed_enum_value, validate_open_enum_value};

pub(crate) use validate::validate_fields;

/// The length limit for a `string` whose property table gives no maximum.
///
/// A handful of OCPI properties are typed `string` with no `(N)`: `TaxAmount.name` and
/// `TaxAmount.account_number`, for example. [`OcpiText`] is the type for those: it enforces the
/// character set and nothing else.
pub const UNBOUNDED: usize = usize::MAX;

/// A `string` with no length limit in the specification.
///
/// See [`UNBOUNDED`].
pub type OcpiText = OcpiString<UNBOUNDED>;

/// A `CiString` with no length limit in the specification.
///
/// Used by `Parking.apds_reference`, the one `CiString` in OCPI 2.3.0 written without a maximum.
///
/// See [`UNBOUNDED`].
pub type CiText = CiString<UNBOUNDED>;
