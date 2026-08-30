//! `DateTime` — the OCPI timestamp type: RFC 3339, always UTC.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

use super::validate::{Validate, Validator, ViolationCode};

/// An OCPI timestamp: an instant in UTC, serialised as RFC 3339 with a `Z` designator.
///
/// > *All timestamps are formatted as string(25) following RFC 3339, with some additional
/// > limitations. All timestamps SHALL be in UTC. The absence of the timezone designator implies
/// > a UTC timestamp. Fractional seconds MAY be used.*
///
/// The six forms the spec lists as the only allowed ones are, by example:
///
/// ```text
/// 2015-06-29T20:39:09Z        2015-06-29T20:39:09
/// 2016-12-29T17:45:09.2Z      2016-12-29T17:45:09.2
/// 2018-01-01T01:08:01.123Z    2018-01-01T01:08:01.123
/// ```
///
/// # `+00:00` is not `Z`
///
/// The spec is explicit: *"NOTE: +00:00 is not the same as UTC."* A `DateTime` parsed from a
/// timestamp carrying an explicit offset — even `+00:00` — is converted to the correct UTC
/// instant so no data is lost, but is flagged: [`DateTime::is_canonical`] returns `false` and
/// [`Validate::validate`] reports it. Serialising always emits the canonical `Z` form, so a
/// value that passes through this crate comes out conformant.
///
/// # Fractional seconds are preserved
///
/// A timestamp read as `…09.2Z` is written back as `…09.2Z`, not `…09.200Z`. The number of
/// fractional digits is formatting metadata: it takes no part in [`PartialEq`], [`Ord`] or
/// [`std::hash::Hash`], which compare the instant alone.
///
/// ```
/// use ocpi_kit::types::DateTime;
///
/// let t: DateTime = "2016-12-29T17:45:09.2Z".parse().unwrap();
/// assert_eq!(t.to_string(), "2016-12-29T17:45:09.2Z");
/// assert_eq!(t, "2016-12-29T17:45:09.200Z".parse::<DateTime>().unwrap());
/// ```
///
/// Spec: 2.3.0 §types_datetime_type
#[derive(Clone, Copy)]
pub struct DateTime {
    /// Always normalised to `UtcOffset::UTC`.
    instant: OffsetDateTime,
    /// Number of fractional-second digits to emit (0..=9).
    frac_digits: u8,
    /// Whether the source text used one of the six forms the spec permits.
    canonical: bool,
}

impl DateTime {
    /// The current time, with second precision.
    #[must_use]
    pub fn now() -> Self {
        Self::from_utc(
            OffsetDateTime::now_utc().replace_nanosecond(0).unwrap_or_else(|_| OffsetDateTime::now_utc()),
        )
    }

    /// Wraps an [`OffsetDateTime`], converting it to UTC.
    ///
    /// The number of fractional digits emitted is chosen to be the shortest that represents the
    /// value exactly: none, three, six or nine.
    #[must_use]
    pub fn from_utc(value: OffsetDateTime) -> Self {
        let instant = value.to_offset(UtcOffset::UTC);
        Self { instant, frac_digits: shortest_frac_digits(instant.nanosecond()), canonical: true }
    }

    /// The instant as an [`OffsetDateTime`] whose offset is UTC.
    #[must_use]
    pub const fn as_offset_date_time(self) -> OffsetDateTime {
        self.instant
    }

    /// Seconds since the Unix epoch.
    #[must_use]
    pub const fn unix_timestamp(self) -> i64 {
        self.instant.unix_timestamp()
    }

    /// Builds a timestamp from seconds since the Unix epoch.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDateTime`] if the value is outside the supported range.
    pub fn from_unix_timestamp(secs: i64) -> Result<Self, InvalidDateTime> {
        OffsetDateTime::from_unix_timestamp(secs)
            .map(Self::from_utc)
            .map_err(|_| InvalidDateTime::new("timestamp out of range"))
    }

