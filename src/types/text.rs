//! Shared character-set rules for the two OCPI string types.

use core::fmt;

/// Which of the two OCPI string types a rule applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringKind {
    /// `CiString`: case-insensitive, printable ASCII only.
    ///
    /// Spec: 2.3.0 §types_cistring_type
    Ci,
    /// `string`: case-sensitive, printable UTF-8.
    ///
    /// Spec: 2.3.0 §types_string_type
    Utf8,
}

impl StringKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Ci => "CiString",
            Self::Utf8 => "string",
        }
    }
}

/// Why a string could not be accepted by a strict constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidString {
    kind: StringKind,
    reason: Reason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Reason {
    TooLong { len: usize, max: usize },
    NonPrintable { at: usize, ch: char },
    NonAscii { at: usize, ch: char },
}

impl InvalidString {
    pub(crate) const fn too_long(len: usize, max: usize, kind: StringKind) -> Self {
        Self { kind, reason: Reason::TooLong { len, max } }
    }

    /// Whether the string was rejected only because it was too long.
    #[must_use]
    pub const fn is_too_long(&self) -> bool {
        matches!(self.reason, Reason::TooLong { .. })
    }

    /// The string type whose rules were broken.
    #[must_use]
    pub const fn kind(&self) -> StringKind {
        self.kind
    }
}

impl fmt::Display for InvalidString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.kind.name();
        match self.reason {
            Reason::TooLong { len, max } => {
                write!(f, "{name}({max}) cannot hold {len} characters")
            }
            Reason::NonPrintable { at, ch } => write!(
                f,
                "{name} must contain only printable characters, found U+{:04X} at index {at}",
                ch as u32
            ),
            Reason::NonAscii { at, ch } => {
                write!(f, "{name} must contain only ASCII, found U+{:04X} at index {at}", ch as u32)
            }
        }
    }
}

impl std::error::Error for InvalidString {}

/// Enforces the `CiString` character set: U+0020..=U+007E.
///
/// Spec: 2.3.0 §types_cistring_type — *"Only printable ASCII allowed. (Non-printable characters
/// like: Carriage returns, Tabs, Line breaks, etc are not allowed)"*
pub(crate) fn check_printable_ascii(value: &str, kind: StringKind) -> Result<(), InvalidString> {
    for (at, ch) in value.char_indices() {
        if !ch.is_ascii() {
            return Err(InvalidString { kind, reason: Reason::NonAscii { at, ch } });
        }
        if !is_printable_ascii(ch) {
            return Err(InvalidString { kind, reason: Reason::NonPrintable { at, ch } });
        }
    }
    Ok(())
}

/// Enforces the `string` character set: printable UTF-8, no control characters.
///
/// Spec: 2.3.0 §types_string_type — *"Case Sensitive String. Only printable UTF-8 allowed."*
///
/// "Printable" is read as "not a Unicode control character": C0 (U+0000..=U+001F), DEL (U+007F)
/// and C1 (U+0080..=U+009F) are rejected, everything else — including emoji and all scripts —
/// is accepted. The spec names carriage returns, tabs and line breaks as the motivating cases.
pub(crate) fn check_printable_utf8(value: &str, kind: StringKind) -> Result<(), InvalidString> {
    for (at, ch) in value.char_indices() {
        if ch.is_control() {
            return Err(InvalidString { kind, reason: Reason::NonPrintable { at, ch } });
        }
    }
    Ok(())
}

const fn is_printable_ascii(ch: char) -> bool {
    matches!(ch, ' '..='~')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_rules() {
        assert!(check_printable_ascii("Hello, World! ~", StringKind::Ci).is_ok());
        assert!(check_printable_ascii("tab\there", StringKind::Ci).is_err());
        assert!(check_printable_ascii("\u{7f}", StringKind::Ci).is_err(), "DEL is not printable");
        assert!(check_printable_ascii("é", StringKind::Ci).is_err());
    }

    #[test]
    fn utf8_rules() {
        assert!(check_printable_utf8("Straße — 日本語 🚗", StringKind::Utf8).is_ok());
        assert!(check_printable_utf8("line\nbreak", StringKind::Utf8).is_err());
        assert!(check_printable_utf8("\u{85}", StringKind::Utf8).is_err(), "C1 NEL is a control");
    }

    #[test]
    fn error_messages_name_the_offending_index() {
        let e = check_printable_ascii("ab\tcd", StringKind::Ci).unwrap_err();
        assert!(e.to_string().contains("index 2"), "{e}");
        assert!(!e.is_too_long());
    }
}
