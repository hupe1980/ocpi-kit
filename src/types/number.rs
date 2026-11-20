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
/// The spec requires JSON *numbers*. How exact the boundary is depends on `serde_json`'s
/// build-wide `arbitrary_precision` feature, which the `exact-json` feature enables and any other
/// crate can too; [`Number::exact_json_active`] says which mode is in effect. Both work.
///
/// * **Exact mode**: every number is read from its literal and written as one; no float.
/// * **Float mode** (default): integers and fractions up to 15 significant digits — all of
///   OCPI's practical range — pass through exactly. Longer values round to the nearest `f64`;
///   [`Number::json_round_trips`] and [`ViolationCode::Imprecise`] say so, never silently.
///
/// Both modes normalise (`0.2500` is `0.25`) and produce the same JSON for every value a float
/// can carry. Past 28 significant digits a literal is rounded; past the [`Decimal`] range it is
/// refused. A quoted number (`"0.25"`) is accepted and written back unquoted.
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
    /// See [the type documentation](Self#the-json-boundary). Always `true` in exact mode; in
    /// float mode `false` only for values with more significant digits than an `f64` can carry.
    #[must_use]
    pub fn json_round_trips(self) -> bool {
        exact_json_active() || float_round_trips(self.0)
    }

    /// Whether `serde_json`'s exact-number mode (`arbitrary_precision`) is active in this build.
    ///
    /// Enabled by this crate's `exact-json` feature or by any other crate: Cargo unifies features,
    /// so a `cfg` cannot know and this is answered at runtime, once.
    #[must_use]
    pub fn exact_json_active() -> bool {
        exact_json_active()
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

    /// Whether the value is strictly positive.
    ///
    /// The counterpart of [`is_negative`](Self::is_negative). Both exclude zero, so
    /// `is_positive`, `is_negative` and [`is_zero`](Self::is_zero) partition every value — which
    /// is what lets a sign comparison be written without a three-way `match` on an ordering.
    #[must_use]
    pub fn is_positive(self) -> bool {
        !self.0.is_sign_negative() && !self.0.is_zero()
    }

    /// The absolute value.
    #[must_use]
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }

    /// Addition that returns `None` instead of panicking on overflow.
    ///
    /// The arithmetic operators on this type are the ones from [`Decimal`], and those **panic**
    /// on overflow and on division by zero. That is the right default for a computation over
    /// values a program produced itself, and the wrong one for values that arrived in a peer's
    /// CDR: a `price` of 10^27 is not a tariff, but it is a number a peer can send, and the panic
    /// it causes lands in whatever task was pricing at the time.
    #[must_use]
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    /// Subtraction that returns `None` instead of panicking on overflow. See
    /// [`checked_add`](Self::checked_add).
    #[must_use]
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    /// Multiplication that returns `None` instead of panicking on overflow. See
    /// [`checked_add`](Self::checked_add).
    #[must_use]
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        self.0.checked_mul(rhs.0).map(Self)
    }

    /// Division that returns `None` instead of panicking on overflow **or on a zero divisor**.
    /// See [`checked_add`](Self::checked_add).
    #[must_use]
    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        self.0.checked_div(rhs.0).map(Self)
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

/// Whether `value` survives the float path of the JSON boundary unchanged.
fn float_round_trips(value: Decimal) -> bool {
    if value.is_integer() && value.to_i64().is_some() {
        return true;
    }
    value.to_f64().and_then(decimal_from_f64).is_some_and(|d| d == value.normalize())
}

/// The key under which `serde_json`'s `arbitrary_precision` mode carries a number's literal.
const SERDE_JSON_NUMBER_TOKEN: &str = "$serde_json::private::Number";

/// Whether `serde_json` keeps number literals exactly in this build. See
/// [`Number::exact_json_active`].
fn exact_json_active() -> bool {
    static ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ACTIVE.get_or_init(|| {
        // More digits than an f64 carries: only an exact representation gives them back.
        const PROBE: &str = "0.10000000000000000001";
        serde_json::from_str::<serde_json::Number>(PROBE).is_ok_and(|n| n.to_string() == PROBE)
    })
}