    /// Whether the value was written in one of the six forms the spec allows.
    ///
    /// `false` means the source used an explicit UTC offset (including `+00:00`), a lower-case
    /// `z`, or a space instead of `T`. The instant itself is still correct.
    #[must_use]
    pub const fn is_canonical(self) -> bool {
        self.canonical
    }

    /// Returns a copy that will be written with exactly `digits` fractional-second digits.
    ///
    /// `digits` is clamped to 9.
    #[must_use]
    pub const fn with_fractional_digits(mut self, digits: u8) -> Self {
        self.frac_digits = if digits > 9 { 9 } else { digits };
        self
    }

    /// How many fractional-second digits this value will be written with.
    #[must_use]
    pub const fn fractional_digits(self) -> u8 {
        self.frac_digits
    }

    /// Parses one of the RFC 3339 forms OCPI allows, plus the tolerated deviations described on
    /// the type.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDateTime`] when the text is not a date-time at all, or carries an
    /// offset that would place it outside the representable range.
    pub fn parse(text: &str) -> Result<Self, InvalidDateTime> {
        parse_ocpi_datetime(text)
    }
}

fn shortest_frac_digits(nanos: u32) -> u8 {
    if nanos == 0 {
        0
    } else if nanos.is_multiple_of(1_000_000) {
        3
    } else if nanos.is_multiple_of(1_000) {
        6
    } else {
        9
    }
}

/// Why a timestamp could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidDateTime(String);

impl InvalidDateTime {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for InvalidDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid OCPI DateTime: {}", self.0)
    }
}

impl std::error::Error for InvalidDateTime {}

