//! `CiString` — the OCPI case-insensitive, printable-ASCII string type.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::text::{InvalidString, StringKind, check_printable_ascii};
use super::validate::{Validate, Validator, ViolationCode};

/// A case-insensitive OCPI string of at most `N` characters.
///
/// > *Case Insensitive String. Only printable ASCII allowed. (Non-printable characters like:
/// > Carriage returns, Tabs, Line breaks, etc are not allowed)*
///
/// `N` is the maximum length from the spec's property table, so `CiString<36>` is the spec's
/// `CiString(36)`. Because the character set is printable ASCII, "characters" and "bytes" are
/// the same thing here and the limit is unambiguous — unlike [`OcpiString`](super::OcpiString).
///
/// # Case insensitivity
///
/// [`PartialEq`], [`Eq`], [`Hash`] and [`Ord`] are **case-insensitive**, so a `CiString` used as
/// a map key behaves the way the spec says identifiers compare. [`Display`](fmt::Display) and
/// [`AsRef<str>`] preserve the original case, so re-serialising an object never rewrites a
/// peer's identifiers.
///
/// ```
/// use ocpi_kit::types::CiString;
///
/// let a: CiString<36> = "NL*TNM*001".parse().unwrap();
/// let b: CiString<36> = "nl*tnm*001".parse().unwrap();
/// assert_eq!(a, b);
/// assert_eq!(a.as_str(), "NL*TNM*001"); // original case is preserved
/// ```
///
/// # Strict and lenient construction
///
/// [`CiString::new`] and [`FromStr`] reject anything the spec forbids:
///
/// ```
/// # use ocpi_kit::types::CiString;
/// assert!("far too long for three".parse::<CiString<3>>().is_err());
/// ```
///
/// The infallible `From<&str>`/`From<String>` conversions — which is what a builder setter uses
/// — are **lenient**, exactly like `Deserialize`: they accept what they are given and leave the
/// complaint to [`Validate::validate`]. That is what keeps a peer's over-long identifier from
/// making a whole page of Locations undecodable, and it is why the
/// [`client`](crate::client) validates outgoing objects before sending them. See
/// [`types::validate`](super::validate) for the full reasoning.
///
/// ```
/// # use ocpi_kit::types::{CiString, Validate};
/// let lenient: CiString<3> = "far too long for three".into();
/// assert!(lenient.validate().is_err());
/// ```
///
/// Spec: 2.3.0 §types_cistring_type
#[derive(Clone, Default)]
pub struct CiString<const N: usize>(String);

impl<const N: usize> CiString<N> {
    /// The maximum length the spec allows for this string, in characters.
    pub const MAX_LEN: usize = N;

    /// Creates a `CiString`, enforcing the character set and the length limit.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidString`] if `value` contains a character outside printable ASCII
    /// (U+0020..=U+007E) or is longer than `N` characters.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidString> {
        let value = value.into();
        check_printable_ascii(&value, StringKind::Ci)?;
        if value.len() > N {
            return Err(InvalidString::too_long(value.len(), N, StringKind::Ci));
        }
        Ok(Self(value))
    }

    /// Creates a `CiString` without enforcing anything.
    ///
    /// This is what `Deserialize` uses. Prefer [`CiString::new`] for values this process
    /// originates; use [`Validate::validate`] to find out afterwards whether a received value
    /// is conformant.
    pub fn new_lenient(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The string with its original case preserved.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes this string and yields the inner [`String`], original case preserved.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    /// The length in characters, which for printable ASCII equals the length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the string is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether this value satisfies the spec constraints that [`CiString::new`] enforces.
    #[must_use]
    pub fn is_conformant(&self) -> bool {
        self.0.len() <= N && check_printable_ascii(&self.0, StringKind::Ci).is_ok()
    }

    /// Re-types this string to a different maximum length.
    ///
    /// Used where the spec reuses one identifier under two limits, for example a
    /// `CiString(36)` `CDR.id` widened to the `CiString(39)` of a credit CDR.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidString`] if the value does not fit in `M` characters.
    pub fn resize<const M: usize>(self) -> Result<CiString<M>, InvalidString> {
        CiString::<M>::new(self.0)
    }

    /// Compares case-insensitively against a plain string.
    #[must_use]
    pub fn eq_ignore_case(&self, other: &str) -> bool {
        self.0.eq_ignore_ascii_case(other)
    }

    /// The `#NA` sentinel the spec allows where a required string cannot be filled.
    ///
    /// Spec: 2.3.0 §transport_and_format_not_available
    pub const NOT_AVAILABLE: &'static str = "#NA";

    /// Whether this value is the `#NA` sentinel.
    ///
    /// Spec: 2.3.0 §transport_and_format_not_available
    #[must_use]
    pub fn is_not_available(&self) -> bool {
        self.0.eq_ignore_ascii_case(Self::NOT_AVAILABLE)
    }
}

