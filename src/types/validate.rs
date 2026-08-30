//! Explicit validation: parse permissively, validate deliberately, construct strictly.
//!
//! # Why validation is a separate pass
//!
//! OCPI 2.3.0 requires that *"An OCPI Platform SHALL NOT reject request or response payloads
//! based on the presence of JSON object field names that are not documented in this
//! specification"* (`transport_and_format` — Non-specified JSON fields). In the same spirit,
//! and because a hub that drops a peer's data is worse than a hub that forwards slightly
//! non-conformant data, `ocpi-kit` never fails a deserialisation because a `string(45)` field
//! turned out to be 46 characters long.
//!
//! Instead the crate follows one rule throughout:
//!
//! > **Parse permissively, validate explicitly, construct strictly.**
//!
//! * `Deserialize` accepts what the peer sent, as long as it is well-typed JSON.
//! * [`Validate::validate`] reports *every* violation, each with a JSON Pointer
//!   ([RFC 6901](https://datatracker.ietf.org/doc/html/rfc6901)) into the object, so a
//!   conformance report can point at the exact field.
//! * The constructors ([`CiString::new`](crate::types::CiString::new),
//!   [`OcpiString::new`](crate::types::OcpiString::new), …) refuse to build a value that is
//!   already out of spec, so data *this* crate emits is conformant by construction.
//!
//! Spec: 2.3.0 §transport_and_format_json_http_implementation_guide

use core::fmt;

/// A single spec violation found by [`Validate::validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// JSON Pointer (RFC 6901) to the offending value, relative to the validated object.
    ///
    /// The empty string refers to the validated object itself.
    pub pointer: String,
    /// Machine-readable classification of the violation.
    pub code: ViolationCode,
    /// Human-readable explanation, including the spec rule that was broken.
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = if self.pointer.is_empty() { "/" } else { &self.pointer };
        write!(f, "{at}: {}", self.message)
    }
}

/// Classification of a [`Violation`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ViolationCode {
    /// A string exceeded the maximum length given in the spec's property table.
    TooLong,
    /// A string contained characters the spec forbids (non-printable, or non-ASCII in a
    /// `CiString`).
    IllegalCharacter,
    /// A list that the spec marks with cardinality `+` (one or more) was empty.
    EmptyRequiredList,
    /// A value was outside the range the spec allows.
    OutOfRange,
    /// A field was syntactically fine but violates a cross-field rule of the spec.
    Inconsistent,
    /// A conditionally required field was missing.
    MissingConditional,
    /// The value cannot survive a JSON round-trip without loss of precision.
    Imprecise,
}

impl ViolationCode {
    /// A short, stable, machine-readable slug for this code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooLong => "too_long",
            Self::IllegalCharacter => "illegal_character",
            Self::EmptyRequiredList => "empty_required_list",
            Self::OutOfRange => "out_of_range",
            Self::Inconsistent => "inconsistent",
            Self::MissingConditional => "missing_conditional",
            Self::Imprecise => "imprecise",
        }
    }
}

/// Every [`Violation`] found in one object, in document order.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Violations(Vec<Violation>);

impl Violations {
    /// The violations, in document order.
    #[must_use]
    pub fn as_slice(&self) -> &[Violation] {
        &self.0
    }

    /// Whether no violation was found.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many violations were found.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Consumes this set and yields the violations.
    #[must_use]
    pub fn into_vec(self) -> Vec<Violation> {
        self.0
    }

    /// The violations, in document order.
    pub fn iter(&self) -> core::slice::Iter<'_, Violation> {
        self.0.iter()
    }
}

