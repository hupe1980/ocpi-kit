//! Credentials tokens: the `Authorization: Token <base64>` header, done carefully.

use core::fmt;

use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::types::{OcpiString, Validate, Validator, ViolationCode};

/// The `Authorization` header value prefix OCPI uses.
///
/// > *The literal 'Token' indicates that the token-based authentication mechanism is used.*
pub const TOKEN_PREFIX: &str = "Token ";

/// A credentials token: the shared secret two platforms authenticate each other with.
///
/// > *`token`: The credentials token for the other party to authenticate in your system. It
/// > should only contain printable non-whitespace ASCII characters, that is, characters with
/// > Unicode code points from the range of U+0021 up to and including U+007E.*
///
/// This type exists so that a credentials token is hard to leak:
///
/// * [`Debug`] and [`Display`](fmt::Display) print `Token(****ab12)`, never the secret. A token
///   that ends up in a `tracing` span, a panic message or a serialised error is therefore not a
///   disclosure.
/// * [`PartialEq`] compares in **constant time**, so a server that looks a token up by comparing
///   against known tokens does not leak its contents through timing.
/// * The buffer is zeroised when the token is dropped.
/// * There is no `Serialize`: a token reaches the wire only through
///   [`CredentialsToken::to_header_value`], or as the
///   [`Credentials.token`](crate::v2_3_0::credentials::Credentials::token) field of a
///   credentials object, which is the one place the protocol puts it in a body.
///
/// ```
/// use ocpi_kit::transport::CredentialsToken;
///
/// let token = CredentialsToken::new("example-token").unwrap();
/// assert_eq!(token.to_header_value(), "Token ZXhhbXBsZS10b2tlbg==");
/// assert_eq!(format!("{token:?}"), "Token(****oken)");
/// ```
///
/// Spec: 2.3.0 §transport_and_format_authorization_header, §credentials_credentials_object
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct CredentialsToken(String);

impl CredentialsToken {
    /// The maximum length the spec gives: `string(64)`.
    pub const MAX_LEN: usize = 64;

