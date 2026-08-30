//! `Number` — the OCPI decimal number type. Never `f64` in a field, never `f64` in arithmetic.

use core::fmt;
use core::str::FromStr;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::validate::{Validate, Validator, ViolationCode};

/// An OCPI `number`: an exact decimal.
///
/// > *Numbers in OCPI are formatted as JSON numbers. Unless mentioned otherwise, numbers use 4
/// > decimals and a sufficiently large amount of digits.*
///
/// # Why not `f64`
///
/// Every price, VAT percentage, energy volume and tax amount in OCPI ends up on an invoice. A
/// binary float cannot represent `0.10` and cannot add a column of cents without drift, so every
/// arithmetic operation in this crate — the whole [`tariffs`](crate::tariffs) engine included —
/// runs on [`rust_decimal::Decimal`]. `f32`/`f64` are denied by lint in the modules where money
/// lives, and no public field of any OCPI object in this crate is a float.
///
/// # The JSON boundary
///
/// The spec requires these values to be JSON *numbers*, not strings. `serde_json` represents a
/// fractional JSON number as an `f64` unless its `arbitrary_precision` feature is enabled — a
/// feature that changes `serde_json::Value` globally for every crate in the build, so
/// `ocpi-kit` does not impose it. The boundary therefore behaves as follows:
///
/// * Integral values pass through exactly, as JSON integers.
/// * Fractional values with at most 15 significant decimal digits — which covers OCPI's entire
///   domain of prices, energies and percentages with room to spare — pass through exactly,
///   because the shortest decimal that round-trips an `f64` *is* the original decimal.
/// * Beyond that, a round-trip rounds to the nearest `f64`. [`Number::json_round_trips`] says
///   whether a given value is affected and [`Validate::validate`] reports it as
///   [`ViolationCode::Imprecise`], so this can never happen silently.
///
/// A peer that sends a number as a JSON *string* (`"0.25"`) is tolerated on input and parsed
/// exactly; output is always a JSON number.
///
/// ```
/// use ocpi_kit::types::Number;
///
/// let price: Number = "0.2500".parse().unwrap();
/// assert_eq!(serde_json::to_string(&price).unwrap(), "0.25");
/// let vat: Number = serde_json::from_str("20").unwrap();
/// assert_eq!(serde_json::to_string(&vat).unwrap(), "20");
/// ```
///
/// Spec: 2.3.0 §types_number_type
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Number(Decimal);

impl Number {
    /// Zero.
    pub const ZERO: Self = Self(Decimal::ZERO);
    /// One.
    pub const ONE: Self = Self(Decimal::ONE);

    /// Wraps a [`Decimal`].
    #[must_use]
    pub const fn new(value: Decimal) -> Self {
        Self(value)
    }

    /// The underlying [`Decimal`].
    #[must_use]
    pub const fn get(self) -> Decimal {
        self.0
    }

    /// The number of digits after the decimal point.
    #[must_use]
    pub fn scale(self) -> u32 {
        self.0.scale()
    }

    /// Rounds to `dp` decimal places, half away from zero.
    #[must_use]
    pub fn round_dp(self, dp: u32) -> Self {
        Self(self.0.round_dp_with_strategy(dp, rust_decimal::RoundingStrategy::MidpointAwayFromZero))
    }

    /// Whether this value survives a JSON round-trip unchanged.
    ///
    /// See [the type documentation](Self#the-json-boundary). `false` only for values with more
    /// significant digits than an `f64` can carry.
    #[must_use]
    pub fn json_round_trips(self) -> bool {
        if self.0.is_integer() && self.0.to_i64().is_some() {
            return true;
        }
        self.0.to_f64().and_then(decimal_from_f64).is_some_and(|d| d == self.0.normalize())
    }

    /// Whether the value is zero.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0.is_zero()
    }

    /// Whether the value is strictly negative.
    #[must_use]
    pub fn is_negative(self) -> bool {
        self.0.is_sign_negative() && !self.0.is_zero()
    }
}

