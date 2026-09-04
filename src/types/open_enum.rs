//! The two enum shapes OCPI 2.3.0 distinguishes, as declarative macros.
//!
//! OCPI 2.3.0 formalised the difference between an `enum` and an `OpenEnum`
//! (§types_enum_type, §types_openenum_type):
//!
//! * An **enum** has *"a finite number of strings … completely known at the time of writing of
//!   the specification"*. An unknown value is a protocol error.
//! * An **OpenEnum** is for fields *"for which the set of all possible values is not known at
//!   the time of writing"*. Implementers are expected to add their own values, following
//!   [RFC 6648](https://datatracker.ietf.org/doc/html/rfc6648).
//!
//! [`ocpi_enum!`](crate::ocpi_enum) and [`ocpi_open_enum!`](crate::ocpi_open_enum) generate the
//! two shapes. The important difference is what happens to a value the crate has never heard of:
//! a closed enum refuses it, an open enum **keeps it** in a `Custom` variant so that a hub or a
//! pull-store-push pipeline hands it on untouched. Discarding it — which is what a plain
//! `#[derive(Deserialize)]` enum does — would make this crate lossy in exactly the place OCPI's
//! extensibility chapter cares about.
//!
//! Both macros are exported, so a party defining a custom module can use them for its own types.

/// Why a string is not a member of a closed OCPI enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownVariant {
    enum_name: &'static str,
    value: String,
    allowed: &'static [&'static str],
}

impl UnknownVariant {
    /// Creates an error for `value`, which is not one of `allowed`.
    #[must_use]
    pub fn new(enum_name: &'static str, value: impl Into<String>, allowed: &'static [&'static str]) -> Self {
        Self { enum_name, value: value.into(), allowed }
    }

    /// The value that was not recognised.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The name of the enum that rejected it.
    #[must_use]
    pub const fn enum_name(&self) -> &'static str {
        self.enum_name
    }

    /// Every value the enum does accept.
    #[must_use]
    pub const fn allowed(&self) -> &'static [&'static str] {
        self.allowed
    }
}

impl core::fmt::Display for UnknownVariant {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?} is not a valid {}; expected one of ", self.value, self.enum_name)?;
        for (i, a) in self.allowed.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{a}")?;
        }
        Ok(())
    }
}

impl std::error::Error for UnknownVariant {}

/// Defines a **closed** OCPI `enum`: a fixed set of strings, where anything else is an error.
///
/// Use it where a value *drives a decision* — the pricing engine's dimensions, a routing role, the
/// outcome of a command — because an unrecognised value there has no meaning to act on. Where the
/// value is only carried, [`ocpi_lenient_enum!`](crate::ocpi_lenient_enum) keeps it instead of
/// losing the object around it.
///
/// ```
/// use ocpi_kit::ocpi_enum;
///
/// ocpi_enum! {
///     /// The unit a charging rate limit is expressed in.
///     ///
///     /// Spec: 2.3.0 §mod_charging_profiles_chargingrateunit_enum
///     pub enum ChargingRateUnit {
///         /// Watts.
///         W = "W",
///         /// Amperes per phase.
///         A = "A",
///     }
/// }
///
/// assert_eq!(ChargingRateUnit::W.as_str(), "W");
/// assert!("KW".parse::<ChargingRateUnit>().is_err());
/// ```
///
/// Attributes that document the enum — `#[cfg_attr(docsrs, doc(cfg(…)))]` above all — go **inside**
/// the invocation, on the `pub enum` line, so they land on the item this expands to. On the
/// invocation itself they document nothing, and rustdoc refuses them. A `#[cfg]` is the exception:
/// it belongs outside, where it gates the whole expansion rather than the enum alone.
#[macro_export]
macro_rules! ocpi_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident = $wire:literal
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis enum $name {
            $(
                $(#[$vmeta])*
                #[doc = concat!("\n\nWire value: `", $wire, "`")]
                $variant,
            )*
        }

        impl $name {
            /// Every value this enum accepts, in declaration order.
            pub const ALL: &'static [Self] = &[ $( Self::$variant ),* ];
            /// Every wire value this enum accepts, in declaration order.
            pub const ALL_WIRE: &'static [&'static str] = &[ $( $wire ),* ];

            /// The value as it appears on the wire.
            #[must_use]
            pub const fn as_str(&self) -> &'static str {
                match self { $( Self::$variant => $wire, )* }
            }

            /// Parses a wire value, ignoring ASCII case.
            ///
            /// OCPI enum values are case-sensitive, so this is only for peers known to get the
            /// case wrong; [`FromStr`](core::str::FromStr) is the strict version.
            #[must_use]
            pub fn from_str_ignore_case(s: &str) -> Option<Self> {
                $( if s.eq_ignore_ascii_case($wire) { return Some(Self::$variant); } )*
                None
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl core::str::FromStr for $name {
            type Err = $crate::types::UnknownVariant;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $wire => Ok(Self::$variant), )*
                    other => Err($crate::types::UnknownVariant::new(
                        stringify!($name), other, Self::ALL_WIRE,
                    )),
                }
            }
        }

        impl $crate::types::Validate for $name {
            fn validate_in(&self, _v: &mut $crate::types::Validator) {}
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl serde::de::Visitor<'_> for V {
                    type Value = $name;
                    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                        write!(f, "one of the {} values of {}", $name::ALL_WIRE.len(), stringify!($name))
                    }
                    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<$name, E> {
                        <$name as core::str::FromStr>::from_str(v).map_err(E::custom)
                    }
                }
                d.deserialize_str(V)
            }
        }

        #[cfg(feature = "schema")]
        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> { stringify!($name).into() }
            fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({ "type": "string", "enum": Self::ALL_WIRE })
            }
        }
    };
}