    /// Creates a token, enforcing the character set and length the spec gives.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidToken`] if the value is empty, longer than 64 characters, or contains a
    /// character outside U+0021..=U+007E — which notably excludes the space.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidToken> {
        let value = value.into();
        if value.is_empty() {
            return Err(InvalidToken("a credentials token cannot be empty".to_owned()));
        }
        if value.chars().count() > Self::MAX_LEN {
            return Err(InvalidToken(format!(
                "a credentials token is string(64); this one has {} characters",
                value.chars().count()
            )));
        }
        if let Some(bad) = value.chars().find(|c| !matches!(c, '!'..='~')) {
            return Err(InvalidToken(format!(
                "a credentials token may only contain U+0021..U+007E; found U+{:04X}",
                bad as u32
            )));
        }
        Ok(Self(value))
    }

    /// Creates a token without enforcing anything, for values read off the wire.
    pub fn new_lenient(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Generates a fresh random token.
    ///
    /// Produces a hyphenated UUID v4, which is what the specification's own examples use and
    /// what the vast majority of implementations do.
    #[must_use]
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// The token in cleartext.
    ///
    /// Named to be conspicuous at a call site: everything else about this type is designed to
    /// stop the secret escaping by accident.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    /// The token as the `string(64)` that goes into a `Credentials` object body.
    #[must_use]
    pub fn to_credentials_field(&self) -> OcpiString<64> {
        OcpiString::new_lenient(self.0.clone())
    }

    /// The full `Authorization` header value, Base64-encoded as the spec requires.
    ///
    /// > *After the literal 'Token', there SHALL be one space, followed by the 'encoded token'.
    /// > The encoded token is obtained by encoding the credentials token to an octet sequence
    /// > with UTF-8 and then encoding that octet sequence with Base64 according to RFC 4648.*
    #[must_use]
    pub fn to_header_value(&self) -> String {
        use base64::Engine as _;
        format!("{TOKEN_PREFIX}{}", base64::engine::general_purpose::STANDARD.encode(&self.0))
    }

    /// The `Authorization` header value **without** Base64, for pre-2.2-d2 peers.
    ///
    /// > *NOTE: Many OCPI 2.1.1 and 2.2 implementations do not Base64 encode the credentials
    /// > token when including it in the 'Authorization' header. … Implementations that wish to be
    /// > compatible with non-encoding 2.1.1 and 2.2 implementations have to choose the right way
    /// > to parse and write authorization headers by either trial and error or configuration
    /// > flags.*
    ///
    /// This crate chooses configuration flags: see
    /// [`Quirks::send_unencoded_token`](super::Quirks::send_unencoded_token).
    #[must_use]
    pub fn to_header_value_unencoded(&self) -> String {
        format!("{TOKEN_PREFIX}{}", self.0)
    }

    /// Parses an `Authorization` header value.
    ///
    /// Both encodings are accepted: the value is Base64-decoded when that yields a valid token,
    /// and otherwise taken literally. `accept_unencoded` gates the fallback — leave it off for a
    /// peer that is known to encode properly, so that a mangled header is an error rather than a
    /// token nobody recognises.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidToken`] if the value does not start with `Token `, or if what follows is
    /// neither valid Base64 of a token nor (when allowed) a bare token.
    pub fn parse_header(value: &str, accept_unencoded: bool) -> Result<Self, InvalidToken> {
        use base64::Engine as _;

        let rest = strip_token_prefix(value)
            .ok_or_else(|| InvalidToken("Authorization header does not start with \"Token \"".into()))?;
        if rest.is_empty() {
            return Err(InvalidToken("Authorization header has no token".into()));
        }

        // The spec mandates RFC 4648 §4 with padding; tolerate the unpadded form on input.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(rest)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(rest));

        if let Ok(bytes) = decoded
            && let Ok(text) = String::from_utf8(bytes)
        {
            // A token that decodes to something outside the charset is more likely a token
            // that merely *looked* like Base64; fall through to the literal reading.
            if !text.is_empty() && text.chars().all(|c| matches!(c, '!'..='~')) {
                return Ok(Self(text));
            }
        }

        if accept_unencoded {
            return Self::new(rest);
        }
        Err(InvalidToken(
            "Authorization header is not Base64-encoded as OCPI 2.2-d2 and later require; \
             set Quirks::accept_unencoded_token for peers that predate that"
                .into(),
        ))
    }

    /// Whether this value satisfies the character set and length the spec gives.
    #[must_use]
    pub fn is_conformant(&self) -> bool {
        !self.0.is_empty()
            && self.0.chars().count() <= Self::MAX_LEN
            && self.0.chars().all(|c| matches!(c, '!'..='~'))
    }

    /// A stable, non-reversible fingerprint, for logging and correlating without disclosure.
    ///
    /// This is the last four characters of the token, which is what the redacted `Debug` shows.
    /// It is a debugging aid, not a secret-safe identifier for a short token.
    #[must_use]
    pub fn hint(&self) -> String {
        let n = self.0.chars().count();
        let tail: String = self.0.chars().skip(n.saturating_sub(4)).collect();
        format!("****{tail}")
    }
}

fn strip_token_prefix(value: &str) -> Option<&str> {
    // "NOTE: HTTP header names are case-insensitive" — the scheme name conventionally is too.
    let (scheme, rest) = value.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("Token") { Some(rest.trim_start()) } else { None }
}

impl PartialEq for CredentialsToken {
    /// Constant-time comparison: a server resolving a token must not leak it through timing.
    fn eq(&self, other: &Self) -> bool {
        let a = self.0.as_bytes();
        let b = other.0.as_bytes();
        // `ct_eq` requires equal lengths; comparing the lengths first leaks only the length,
        // which the Base64 in the header already reveals.
        a.len() == b.len() && bool::from(a.ct_eq(b))
    }
}

impl Eq for CredentialsToken {}

impl fmt::Debug for CredentialsToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token({})", self.hint())
    }
}

impl fmt::Display for CredentialsToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token({})", self.hint())
    }
}

impl core::str::FromStr for CredentialsToken {
    type Err = InvalidToken;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl Validate for CredentialsToken {
    fn validate_in(&self, v: &mut Validator) {
        if !self.is_conformant() {
            v.report(ViolationCode::IllegalCharacter, "a credentials token is string(64) of U+0021..U+007E");
        }
    }
}

/// Why a string is not a usable credentials token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidToken(String);

impl fmt::Display for InvalidToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid credentials token: {}", self.0)
    }
}
impl std::error::Error for InvalidToken {}