impl Validate for Number {
    fn validate_in(&self, v: &mut Validator) {
        if !self.json_round_trips() {
            v.report(
                ViolationCode::Imprecise,
                format!("{} carries more significant digits than a JSON number round-trip preserves", self.0),
            );
        }
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Number({})", self.0)
    }
}

impl From<Decimal> for Number {
    fn from(value: Decimal) -> Self {
        Self(value)
    }
}
impl From<Number> for Decimal {
    fn from(value: Number) -> Self {
        value.0
    }
}

macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Number {
            fn from(value: $t) -> Self { Self(Decimal::from(value)) }
        }
    )*};
}
from_int!(i8, i16, i32, i64, u8, u16, u32, u64);

impl core::ops::Add for Number {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}
impl core::ops::Sub for Number {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}
impl core::ops::Mul for Number {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self(self.0 * rhs.0)
    }
}
impl core::ops::Div for Number {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        Self(self.0 / rhs.0)
    }
}
impl core::ops::Neg for Number {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}
impl core::iter::Sum for Number {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, core::ops::Add::add)
    }
}

/// The exact decimal that an `f64` came from.
///
/// `serde_json` hands a fractional JSON number over as an `f64`, so this is the one place where
/// a float touches a value that will end up on an invoice. Getting it back to a decimal has to
/// be exact.
///
/// The obvious route, `Decimal::try_from(f64)`, is **not** exact: it is wrong for roughly one in
/// two thousand four-decimal values in OCPI's ordinary range, turning `4106.9638` into
/// `4106.963800000001`. Rust's `{}` for `f64` instead prints the *shortest decimal string that
/// round-trips the value*, which for anything serde_json could have parsed from at most 15
/// significant digits is exactly the decimal the peer wrote. Parsing that string exactly is
/// therefore both correct and total.
///
/// Returns `None` for a value no `Decimal` can hold — infinities, NaN, and magnitudes beyond
/// 96 bits — none of which are OCPI numbers.
fn decimal_from_f64(value: f64) -> Option<Decimal> {
    if !value.is_finite() {
        return None;
    }
    // `f64::to_string` never uses exponent notation, so this is always a plain decimal literal.
    Decimal::from_str_exact(&value.to_string()).ok()
}

/// Why a decimal could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidNumber(String);

impl fmt::Display for InvalidNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid OCPI number: {}", self.0)
    }
}
impl std::error::Error for InvalidNumber {}

impl FromStr for Number {
    type Err = InvalidNumber;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::from_str_exact(s).map(Self).map_err(|e| InvalidNumber(format!("{s:?}: {e}")))
    }
}

impl Serialize for Number {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        if self.0.is_integer() {
            if let Some(i) = self.0.to_i64() {
                return serializer.serialize_i64(i);
            }
            if let Some(u) = self.0.to_u64() {
                return serializer.serialize_u64(u);
            }
        }
        let f = self
            .0
            .to_f64()
            .ok_or_else(|| S::Error::custom(format!("{} is not representable as a JSON number", self.0)))?;
        serializer.serialize_f64(f)
    }
}

