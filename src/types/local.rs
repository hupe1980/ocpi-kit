//! Local wall-clock times and dates: the `string(5)` and `string(10)` fields OCPI uses for
//! opening hours and tariff restrictions.
//!
//! These are the only values in OCPI that are **not** in UTC. Both `RegularHours` and
//! `TariffRestrictions` say the same thing: *"in local time, the time zone is defined in the
//! `time_zone` field of the Location"*. Modelling them as parsed hour/minute and year/month/day
//! rather than as opaque strings is what lets [`tariffs`](crate::tariffs) evaluate a restriction
//! like "weekdays 09:00–18:00" against a session that crosses a daylight-saving boundary.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::validate::{Validate, Validator};

/// A wall-clock time of day, `HH:MM` in 24-hour form with leading zeros.
///
/// > *Must be in 24h format with leading zeros. Example: "18:15". Hour/Minute separator: ":"
/// > Regex: `([0-1][0-9]|2[0-3]):[0-5][0-9]`*
///
/// Spec: 2.3.0 §mod_locations_regularhours_class, §mod_tariffs_tariffrestrictions_class
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LocalTime {
    hour: u8,
    minute: u8,
}

impl LocalTime {
    /// Midnight, which the spec also uses to mean "end of day" in `TariffRestrictions.end_time`.
    pub const MIDNIGHT: Self = Self { hour: 0, minute: 0 };

    /// Creates a time of day.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidLocalTime`] unless `hour` is 0–23 and `minute` is 0–59.
    pub fn new(hour: u8, minute: u8) -> Result<Self, InvalidLocalTime> {
        if hour > 23 || minute > 59 {
            return Err(InvalidLocalTime(format!("{hour:02}:{minute:02} is not a time of day")));
        }
        Ok(Self { hour, minute })
    }

    /// The hour, 0–23.
    #[must_use]
    pub const fn hour(self) -> u8 {
        self.hour
    }

    /// The minute, 0–59.
    #[must_use]
    pub const fn minute(self) -> u8 {
        self.minute
    }

    /// Minutes since midnight, for comparing and for interval arithmetic.
    #[must_use]
    pub const fn minutes_since_midnight(self) -> u16 {
        self.hour as u16 * 60 + self.minute as u16
    }

    /// Whether `self` falls in the half-open interval `[start, end)`, wrapping past midnight.
    ///
    /// > *If `end_time` < `start_time` then the period wraps around to the next day. To stop at
    /// > end of the day use: 00:00.*
    ///
    /// # `start == end` is the whole day
    ///
    /// The specification does not say what `start_time == end_time` means, and the two readings
    /// are not close: taken as an empty interval the restriction never matches, taken as a
    /// wrap-around it always does. This crate reads it as **the whole day**, for two reasons.
    ///
    /// It is what the wrap-around rule already produces without a special case — the interval
    /// runs from `start`, past midnight, all the way back round to `start` — and it is the
    /// reading that fails safe. A `TariffElement` restricted to `00:00`–`00:00` that never
    /// matches leaves its dimension with no Price Component, and the specification's answer to
    /// that is that the dimension is free; a tariff writer who meant "all day" would have
    /// silently given the energy away. The other direction merely charges what the tariff says.
    ///
    /// Spec: 2.3.0 §mod_tariffs_tariffrestrictions_class
    #[must_use]
    pub const fn is_within(self, start: Self, end: Self) -> bool {
        let (t, s, e) =
            (self.minutes_since_midnight(), start.minutes_since_midnight(), end.minutes_since_midnight());
        if s == e {
            return true;
        }
        if s < e { t >= s && t < e } else { t >= s || t < e }
    }
}

impl fmt::Display for LocalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl fmt::Debug for LocalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalTime({self})")
    }
}

/// Why a string is not an `HH:MM` time of day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidLocalTime(String);

impl fmt::Display for InvalidLocalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid time of day: {}", self.0)
    }
}
impl std::error::Error for InvalidLocalTime {}

impl FromStr for LocalTime {
    type Err = InvalidLocalTime;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || InvalidLocalTime(format!("{s:?} is not \"HH:MM\""));
        let (h, m) = s.split_once(':').ok_or_else(bad)?;
        if h.len() != 2 || m.len() != 2 || !h.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        Self::new(h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?)
    }
}

impl Validate for LocalTime {
    // Unrepresentable values cannot exist: parsing already rejected them.
    fn validate_in(&self, _v: &mut Validator) {}
}

impl Serialize for LocalTime {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LocalTime {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for LocalTime {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LocalTime".into()
    }
    fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string", "maxLength": 5, "pattern": "^([0-1][0-9]|2[0-3]):[0-5][0-9]$"
        })
    }
}

/// A local calendar date, `YYYY-MM-DD`.
///
/// > *Start date in local time, the time zone is defined in the `time_zone` field of the
/// > Location, for example: 2015-12-24, valid from this day (inclusive). Regex:
/// > `([12][0-9]{3})-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])`*
///
/// Spec: 2.3.0 §mod_tariffs_tariffrestrictions_class
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate {
    year: i32,
    month: u8,
    day: u8,
}