impl IntoIterator for Violations {
    type Item = Violation;
    type IntoIter = std::vec::IntoIter<Violation>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Violations {
    type Item = &'a Violation;
    type IntoIter = core::slice::Iter<'a, Violation>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl fmt::Display for Violations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, v) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{v}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Violations {}

/// Accumulates violations while walking an object graph, tracking the current JSON Pointer.
#[derive(Debug, Default)]
pub struct Validator {
    path: String,
    found: Vec<Violation>,
}

impl Validator {
    /// A validator positioned at the root of an object.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a violation at the current position.
    pub fn report(&mut self, code: ViolationCode, message: impl Into<String>) {
        self.found.push(Violation { pointer: self.path.clone(), code, message: message.into() });
    }

    /// Records a violation at `field` below the current position.
    pub fn report_at(&mut self, field: &str, code: ViolationCode, message: impl Into<String>) {
        self.enter(field);
        self.report(code, message);
        self.leave();
    }

    /// Descends into a named field or array index, escaping it per RFC 6901.
    pub fn enter(&mut self, segment: &str) {
        self.path.push('/');
        for ch in segment.chars() {
            match ch {
                '~' => self.path.push_str("~0"),
                '/' => self.path.push_str("~1"),
                c => self.path.push(c),
            }
        }
    }

    /// Returns to the parent of the current position.
    ///
    /// # Panics
    ///
    /// Panics if called more often than [`Validator::enter`].
    pub fn leave(&mut self) {
        let cut = self.path.rfind('/').expect("leave() without a matching enter()");
        self.path.truncate(cut);
    }

    /// Validates `value` at `segment` below the current position.
    pub fn field(&mut self, segment: &str, value: &impl Validate) {
        self.enter(segment);
        value.validate_in(self);
        self.leave();
    }

    /// The JSON Pointer of the current position.
    #[must_use]
    pub fn pointer(&self) -> &str {
        &self.path
    }

    /// Consumes the validator and yields everything it found.
    #[must_use]
    pub fn finish(self) -> Violations {
        Violations(self.found)
    }
}

/// Checks an OCPI value against the constraints in the specification's property tables.
///
/// See the [module documentation](self) for why this is not part of `Deserialize`.
pub trait Validate {
    /// Appends this value's violations to `v`, relative to `v`'s current position.
    fn validate_in(&self, v: &mut Validator);

    /// Validates this value as the root of an object graph.
    ///
    /// # Errors
    ///
    /// Returns every violation found, in document order.
    fn validate(&self) -> Result<(), Violations> {
        let mut v = Validator::new();
        self.validate_in(&mut v);
        let found = v.finish();
        if found.is_empty() { Ok(()) } else { Err(found) }
    }
}

impl<T: Validate> Validate for Option<T> {
    fn validate_in(&self, v: &mut Validator) {
        if let Some(inner) = self {
            inner.validate_in(v);
        }
    }
}

impl<T: Validate> Validate for Vec<T> {
    fn validate_in(&self, v: &mut Validator) {
        for (i, item) in self.iter().enumerate() {
            v.enter(&i.to_string());
            item.validate_in(v);
            v.leave();
        }
    }
}

impl<T: Validate> Validate for Box<T> {
    fn validate_in(&self, v: &mut Validator) {
        T::validate_in(self, v);
    }
}

/// Implements [`Validate`] as a no-op for types that carry no spec constraints.
macro_rules! impl_validate_noop {
    ($($t:ty),* $(,)?) => {
        $(impl Validate for $t {
            fn validate_in(&self, _v: &mut Validator) {}
        })*
    };
}

impl_validate_noop!(bool, i8, i16, i32, i64, u8, u16, u32, u64, usize, String, serde_json::Value);

/// Validates each named field of `$self` in turn.
///
/// Expands to a [`Validator::field`] call per field, using the Rust field name as the JSON
/// Pointer segment, so the pointers a violation reports are pointers into the JSON the peer
/// actually sent. Where a field is `#[serde(rename)]`d — `type` is a Rust keyword, so
/// `image_type` carries it — give the wire name with `field as "type"`.
macro_rules! validate_fields {
    ($self:ident, $v:ident, $($field:ident $(as $wire:literal)?),* $(,)?) => {
        $( $v.field(validate_fields!(@wire $field $(, $wire)?), &$self.$field); )*
    };
    (@wire $field:ident) => { stringify!($field) };
    (@wire $field:ident, $wire:literal) => { $wire };
}

pub(crate) use validate_fields;

#[cfg(test)]
mod tests {
    use super::*;

    struct Leaf(bool);
    impl Validate for Leaf {
        fn validate_in(&self, v: &mut Validator) {
            if !self.0 {
                v.report(ViolationCode::OutOfRange, "leaf is false");
            }
        }
    }

    #[test]
    fn pointer_tracks_nesting_and_escapes_rfc6901() {
        let mut v = Validator::new();
        v.enter("a/b");
        v.enter("c~d");
        v.report(ViolationCode::TooLong, "boom");
        v.leave();
        v.leave();
        let found = v.finish();
        assert_eq!(found.as_slice()[0].pointer, "/a~1b/c~0d");
    }

    #[test]
    fn vec_and_option_are_walked_with_indices() {
        let value = vec![Leaf(true), Leaf(false), Leaf(false)];
        let err = value.validate().unwrap_err();
        assert_eq!(err.len(), 2);
        assert_eq!(err.as_slice()[0].pointer, "/1");
        assert_eq!(err.as_slice()[1].pointer, "/2");
        assert!(Some(Leaf(true)).validate().is_ok());
        assert!(Option::<Leaf>::None.validate().is_ok());
    }
}
