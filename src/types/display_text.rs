//! `DisplayText` — a string with the language it is written in.

use serde::{Deserialize, Serialize};

use super::extensions::Extensions;
use super::string::OcpiString;
use super::text::InvalidString;
use super::validate::{Validate, Validator};
use super::validate_fields;

/// Text to be shown to an end user, tagged with its language.
///
/// > *`language`: Language Code ISO 639-1. `text`: Text to be displayed to a end user. No markup,
/// > html etc. allowed.*
///
/// Spec: 2.3.0 §types_displaytext_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct DisplayText {
    /// Language Code ISO 639-1.
    pub language: OcpiString<2>,
    /// Text to be displayed to an end user. No markup, HTML etc. allowed.
    pub text: OcpiString<512>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl DisplayText {
    /// Creates a `DisplayText`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidString`] if the language code is not two characters or the text is
    /// longer than 512 characters, or if either contains a control character.
    pub fn new(language: impl Into<String>, text: impl Into<String>) -> Result<Self, InvalidString> {
        Ok(Self {
            language: OcpiString::new(language)?,
            text: OcpiString::new(text)?,
            extensions: Extensions::new(),
        })
    }
}

impl Validate for DisplayText {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, language, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_keeps_extensions() {
        let json = r#"{"language":"en","text":"2 euro per hour","nltnm_source":"cms"}"#;
        let dt: DisplayText = serde_json::from_str(json).unwrap();
        assert_eq!(dt.text.as_str(), "2 euro per hour");
        assert_eq!(serde_json::to_string(&dt).unwrap(), json);
    }

    #[test]
    fn constructor_enforces_the_language_code_length() {
        assert!(DisplayText::new("en", "hello").is_ok());
        assert!(DisplayText::new("english", "hello").is_err());
    }
}