/// Parses a JSON number literal from the wire exactly.
///
/// Plain literals keep every digit up to the 28 a [`Decimal`] holds and are rounded beyond that —
/// never refused, so exact mode accepts everything float mode does. Exponent literals are scaled
/// exactly. A magnitude outside the decimal range is refused, quoting at most 40 characters of
/// the literal so a hostile peer cannot fill a log line.
///
/// The result is normalised (`5.00` is `5`), which is what the float path produces too: a value
/// decodes to the same `Number`, displayed the same way, whichever mode the build is in.
fn parse_literal(literal: &str) -> Result<Decimal, InvalidNumber> {
    let parsed = if literal.contains(['e', 'E']) {
        Decimal::from_scientific(literal)
    } else {
        Decimal::from_str_exact(literal).or_else(|_| Decimal::from_str(literal))
    };
    parsed.map(|d| d.normalize()).map_err(|e| {
        let shown: String = literal.chars().take(40).collect();
        let ellipsis = if literal.chars().count() > 40 { "…" } else { "" };
        InvalidNumber(format!("{shown:?}{ellipsis}: {e}"))
    })
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
        // Only a value the float path would round needs the exact form. Everything else is
        // written as a plain float, which serde_json prints identically in both modes and which
        // every other serializer (YAML, snapshots, CBOR) understands: serde_json's private
        // wrapper is a map to anyone but serde_json.
        if exact_json_active()
            && !float_round_trips(self.0)
            && let Ok(exact) = serde_json::Number::from_str(&self.0.normalize().to_string())
        {
            return exact.serialize(serializer);
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
        impl<'de> serde::de::Visitor<'de> for V {
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
                parse_literal(v).map(Number).map_err(E::custom)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Number, A::Error> {
                use serde::de::{Error as _, Unexpected};
                // serde_json's exact mode presents a number as a one-entry map holding its
                // literal. Outside that mode an object is an object, and not a number.
                if !exact_json_active() {
                    return Err(A::Error::invalid_type(Unexpected::Map, &self));
                }
                if map.next_key::<std::borrow::Cow<'de, str>>()?.as_deref() != Some(SERDE_JSON_NUMBER_TOKEN) {
                    return Err(A::Error::invalid_type(Unexpected::Map, &self));
                }
                let literal: std::borrow::Cow<'de, str> = map.next_value()?;
                parse_literal(&literal).map(Number).map_err(A::Error::custom)
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
        if exact_json_active() {
            return; // nothing is rounded in exact mode, so there is nothing to flag
        }
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
    fn exponent_literals_and_quoted_numbers_are_exact() {
        let parsed: Number = serde_json::from_str("\"2.5E2\"").unwrap();
        assert_eq!(parsed, n("250"));
        assert_eq!(parse_literal("1e-3").unwrap(), Decimal::new(1, 3));
        assert_eq!(parse_literal("-0").unwrap(), Decimal::ZERO);
    }

    #[test]
    fn a_hostile_literal_is_refused_without_panicking_or_flooding_the_log() {
        let huge = "9".repeat(10_000);
        for literal in ["1e999999999", "-1e999999999", huge.as_str(), "1e-999999999"] {
            let error = match parse_literal(literal) {
                Err(e) => e.to_string(),
                // A vanishingly small value may legitimately round to zero; it must not panic.
                Ok(value) => {
                    assert!(value.is_zero(), "{literal:.20} decoded to {value}");
                    continue;
                }
            };
            assert!(error.chars().count() < 120, "{error}");
        }
    }

    #[test]
    fn excess_digits_round_to_what_a_decimal_holds_rather_than_failing() {
        // What a peer writes when it prints an f64 in full. Float mode accepts it (via the f64),
        // so exact mode must too.
        let value = parse_literal("0.1000000000000000055511151231257827021181583404541015625").unwrap();
        assert_eq!(value.to_string(), "0.1000000000000000055511151231");
    }

    #[test]
    fn output_is_the_same_text_in_both_modes() {
        // Normalised, so fixtures and snapshots do not depend on which mode wrote them.
        for (input, written) in [("0.2500", "0.25"), ("1.50", "1.5"), ("20", "20"), ("-0.0295", "-0.0295")] {
            assert_eq!(serde_json::to_string(&n(input)).unwrap(), written);
        }
    }

    #[cfg(not(feature = "exact-json"))]
    #[test]
    fn outside_exact_mode_an_object_is_not_a_number() {
        if exact_json_active() {
            return; // another crate in this build turned the mode on
        }
        let error = serde_json::from_str::<Number>(r#"{"$serde_json::private::Number":"1"}"#).unwrap_err();
        assert!(error.to_string().contains("expected a JSON number"), "{error}");
    }

    #[cfg(feature = "exact-json")]
    mod exact {
        use super::*;

        const LONG: &str = "0.0416666666666666666666666667";

        #[test]
        fn the_mode_is_detected() {
            assert!(Number::exact_json_active());
        }

        #[test]
        fn a_fraction_decodes_directly_and_through_a_value() {
            // The reported failure: "invalid type: map, expected a JSON number".
            for text in ["0.25", LONG, "-1.05", "1e-3"] {
                let direct: Number = serde_json::from_str(text).unwrap();
                let value: serde_json::Value = serde_json::from_str(text).unwrap();
                let via_value: Number = serde_json::from_value(value).unwrap();
                assert_eq!(direct, via_value, "{text}");
            }
            assert_eq!(serde_json::from_str::<Number>(LONG).unwrap(), n(LONG));
        }

        #[test]
        fn twenty_eight_digits_round_trip_and_are_not_imprecise() {
            let value = n(LONG);
            assert_eq!(serde_json::to_string(&value).unwrap(), LONG);
            assert!(value.json_round_trips());
            assert!(value.validate().is_ok());
        }
    }

    #[test]
    fn arithmetic_is_exact() {
        let sum: Number = ["0.1", "0.2"].into_iter().map(n).sum();
        assert_eq!(sum, n("0.3"), "0.1 + 0.2 is exactly 0.3 in decimal");
    }
}
