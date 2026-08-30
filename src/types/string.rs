//! `OcpiString` — the OCPI case-sensitive, printable-UTF-8 string type.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::text::{InvalidString, StringKind, check_printable_utf8};
use super::validate::{Validate, Validator, ViolationCode};

/// A case-sensitive OCPI string of at most `N` characters.
///
/// > *Case Sensitive String. Only printable UTF-8 allowed. (Non-printable characters like:
/// > Carriage returns, Tabs, Line breaks, etc are not allowed)*
///
/// `N` is the maximum length from the spec's property table, so `OcpiString<255>` is the spec's
/// `string(255)`.
///
/// # Characters, not bytes
///
/// The specification writes `string(N)` without saying whether `N` counts bytes or characters.
/// This crate counts **Unicode scalar values** (`char`s), because the limits are clearly meant
/// to bound what a human sees — `string(45)` for a street address is a display constraint, and
/// counting bytes would silently halve the usable length of a Greek or Japanese address while
/// leaving an English one untouched. [`OcpiString::len_bytes`] is available where the byte
/// length matters, and [`OcpiString::is_conformant_in_bytes`] answers the stricter reading for
/// peers known to enforce it.
///
/// # Strict and lenient construction
///
/// [`OcpiString::new`] and [`FromStr`] are strict; the infallible `From` conversions are
/// lenient, like `Deserialize`. See [`CiString`](super::CiString#strict-and-lenient-construction).
///
/// Spec: 2.3.0 §types_string_type
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OcpiString<const N: usize>(String);

impl<const N: usize> OcpiString<N> {
    /// The maximum length the spec allows for this string, in characters.
    pub const MAX_LEN: usize = N;

    /// Creates an `OcpiString`, enforcing the character set and the length limit.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidString`] if `value` contains a control character or is longer than `N`
    /// Unicode scalar values.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidString> {
        let value = value.into();
        check_printable_utf8(&value, StringKind::Utf8)?;
        let len = value.chars().count();
        if len > N {
            return Err(InvalidString::too_long(len, N, StringKind::Utf8));
        }
        Ok(Self(value))
    }

    /// Creates an `OcpiString` without enforcing anything.
    ///
    /// This is what `Deserialize` uses; see [`types::validate`](super::validate).
    pub fn new_lenient(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes this string and yields the inner [`String`].
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    /// The length in Unicode scalar values, which is what `N` bounds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.chars().count()
    }

    /// The length in UTF-8 bytes.
    #[must_use]
    pub fn len_bytes(&self) -> usize {
        self.0.len()
    }

    /// Whether the string is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether this value satisfies the spec constraints that [`OcpiString::new`] enforces.
    #[must_use]
    pub fn is_conformant(&self) -> bool {
        self.len() <= N && check_printable_utf8(&self.0, StringKind::Utf8).is_ok()
    }

    /// Whether this value also fits `N` *bytes*, the stricter reading of `string(N)`.
    ///
    /// Use this when talking to a peer known to count bytes; ASCII-only values satisfy both
    /// readings at once.
    #[must_use]
    pub fn is_conformant_in_bytes(&self) -> bool {
        self.0.len() <= N && check_printable_utf8(&self.0, StringKind::Utf8).is_ok()
    }

    /// Re-types this string to a different maximum length.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidString`] if the value does not fit in `M` characters.
    pub fn resize<const M: usize>(self) -> Result<OcpiString<M>, InvalidString> {
        OcpiString::<M>::new(self.0)
    }

    /// The `#NA` sentinel the spec allows where a required string cannot be filled.
    ///
    /// Spec: 2.3.0 §transport_and_format_not_available
    pub const NOT_AVAILABLE: &'static str = "#NA";

    /// Whether this value is the `#NA` sentinel.
    #[must_use]
    pub fn is_not_available(&self) -> bool {
        self.0 == Self::NOT_AVAILABLE
    }
}

impl<const N: usize> Validate for OcpiString<N> {
    fn validate_in(&self, v: &mut Validator) {
        if let Err(e) = check_printable_utf8(&self.0, StringKind::Utf8) {
            v.report(ViolationCode::IllegalCharacter, e.to_string());
        }
        let len = self.len();
        if len > N {
            v.report(ViolationCode::TooLong, format!("string({N}) holds {len} characters"));
        }
    }
}

impl<const N: usize> fmt::Display for OcpiString<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const N: usize> fmt::Debug for OcpiString<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl<const N: usize> AsRef<str> for OcpiString<N> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> core::ops::Deref for OcpiString<N> {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> FromStr for OcpiString<N> {
    type Err = InvalidString;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

// The infallible conversions are **lenient**, matching `Deserialize`; see [`CiString`].
impl<const N: usize> From<&str> for OcpiString<N> {
    fn from(s: &str) -> Self {
        Self::new_lenient(s)
    }
}

impl<const N: usize> From<String> for OcpiString<N> {
    fn from(s: String) -> Self {
        Self::new_lenient(s)
    }
}

impl<const N: usize> Serialize for OcpiString<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, const N: usize> Deserialize<'de> for OcpiString<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

#[cfg(feature = "schema")]
impl<const N: usize> schemars::JsonSchema for OcpiString<N> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("String{N}").into()
    }
    fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "maxLength": N,
            "description": "OCPI string: case-sensitive, printable UTF-8 only",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_characters_not_bytes() {
        // 5 characters, 10 bytes.
        let s = OcpiString::<5>::new("日本語です").unwrap();
        assert_eq!(s.len(), 5);
        assert_eq!(s.len_bytes(), 15);
        assert!(s.is_conformant());
        assert!(!s.is_conformant_in_bytes(), "the byte reading is stricter");
        assert!(OcpiString::<4>::new("日本語です").is_err());
    }

    #[test]
    fn accepts_utf8_but_rejects_control_characters() {
        assert!(OcpiString::<64>::new("Straße 12 — Küche 🚗").is_ok());
        assert!(OcpiString::<64>::new("a\rb").is_err());
    }

    #[test]
    fn deserialize_is_permissive() {
        let s: OcpiString<2> = serde_json::from_str("\"much too long\"").unwrap();
        assert_eq!(s.as_str(), "much too long");
        assert_eq!(s.validate().unwrap_err().as_slice()[0].code, ViolationCode::TooLong);
    }
}