impl<const N: usize> Validate for CiString<N> {
    fn validate_in(&self, v: &mut Validator) {
        if let Err(e) = check_printable_ascii(&self.0, StringKind::Ci) {
            v.report(ViolationCode::IllegalCharacter, e.to_string());
        }
        if self.0.len() > N {
            // Counted in characters rather than bytes for the message: the limit itself is in
            // bytes because the character set is printable ASCII, but a *lenient* value from the
            // wire may not be, and "holds 41 characters" is what a partner can check against the
            // value they sent. The charset violation above says the rest.
            v.report(
                ViolationCode::TooLong,
                format!("CiString({N}) holds {} characters", self.0.chars().count()),
            );
        }
    }
}

impl<const N: usize> fmt::Display for CiString<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const N: usize> fmt::Debug for CiString<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl<const N: usize> PartialEq for CiString<N> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl<const N: usize> Eq for CiString<N> {}

impl<const N: usize> PartialOrd for CiString<N> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<const N: usize> Ord for CiString<N> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        let a = self.0.bytes().map(|b| b.to_ascii_lowercase());
        let b = other.0.bytes().map(|b| b.to_ascii_lowercase());
        a.cmp(b)
    }
}

impl<const N: usize> Hash for CiString<N> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash the case-folded form so that `a == b` implies `hash(a) == hash(b)`.
        for byte in self.0.bytes() {
            state.write_u8(byte.to_ascii_lowercase());
        }
        state.write_u8(0xff);
    }
}

impl<const N: usize> AsRef<str> for CiString<N> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> core::ops::Deref for CiString<N> {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> FromStr for CiString<N> {
    type Err = InvalidString;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

// The infallible conversions are **lenient**, matching `Deserialize`; see the type docs.
impl<const N: usize> From<&str> for CiString<N> {
    fn from(s: &str) -> Self {
        Self::new_lenient(s)
    }
}

impl<const N: usize> From<String> for CiString<N> {
    fn from(s: String) -> Self {
        Self::new_lenient(s)
    }
}

impl<const N: usize> Serialize for CiString<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, const N: usize> Deserialize<'de> for CiString<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

#[cfg(feature = "schema")]
impl<const N: usize> schemars::JsonSchema for CiString<N> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("CiString{N}").into()
    }
    fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "maxLength": N,
            "pattern": "^[\\u0020-\\u007E]*$",
            "description": "OCPI CiString: case-insensitive, printable ASCII only",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn rejects_non_printable_and_non_ascii() {
        assert!(CiString::<36>::new("ok").is_ok());
        assert!(CiString::<36>::new("no\nnewline").is_err());
        assert!(CiString::<36>::new("no\ttab").is_err());
        assert!(CiString::<36>::new("caf\u{e9}").is_err(), "CiString is ASCII-only");
        assert!(CiString::<3>::new("abcd").is_err());
    }

    #[test]
    fn equality_and_hashing_ignore_case() {
        let a = CiString::<36>::new("NL*TNM*001").unwrap();
        let b = CiString::<36>::new("nl*tnm*001").unwrap();
        assert_eq!(a, b);
        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b), "case-folded hash must agree with case-folded Eq");
    }

    #[test]
    fn deserialize_is_permissive_but_validate_complains() {
        let long: CiString<3> = serde_json::from_str("\"abcdef\"").unwrap();
        assert_eq!(long.as_str(), "abcdef", "peer data is never dropped");
        let err = long.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].code, ViolationCode::TooLong);
        assert!(!long.is_conformant());
    }

    #[test]
    fn serialize_preserves_original_case() {
        let a = CiString::<36>::new("MiXeD").unwrap();
        assert_eq!(serde_json::to_string(&a).unwrap(), "\"MiXeD\"");
    }

    #[test]
    fn na_sentinel_is_recognised() {
        assert!(CiString::<36>::new("#NA").unwrap().is_not_available());
        assert!(!CiString::<36>::new("NA").unwrap().is_not_available());
    }
}