/// Which token of the registration handshake a value is.
///
/// > *the Receiver Platform must create a unique credentials token: `CREDENTIALS_TOKEN_A` … The
/// > Sender generates a unique credentials token: `CREDENTIALS_TOKEN_B` … The Receiver generates
/// > a unique credentials token: `CREDENTIALS_TOKEN_C`.*
///
/// The distinction matters at runtime because Token A is scoped:
///
/// > *When a server receives a request with a valid `CREDENTIALS_TOKEN_A`, on another module
/// > than `credentials` or `versions`, the server SHALL respond with an HTTP `401 -
/// > Unauthorized` status code.*
///
/// Spec: 2.3.0 §credentials_registration, §transport_and_format_authorization_header
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenRole {
    /// `CREDENTIALS_TOKEN_A`: the bootstrap token, valid only for `credentials` and `versions`.
    A,
    /// `CREDENTIALS_TOKEN_B`: the token the Sender gives the Receiver in the POST.
    B,
    /// `CREDENTIALS_TOKEN_C`: the token the Receiver returns, used for everything afterwards.
    C,
}

impl TokenRole {
    /// Whether a request authenticated with this token may address `module`.
    ///
    /// Spec: 2.3.0 §transport_and_format_authorization_header
    #[must_use]
    pub fn may_access(self, module: &crate::ModuleId) -> bool {
        use crate::ModuleId;
        match self {
            Self::A => matches!(module, ModuleId::Credentials | ModuleId::Versions),
            Self::B | Self::C => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ModuleId;

    #[test]
    fn header_encoding_matches_the_spec_example() {
        // The spec's own example: credentials token 'example-token'.
        let token = CredentialsToken::new("example-token").unwrap();
        assert_eq!(token.to_header_value(), "Token ZXhhbXBsZS10b2tlbg==");
        assert_eq!(token.to_header_value_unencoded(), "Token example-token");
    }

    #[test]
    fn header_parsing_accepts_both_encodings_under_the_flag() {
        let encoded = "Token ZXhhbXBsZS10b2tlbg==";
        let parsed = CredentialsToken::parse_header(encoded, false).unwrap();
        assert_eq!(parsed.expose_secret(), "example-token");

        // A bare token is not valid Base64 of a token, so it needs the quirk.
        let bare = "Token 12345678-1234-1234-1234-123456789012";
        assert!(CredentialsToken::parse_header(bare, false).is_err());
        assert_eq!(
            CredentialsToken::parse_header(bare, true).unwrap().expose_secret(),
            "12345678-1234-1234-1234-123456789012"
        );
    }

    #[test]
    fn header_parsing_is_case_insensitive_on_the_scheme_and_rejects_junk() {
        assert!(CredentialsToken::parse_header("token ZXhhbXBsZS10b2tlbg==", false).is_ok());
        assert!(CredentialsToken::parse_header("Bearer abc", true).is_err());
        assert!(CredentialsToken::parse_header("Token ", true).is_err());
        assert!(CredentialsToken::parse_header("", true).is_err());
    }

    #[test]
    fn the_secret_never_appears_in_debug_or_display() {
        let token = CredentialsToken::new("super-secret-value").unwrap();
        let debug = format!("{token:?}");
        let display = format!("{token}");
        for rendering in [&debug, &display] {
            assert!(!rendering.contains("super-secret"), "{rendering}");
            assert!(rendering.contains("****alue"), "{rendering}");
        }
    }

    #[test]
    fn the_charset_excludes_whitespace() {
        assert!(CredentialsToken::new("has space").is_err());
        assert!(CredentialsToken::new("").is_err());
        assert!(CredentialsToken::new("a".repeat(65)).is_err());
        assert!(CredentialsToken::new("a".repeat(64)).is_ok());
        assert!(CredentialsToken::new("~!@#$%^&*()_+").is_ok());
    }

    #[test]
    fn equality_is_value_based_and_generated_tokens_differ() {
        let a = CredentialsToken::new("same").unwrap();
        let b = CredentialsToken::new("same").unwrap();
        let c = CredentialsToken::new("other").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(CredentialsToken::generate(), CredentialsToken::generate());
        assert!(CredentialsToken::generate().is_conformant());
    }

    #[test]
    fn token_a_is_scoped_to_credentials_and_versions() {
        assert!(TokenRole::A.may_access(&ModuleId::Credentials));
        assert!(TokenRole::A.may_access(&ModuleId::Versions));
        assert!(!TokenRole::A.may_access(&ModuleId::Locations));
        assert!(!TokenRole::A.may_access(&ModuleId::Cdrs));
        assert!(TokenRole::C.may_access(&ModuleId::Locations));
    }
}