/// Defines an OCPI `OpenEnum`: known values plus a `Custom` variant that preserves anything else.
///
/// ```
/// use ocpi_kit::ocpi_open_enum;
///
/// ocpi_open_enum! {
///     /// Categories of environmental impact values.
///     ///
///     /// Spec: 2.3.0 §mod_locations_environmentalimpactcategory_enum
///     pub enum EnvironmentalImpactCategory {
///         /// Produced nuclear waste in grams per kilowatthour.
///         NuclearWaste = "NUCLEAR_WASTE",
///         /// Exhausted carbon dioxide in grams per kilowatthour.
///         CarbonDioxide = "CARBON_DIOXIDE",
///     }
/// }
///
/// let vendor: EnvironmentalImpactCategory = "nltnm-METHANE".parse().unwrap();
/// assert!(!vendor.is_known());
/// assert_eq!(vendor.as_str(), "nltnm-METHANE"); // never dropped, never rewritten
/// ```
#[macro_export]
macro_rules! ocpi_open_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident = $wire:literal
            ),* $(,)?
        }
    ) => {
        $crate::__ocpi_open_enum_impl! {
            @policy $crate::types::validate_open_enum_value;
            $(#[$meta])*
            ///
            /// This is an OCPI `OpenEnum`: a value this version of the specification does not
            /// define is a legitimate extension, and [`Validate`](crate::types::Validate) does
            /// not report it.
            $vis enum $name { $( $(#[$vmeta])* $variant = $wire, )* }
        }
    };
}

/// Defines an enum the specification declares **closed**, but which this crate still accepts
/// unknown values for — and reports them.
///
/// OCPI 2.2.1 has no `OpenEnum` at all: `ConnectorType`, `TokenType` and the rest are closed, so
/// by the letter of that specification an unrecognised connector type is a decode error. In
/// practice new plug standards appear faster than OCPI releases — OCPI 2.3.0 reclassified
/// exactly these enums as `OpenEnum` for that reason — and refusing the value would make a whole
/// page of Locations undecodable over one connector nobody has heard of.
///
/// So the value is kept, and [`Validate`](crate::types::Validate) reports it as a
/// [`ViolationCode::Inconsistent`](crate::types::ViolationCode::Inconsistent) violation. Decoding
/// succeeds; a conformance report still says the peer sent something its own version does not
/// define.
#[macro_export]
macro_rules! ocpi_lenient_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident = $wire:literal
            ),* $(,)?
        }
    ) => {
        $crate::__ocpi_open_enum_impl! {
            @policy $crate::types::validate_closed_enum_value;
            $(#[$meta])*
            ///
            /// The specification declares this enum **closed**. This crate still decodes an
            /// unrecognised value into [`Custom`](Self::Custom) rather than failing the whole
            /// object, and [`Validate`](crate::types::Validate) reports it.
            $vis enum $name { $( $(#[$vmeta])* $variant = $wire, )* }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __ocpi_open_enum_impl {
    (
        @policy $policy:path;
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident = $wire:literal
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug)]
        #[non_exhaustive]
        $vis enum $name {
            $(
                $(#[$vmeta])*
                #[doc = concat!("\n\nWire value: `", $wire, "`")]
                $variant,
            )*
            /// A value this version of the specification does not define, preserved verbatim.
            ///
            /// The variant is named `Custom` rather than `Other` because several OCPI OpenEnums
            /// have a defined value that is literally `OTHER`.
            Custom(String),
        }

        impl $name {
            /// Every value this version of the specification defines, in declaration order.
            pub const ALL_KNOWN: &'static [Self] = &[ $( Self::$variant ),* ];
            /// Every wire value this version of the specification defines.
            pub const ALL_KNOWN_WIRE: &'static [&'static str] = &[ $( $wire ),* ];

            /// The value as it appears on the wire.
            #[must_use]
            pub fn as_str(&self) -> &str {
                match self {
                    $( Self::$variant => $wire, )*
                    Self::Custom(v) => v.as_str(),
                }
            }

            /// Whether this is a value the specification defines.
            #[must_use]
            pub const fn is_known(&self) -> bool {
                !matches!(self, Self::Custom(_))
            }

            /// Parses a wire value, ignoring ASCII case for the known values.
            ///
            /// For peers that get the case wrong; [`FromStr`](core::str::FromStr) is strict.
            #[must_use]
            pub fn from_str_ignore_case(s: &str) -> Self {
                $( if s.eq_ignore_ascii_case($wire) { return Self::$variant; } )*
                Self::Custom(s.to_owned())
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl core::str::FromStr for $name {
            // Parsing an OpenEnum cannot fail: an unrecognised value is a legitimate value.
            type Err = core::convert::Infallible;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(match s {
                    $( $wire => Self::$variant, )*
                    other => Self::Custom(other.to_owned()),
                })
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                <Self as core::str::FromStr>::from_str(s).unwrap_or_else(|e| match e {})
            }
        }

        // Comparison goes through the wire value, so a value that reached `Custom` by another
        // route still equals the variant it names.
        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool { self.as_str() == other.as_str() }
        }
        impl Eq for $name {}
        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> { Some(self.cmp(other)) }
        }
        impl Ord for $name {
            fn cmp(&self, other: &Self) -> core::cmp::Ordering { self.as_str().cmp(other.as_str()) }
        }
        impl core::hash::Hash for $name {
            fn hash<H: core::hash::Hasher>(&self, state: &mut H) { self.as_str().hash(state); }
        }

        impl $crate::types::Validate for $name {
            fn validate_in(&self, v: &mut $crate::types::Validator) {
                if let Self::Custom(value) = self {
                    $policy(stringify!($name), value, v);
                }
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl serde::de::Visitor<'_> for V {
                    type Value = $name;
                    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                        write!(f, "a {} value", stringify!($name))
                    }
                    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<$name, E> {
                        Ok(<$name as core::convert::From<&str>>::from(v))
                    }
                }
                d.deserialize_str(V)
            }
        }

        #[cfg(feature = "schema")]
        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> { stringify!($name).into() }
            fn json_schema(_g: &mut schemars::SchemaGenerator) -> schemars::Schema {
                // An OpenEnum accepts any string; the known values are advertised as examples.
                schemars::json_schema!({ "type": "string", "examples": Self::ALL_KNOWN_WIRE })
            }
        }
    };
}