#[allow(clippy::too_many_lines)]
fn parse_ocpi_datetime(text: &str) -> Result<DateTime, InvalidDateTime> {
    let bytes = text.as_bytes();
    let err = |m: &str| InvalidDateTime::new(format!("{m} in {text:?}"));

    if bytes.len() < 19 {
        return Err(err("too short for YYYY-MM-DDTHH:MM:SS"));
    }
    let digits = |from: usize, len: usize| -> Result<u32, InvalidDateTime> {
        let slice = text.get(from..from + len).ok_or_else(|| err("truncated"))?;
        if !slice.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err("expected digits"));
        }
        slice.parse::<u32>().map_err(|_| err("expected digits"))
    };
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(err("expected YYYY-MM-DD"));
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return Err(err("expected HH:MM:SS"));
    }

    let mut canonical = true;
    match bytes[10] {
        b'T' => {}
        b't' | b' ' => canonical = false,
        _ => return Err(err("expected 'T' between date and time")),
    }

    let year = i32::try_from(digits(0, 4)?).map_err(|_| err("year out of range"))?;
    let month = u8::try_from(digits(5, 2)?).map_err(|_| err("month out of range"))?;
    let day = u8::try_from(digits(8, 2)?).map_err(|_| err("day out of range"))?;
    let hour = u8::try_from(digits(11, 2)?).map_err(|_| err("hour out of range"))?;
    let minute = u8::try_from(digits(14, 2)?).map_err(|_| err("minute out of range"))?;
    let second = u8::try_from(digits(17, 2)?).map_err(|_| err("second out of range"))?;

    let mut idx = 19;
    let mut nanos: u32 = 0;
    let mut frac_digits: u8 = 0;
    if bytes.get(idx) == Some(&b'.') || bytes.get(idx) == Some(&b',') {
        if bytes[idx] == b',' {
            canonical = false;
        }
        idx += 1;
        let start = idx;
        while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
            idx += 1;
        }
        if idx == start {
            return Err(err("fractional separator with no digits"));
        }
        let raw = &text[start..idx];
        // More digits than a `u8` can count is still "more than nine", so it clamps and is
        // flagged like any other over-long fraction rather than falling through as canonical.
        frac_digits = u8::try_from(raw.len()).unwrap_or(u8::MAX);
        // Scale the first nine digits to nanoseconds; ignore any beyond.
        let mut scaled = String::with_capacity(9);
        scaled.push_str(&raw[..raw.len().min(9)]);
        while scaled.len() < 9 {
            scaled.push('0');
        }
        nanos = scaled.parse().map_err(|_| err("bad fractional seconds"))?;
        if frac_digits > 9 {
            frac_digits = 9;
            canonical = false;
        }
    }

    let offset_minutes: i32 = match bytes.get(idx) {
        None => {
            // "The absence of the timezone designator implies a UTC timestamp."
            0
        }
        Some(b'Z') => {
            idx += 1;
            0
        }
        Some(b'z') => {
            canonical = false;
            idx += 1;
            0
        }
        Some(sign @ (b'+' | b'-')) => {
            // Spec: "+00:00 is not the same as UTC" — accepted, but not canonical.
            canonical = false;
            let sign = if *sign == b'-' { -1 } else { 1 };
            idx += 1;
            let oh = i32::try_from(digits(idx, 2)?).map_err(|_| err("offset out of range"))?;
            idx += 2;
            if bytes.get(idx) == Some(&b':') {
                idx += 1;
            }
            let om = i32::try_from(digits(idx, 2)?).map_err(|_| err("offset out of range"))?;
            idx += 2;
            sign * (oh * 60 + om)
        }
        Some(_) => return Err(err("unexpected trailing characters")),
    };
    if idx != bytes.len() {
        return Err(err("unexpected trailing characters"));
    }

    let date = time::Date::from_calendar_date(
        year,
        time::Month::try_from(month).map_err(|_| err("month out of range"))?,
        day,
    )
    .map_err(|_| err("no such calendar date"))?;
    // RFC 3339 allows second 60 for leap seconds; clamp to 59 as `time` has no leap seconds.
    let (second, leap) = if second == 60 { (59, true) } else { (second, false) };
    let time_of_day =
        time::Time::from_hms_nano(hour, minute, second, nanos).map_err(|_| err("no such time of day"))?;
    if leap {
        canonical = false;
    }
    let naive = PrimitiveDateTime::new(date, time_of_day);
    let offset =
        UtcOffset::from_whole_seconds(offset_minutes * 60).map_err(|_| err("offset out of range"))?;
    let instant = naive.assume_offset(offset).to_offset(UtcOffset::UTC);

    Ok(DateTime { instant, frac_digits, canonical })
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = self.instant.date();
        let t = self.instant.time();
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            d.year(),
            u8::from(d.month()),
            d.day(),
            t.hour(),
            t.minute(),
            t.second()
        )?;
        if self.frac_digits > 0 {
            let nanos = t.nanosecond();
            let text = format!("{nanos:09}");
            f.write_str(".")?;
            f.write_str(&text[..usize::from(self.frac_digits)])?;
        }
        f.write_str("Z")
    }
}

impl fmt::Debug for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DateTime({self})")
    }
}

impl PartialEq for DateTime {
    fn eq(&self, other: &Self) -> bool {
        self.instant == other.instant
    }
}
impl Eq for DateTime {}

impl PartialOrd for DateTime {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for DateTime {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.instant.cmp(&other.instant)
    }
}
impl core::hash::Hash for DateTime {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.instant.hash(state);
    }
}

impl From<OffsetDateTime> for DateTime {
    fn from(value: OffsetDateTime) -> Self {
        Self::from_utc(value)
    }
}

impl From<DateTime> for OffsetDateTime {
    fn from(value: DateTime) -> Self {
        value.instant
    }
}

impl FromStr for DateTime {
    type Err = InvalidDateTime;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_ocpi_datetime(s)
    }
}

impl TryFrom<&str> for DateTime {
    type Error = InvalidDateTime;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        parse_ocpi_datetime(s)
    }
}

impl Validate for DateTime {
    fn validate_in(&self, v: &mut Validator) {
        if !self.canonical {
            v.report(
                ViolationCode::Inconsistent,
                "timestamp was not written in one of the six forms OCPI allows \
                 (an explicit UTC offset, a lower-case 'z' or a space separator was used); \
                 note that the spec states \"+00:00 is not the same as UTC\"",
            );
        }
    }
}