impl<'de> Deserialize<'de> for Number {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = Number;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON number")
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Number, E> {
                Ok(Number(Decimal::from(v)))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Number, E> {
                Ok(Number(Decimal::from(v)))
            }
            fn visit_i128<E: serde::de::Error>(self, v: i128) -> Result<Number, E> {
                Ok(Number(Decimal::from(v)))
            }
            fn visit_u128<E: serde::de::Error>(self, v: u128) -> Result<Number, E> {
                Ok(Number(Decimal::from(v)))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Number, E> {
                decimal_from_f64(v)
                    .map(Number)
                    .ok_or_else(|| E::custom(format!("{v} is not representable as an OCPI number")))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Number, E> {
                // Tolerated: some peers quote their numbers. Parsed exactly, emitted unquoted.
                Number::from_str(v).map_err(E::custom)
            }
        }
        deserializer.deserialize_any(V)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Number {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Number".into()
    }
    fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "number",
            "description": "OCPI number: an exact decimal, serialised as a JSON number",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Number {
        s.parse().unwrap()
    }

    #[test]
    fn integers_stay_integers_on_the_wire() {
        assert_eq!(serde_json::to_string(&n("20")).unwrap(), "20");
        assert_eq!(serde_json::to_string(&n("0")).unwrap(), "0");
        assert_eq!(serde_json::to_string(&n("-7")).unwrap(), "-7");
    }

    #[test]
    fn realistic_ocpi_values_round_trip_exactly() {
        for text in ["0.25", "0.0295", "2.5", "20.45", "0.0002", "123456789.1234", "-1.05"] {
            let parsed: Number = serde_json::from_str(text).unwrap();
            assert_eq!(serde_json::to_string(&parsed).unwrap(), text, "round-trip of {text}");
            assert!(parsed.json_round_trips());
            assert!(parsed.validate().is_ok());
        }
    }

    #[test]
    fn trailing_zeros_are_normalised_away() {
        // 0.2500 and 0.25 are the same number; JSON has no way to distinguish them.
        let parsed: Number = "0.2500".parse().unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), "0.25");
        assert_eq!(parsed, n("0.25"));
    }

    #[test]
    fn a_fractional_number_decodes_to_exactly_what_the_peer_wrote() {
        // `Decimal::try_from(f64)` renders these as `…000000001`. They are ordinary prices, and
        // getting them wrong would put a spurious digit on an invoice — and, because
        // `json_round_trips` used the same conversion, would also report them as imprecise.
        for text in ["4106.9638", "4112.654", "4130.8379", "4136.529", "4163.9629", "4291.154"] {
            let parsed: Number = serde_json::from_str(text).unwrap();
            assert_eq!(parsed, n(text), "decoding {text}");
            assert_eq!(serde_json::to_string(&parsed).unwrap(), text, "re-encoding {text}");
            assert!(parsed.json_round_trips(), "{text} does survive a round-trip");
            assert!(parsed.validate().is_ok(), "{text} is a perfectly ordinary number");
        }
    }

    #[test]
    fn every_four_decimal_value_in_ocpi_range_survives_the_boundary() {
        // A sweep rather than an example, because the failures are sparse: about one in two
        // thousand. Anything that regresses this conversion will trip here.
        let mut mantissa = 1i64;
        while mantissa < 100_000_000 {
            let value = Number::new(Decimal::new(mantissa, 4));
            let json = serde_json::to_string(&value).unwrap();
            let back: Number = serde_json::from_str(&json).unwrap();
            assert_eq!(back, value, "{value} round-tripped through {json} as {back}");
            assert!(value.json_round_trips(), "{value}");
            mantissa += 1237;
        }
    }

    #[test]
    fn a_value_that_is_not_a_number_is_refused() {
        assert!(decimal_from_f64(f64::NAN).is_none());
        assert!(decimal_from_f64(f64::INFINITY).is_none());
        assert!(decimal_from_f64(f64::MAX).is_none(), "beyond what a Decimal can hold");
        assert_eq!(decimal_from_f64(0.0), Some(Decimal::ZERO));
    }

    #[test]
    fn excess_precision_is_flagged_rather_than_hidden() {
        let precise = n("0.123456789012345678901234");
        assert!(!precise.json_round_trips());
        assert_eq!(precise.validate().unwrap_err().as_slice()[0].code, ViolationCode::Imprecise);
    }

    #[test]
    fn quoted_numbers_are_tolerated_on_input() {
        let parsed: Number = serde_json::from_str("\"0.25\"").unwrap();
        assert_eq!(parsed, n("0.25"));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), "0.25");
    }

    #[test]
    fn arithmetic_is_exact() {
        let sum: Number = ["0.1", "0.2"].into_iter().map(n).sum();
        assert_eq!(sum, n("0.3"), "0.1 + 0.2 is exactly 0.3 in decimal");
    }
}