/// Shared validation for the `Custom` payload of an [`ocpi_lenient_enum!`] type: the value is
/// kept, but reported, because the specification version in question declares the enum closed.
///
/// Not part of the public contract; called by the generated code.
#[doc(hidden)]
pub fn validate_closed_enum_value(enum_name: &'static str, value: &str, v: &mut super::validate::Validator) {
    use super::validate::ViolationCode;
    validate_open_enum_value(enum_name, value, v);
    v.report(
        ViolationCode::Inconsistent,
        format!(
            "{value:?} is not one of the values this version of the specification defines for \
             {enum_name}, which it declares as a closed enum; the value was kept rather than \
             dropped, but a conformant peer would not have sent it"
        ),
    );
}

/// Shared validation for the `Custom` payload of every [`ocpi_open_enum!`] type.
///
/// An unrecognised value is a legitimate extension, so the value itself is never reported. What
/// is reported is a value that could not have come off a conformant wire at all: an empty string,
/// or one carrying a control character.
///
/// Not part of the public contract; called by the generated code.
#[doc(hidden)]
pub fn validate_open_enum_value(enum_name: &'static str, value: &str, v: &mut super::validate::Validator) {
    use super::validate::ViolationCode;
    if value.is_empty() {
        v.report(ViolationCode::IllegalCharacter, format!("{enum_name} value is empty"));
        return;
    }
    if value.chars().any(char::is_control) {
        v.report(
            ViolationCode::IllegalCharacter,
            format!("{enum_name} value {value:?} contains a control character"),
        );
    }
}