impl LocalDate {
    /// Creates a date, checking that it exists in the proleptic Gregorian calendar.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidLocalDate`] for a date that does not exist, such as 2015-02-30.
    pub fn new(year: i32, month: u8, day: u8) -> Result<Self, InvalidLocalDate> {
        let bad = || InvalidLocalDate(format!("{year:04}-{month:02}-{day:02} is not a date"));
        let m = time::Month::try_from(month).map_err(|_| bad())?;
        time::Date::from_calendar_date(year, m, day).map_err(|_| bad())?;
        Ok(Self { year, month, day })
    }

    /// The year.
    #[must_use]
    pub const fn year(self) -> i32 {
        self.year
    }
    /// The month, 1–12.
    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }
    /// The day of the month, 1–31.
    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }

    /// The date as a [`time::Date`].
    ///
    /// # Panics
    ///
    /// Never: the constructor already proved the date exists.
    #[must_use]
    pub fn to_date(self) -> time::Date {
        time::Date::from_calendar_date(
            self.year,
            time::Month::try_from(self.month).expect("checked in constructor"),
            self.day,
        )
        .expect("checked in constructor")
    }

    /// Builds a `LocalDate` from a [`time::Date`].
    #[must_use]
    pub fn from_date(date: time::Date) -> Self {
        Self { year: date.year(), month: u8::from(date.month()), day: date.day() }
    }
}

impl fmt::Display for LocalDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}
impl fmt::Debug for LocalDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalDate({self})")
    }
}

/// Why a string is not a `YYYY-MM-DD` date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidLocalDate(String);

impl fmt::Display for InvalidLocalDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid local date: {}", self.0)
    }
}
impl std::error::Error for InvalidLocalDate {}

impl FromStr for LocalDate {
    type Err = InvalidLocalDate;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || InvalidLocalDate(format!("{s:?} is not \"YYYY-MM-DD\""));
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return Err(bad());
        }
        if !s[0..4].bytes().chain(s[5..7].bytes()).chain(s[8..10].bytes()).all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        Self::new(
            s[0..4].parse().map_err(|_| bad())?,
            s[5..7].parse().map_err(|_| bad())?,
            s[8..10].parse().map_err(|_| bad())?,
        )
    }
}

impl Validate for LocalDate {
    fn validate_in(&self, _v: &mut Validator) {}
}

impl Serialize for LocalDate {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for LocalDate {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for LocalDate {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LocalDate".into()
    }
    fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "format": "date", "maxLength": 10 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_of_day_requires_leading_zeros() {
        assert_eq!("08:15".parse::<LocalTime>().unwrap().to_string(), "08:15");
        for bad in ["8:15", "08:5", "24:00", "12:60", "0815", ""] {
            assert!(bad.parse::<LocalTime>().is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn windows_wrap_around_midnight() {
        let t = |s: &str| s.parse::<LocalTime>().unwrap();
        // 09:00-18:00, a normal daytime window.
        assert!(t("09:00").is_within(t("09:00"), t("18:00")));
        assert!(!t("18:00").is_within(t("09:00"), t("18:00")), "end is exclusive");
        assert!(!t("08:59").is_within(t("09:00"), t("18:00")));
        // 22:00-06:00 wraps to the next day.
        assert!(t("23:30").is_within(t("22:00"), t("06:00")));
        assert!(t("05:59").is_within(t("22:00"), t("06:00")));
        assert!(!t("12:00").is_within(t("22:00"), t("06:00")));
        // "To stop at end of the day use: 00:00."
        assert!(t("23:59").is_within(t("18:00"), t("00:00")));
        assert!(!t("17:59").is_within(t("18:00"), t("00:00")));
    }

    #[test]
    fn a_window_whose_ends_coincide_is_the_whole_day() {
        // The spec leaves this open; reading it as an empty window would make a tariff element
        // restricted to 00:00-00:00 never match, and the dimension it prices free of charge.
        let t = |s: &str| s.parse::<LocalTime>().unwrap();
        for probe in ["00:00", "09:30", "23:59"] {
            assert!(t(probe).is_within(t("00:00"), t("00:00")), "{probe} is inside an all-day window");
            assert!(t(probe).is_within(t("09:00"), t("09:00")), "{probe} is inside a wrapped full day");
        }
    }

    #[test]
    fn dates_must_exist() {
        assert_eq!("2015-12-24".parse::<LocalDate>().unwrap().to_string(), "2015-12-24");
        assert!("2015-02-30".parse::<LocalDate>().is_err());
        assert!("2016-02-29".parse::<LocalDate>().is_ok(), "2016 is a leap year");
        assert!("15-02-01".parse::<LocalDate>().is_err());
    }

    #[test]
    fn serde_uses_the_wire_form() {
        let t: LocalTime = serde_json::from_str("\"18:15\"").unwrap();
        assert_eq!(serde_json::to_string(&t).unwrap(), "\"18:15\"");
        let d: LocalDate = serde_json::from_str("\"2015-12-24\"").unwrap();
        assert_eq!(serde_json::to_string(&d).unwrap(), "\"2015-12-24\"");
    }
}