impl Serialize for DateTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DateTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = DateTime;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an RFC 3339 UTC timestamp such as \"2015-06-29T20:39:09Z\"")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<DateTime, E> {
                parse_ocpi_datetime(v).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(V)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for DateTime {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DateTime".into()
    }
    fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "date-time",
            "maxLength": 25,
            "description": "OCPI DateTime: RFC 3339, always UTC",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_six_spec_forms() {
        for (text, expect) in [
            ("2015-06-29T20:39:09Z", "2015-06-29T20:39:09Z"),
            ("2015-06-29T20:39:09", "2015-06-29T20:39:09Z"),
            ("2016-12-29T17:45:09.2Z", "2016-12-29T17:45:09.2Z"),
            ("2016-12-29T17:45:09.2", "2016-12-29T17:45:09.2Z"),
            ("2018-01-01T01:08:01.123Z", "2018-01-01T01:08:01.123Z"),
            ("2018-01-01T01:08:01.123", "2018-01-01T01:08:01.123Z"),
        ] {
            let dt: DateTime = text.parse().unwrap();
            assert!(dt.is_canonical(), "{text} should be canonical");
            assert_eq!(dt.to_string(), expect, "round-trip of {text}");
        }
    }

    #[test]
    fn explicit_offsets_are_converted_and_flagged() {
        let dt: DateTime = "2015-06-29T22:39:09+02:00".parse().unwrap();
        assert_eq!(dt.to_string(), "2015-06-29T20:39:09Z");
        assert!(!dt.is_canonical());
        let violations = dt.validate().unwrap_err();
        assert_eq!(violations.as_slice()[0].code, ViolationCode::Inconsistent);

        // The spec's own note: +00:00 is not the same as UTC.
        let zero: DateTime = "2015-06-29T20:39:09+00:00".parse().unwrap();
        assert_eq!(zero.to_string(), "2015-06-29T20:39:09Z");
        assert!(!zero.is_canonical());
    }

    #[test]
    fn equality_ignores_fractional_digit_count() {
        let a: DateTime = "2016-12-29T17:45:09.2Z".parse().unwrap();
        let b: DateTime = "2016-12-29T17:45:09.200Z".parse().unwrap();
        assert_eq!(a, b);
        assert_ne!(a.to_string(), b.to_string(), "but formatting is preserved");
    }

    #[test]
    fn rejects_nonsense() {
        for bad in [
            "",
            "2015-06-29",
            "not a date",
            "2015-13-01T00:00:00Z",
            "2015-06-29T25:00:00Z",
            "2015-06-29T20:39:09Zjunk",
        ] {
            assert!(bad.parse::<DateTime>().is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn an_over_long_fraction_is_truncated_and_flagged() {
        // Nine digits is all a nanosecond timestamp can hold, and `string(25)` cannot carry
        // them anyway; the instant survives, the deviation is reported.
        let dt: DateTime = "2018-01-01T01:08:01.1234567891234Z".parse().unwrap();
        assert_eq!(dt.fractional_digits(), 9);
        assert!(!dt.is_canonical());
        assert_eq!(dt.to_string(), "2018-01-01T01:08:01.123456789Z");

        let absurd = format!("2018-01-01T01:08:01.{}Z", "1".repeat(300));
        let dt: DateTime = absurd.parse().unwrap();
        assert!(!dt.is_canonical(), "300 fractional digits is not one of the six forms");
    }

    #[test]
    fn serde_round_trip() {
        let json = "\"2018-01-01T01:08:01.123Z\"";
        let dt: DateTime = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&dt).unwrap(), json);
    }

    #[test]
    fn from_utc_picks_the_shortest_exact_fraction() {
        let base = OffsetDateTime::from_unix_timestamp(1_500_000_000).unwrap();
        assert_eq!(DateTime::from_utc(base).fractional_digits(), 0);
        let ms = base.replace_nanosecond(120_000_000).unwrap();
        assert_eq!(DateTime::from_utc(ms).fractional_digits(), 3);
        let us = base.replace_nanosecond(120_000_100).unwrap();
        assert_eq!(DateTime::from_utc(us).fractional_digits(), 9);
    }
}