#[cfg(test)]
#[allow(dead_code, reason = "the generated enums expose more API than each test exercises")]
mod tests {
    use crate::types::Validate;
    use core::str::FromStr;

    crate::ocpi_enum! {
        /// Test-only closed enum.
        pub enum Closed {
            /// a
            Alpha = "ALPHA",
            /// b
            Beta = "BETA",
        }
    }

    crate::ocpi_open_enum! {
        /// Test-only open enum.
        pub enum Open {
            /// a
            Alpha = "ALPHA",
        }
    }

    #[test]
    fn closed_enum_rejects_unknown_values() {
        assert_eq!(Closed::from_str("ALPHA").unwrap(), Closed::Alpha);
        let err = serde_json::from_str::<Closed>("\"GAMMA\"").unwrap_err().to_string();
        assert!(err.contains("GAMMA") && err.contains("ALPHA"), "{err}");
        assert_eq!(Closed::ALL.len(), 2);
    }

    #[test]
    fn open_enum_preserves_unknown_values_verbatim() {
        let v: Open = serde_json::from_str("\"nltnm-CUSTOM\"").unwrap();
        assert!(!v.is_known());
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"nltnm-CUSTOM\"");
    }

    #[test]
    fn open_enum_equality_goes_through_the_wire_value() {
        use std::collections::HashSet;
        assert_eq!(Open::Custom("ALPHA".into()), Open::Alpha);
        let mut set = HashSet::new();
        set.insert(Open::Custom("ALPHA".into()));
        assert!(set.contains(&Open::Alpha), "Hash must agree with Eq");
    }

    #[test]
    fn case_insensitive_parsing_is_opt_in() {
        assert_eq!(Open::from_str("alpha").unwrap(), Open::Custom("alpha".into()));
        assert_eq!(Open::from_str_ignore_case("alpha"), Open::Alpha);
        assert_eq!(Closed::from_str_ignore_case("beta"), Some(Closed::Beta));
    }

    crate::ocpi_lenient_enum! {
        /// Test-only enum that the specification declares closed.
        pub enum ClosedInSpec {
            /// a
            Alpha = "ALPHA",
        }
    }

    #[test]
    fn a_closed_in_spec_enum_decodes_an_unknown_value_and_reports_it() {
        let v: ClosedInSpec = serde_json::from_str("\"MCS\"").unwrap();
        assert!(!v.is_known());
        // Decoding succeeded: one unknown connector type must not lose a page of Locations.
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"MCS\"");
        // But a conformance report still says the peer sent something out of spec.
        let err = v.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].code, crate::types::ViolationCode::Inconsistent);
        assert!(ClosedInSpec::Alpha.validate().is_ok());
    }

    #[test]
    fn open_enum_other_payload_is_validated() {
        assert!(Open::Custom("fine".into()).validate().is_ok());
        assert!(Open::Custom(String::new()).validate().is_err());
        assert!(Open::Custom("bad\nvalue".into()).validate().is_err());
    }
}
