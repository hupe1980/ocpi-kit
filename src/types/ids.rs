//! The identifier types OCPI reuses across every module.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Serialize};

use super::cistring::CiString;
use super::string::OcpiString;
use super::text::InvalidString;
use super::validate::{Validate, Validator};
use super::validate_fields;

/// ISO-3166 alpha-2 country code of the party that owns an object.
///
/// The spec types this as `CiString(2)`, *not* as "a valid ISO 3166 code", so this crate does not
/// reject an unassigned code. [`CountryCode::is_iso_shaped`] answers the stricter question.
///
/// Spec: 2.3.0 §credentials_credentials_role_class
pub type CountryCode = CiString<2>;

/// CPO, eMSP or other role ID of a party, following ISO-15118.
///
/// Spec: 2.3.0 §credentials_credentials_role_class
pub type PartyId = CiString<3>;

/// ISO-4217 currency code.
///
/// Spec: 2.3.0 §mod_cdrs_cdr_object — `currency`
pub type Currency = OcpiString<3>;

/// An EVSE ID in the eMI3/IDACS format, as used in `EVSE.evse_id`.
///
/// Spec: 2.3.0 §mod_locations_evse_object
pub type EvseId = CiString<48>;

/// A contract ID (eMA ID) identifying an EV driver's contract at an eMSP.
///
/// Spec: 2.3.0 §mod_tokens_token_object — `contract_id`
pub type ContractId = CiString<36>;

/// Additional checks for [`CountryCode`] values.
pub trait CountryCodeExt {
    /// Whether the value has the shape of an ISO-3166 alpha-2 code: two ASCII letters.
    ///
    /// This does not check the code against the ISO register, which changes over time and which
    /// the OCPI spec does not require a party to know.
    fn is_iso_shaped(&self) -> bool;
}

impl CountryCodeExt for CountryCode {
    fn is_iso_shaped(&self) -> bool {
        self.len() == 2 && self.as_str().bytes().all(|b| b.is_ascii_alphabetic())
    }
}

/// The `country_code` + `party_id` pair that identifies one OCPI party.
///
/// This is the pair that appears in the `OCPI-to-*` and `OCPI-from-*` routing headers, in the
/// URL of every client-owned object, and in the `roles` of a `Credentials` object. Because
/// [`CiString`] compares case-insensitively, `NL/TNM` and `nl/tnm` are the same party.
///
/// ```
/// use ocpi_kit::types::PartyRef;
///
/// let a: PartyRef = "NL/TNM".parse().unwrap();
/// let b = PartyRef::new("nl", "tnm").unwrap();
/// assert_eq!(a, b);
/// assert_eq!(a.to_string(), "NL/TNM");
/// ```
///
/// Spec: 2.3.0 §transport_and_format_message_routing
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PartyRef {
    /// ISO-3166 alpha-2 country code of the party.
    pub country_code: CountryCode,
    /// The party's ID.
    pub party_id: PartyId,
}

impl PartyRef {
    /// Creates a party reference.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidString`] if either part is not printable ASCII of the right length.
    pub fn new(country_code: impl Into<String>, party_id: impl Into<String>) -> Result<Self, InvalidString> {
        Ok(Self { country_code: CiString::new(country_code)?, party_id: CiString::new(party_id)? })
    }

    /// The five-character concatenation used by `Credentials.hub_party_id`.
    ///
    /// Spec: 2.3.0 §credentials_credentials_object — *"The two-letter country code and
    /// three-character party ID are concatenated together in this field as one five-character
    /// string."*
    #[must_use]
    pub fn to_hub_party_id(&self) -> CiString<5> {
        CiString::new_lenient(format!("{}{}", self.country_code, self.party_id))
    }

    /// Splits a five-character `hub_party_id` back into its country code and party ID.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidString`] if the value is not exactly five printable ASCII characters.
    pub fn from_hub_party_id(value: &CiString<5>) -> Result<Self, InvalidString> {
        let text = value.as_str();
        if text.len() != 5 {
            return Err(InvalidString::wrong_length(text.len(), 5, super::text::StringKind::Ci));
        }
        Self::new(&text[..2], &text[2..])
    }
}

impl fmt::Display for PartyRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.country_code, self.party_id)
    }
}

impl FromStr for PartyRef {
    type Err = InvalidPartyRef;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (country, party) = s
            .split_once(['/', '*'])
            .ok_or_else(|| InvalidPartyRef(format!("{s:?} is not \"<country>/<party>\"")))?;
        Self::new(country, party).map_err(|e| InvalidPartyRef(e.to_string()))
    }
}

impl Validate for PartyRef {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, country_code, party_id);
    }
}

/// Why a string is not a `country_code`/`party_id` pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidPartyRef(String);

impl fmt::Display for InvalidPartyRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid party reference: {}", self.0)
    }
}
impl std::error::Error for InvalidPartyRef {}

/// The parts of an EVSE ID that follows the eMI3/IDACS format.
///
/// > *Compliant with the following specification for EVSE ID: "E-mobility ID-codes: the purpose
/// > of IDs, ID usage and ID format".*
///
/// The format is `<country code><spot operator><'E'><power outlet id>`, with `*` optionally
/// separating the parts: `NL*TNM*E1234` and `NLTNME1234` are the same EVSE.
///
/// The `evse_id` field is only *recommended* to follow this format, so parsing is a query, never
/// a requirement: [`EvseIdParts::parse`] returns `None` for an ID in any other shape and the
/// crate carries on.
///
/// ```
/// use ocpi_kit::types::EvseIdParts;
///
/// let parts = EvseIdParts::parse("NL*TNM*E1234").unwrap();
/// assert_eq!(parts.country_code, "NL");
/// assert_eq!(parts.spot_operator, "TNM");
/// assert_eq!(parts.power_outlet_id, "1234");
/// assert_eq!(EvseIdParts::parse("NLTNME1234").unwrap(), parts);
/// assert!(EvseIdParts::parse("some-internal-id").is_none());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvseIdParts {
    /// The two-letter country code of the spot operator.
    pub country_code: String,
    /// The three-character spot operator ID.
    pub spot_operator: String,
    /// The power outlet ID: the part after the `E` type marker.
    pub power_outlet_id: String,
}

impl EvseIdParts {
    /// Parses an eMI3/IDACS EVSE ID, or returns `None` if `id` is in another shape.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        let stripped: String = id.chars().filter(|c| *c != '*').collect();
        // <2 country><3 operator><'E'><1+ outlet>
        if stripped.len() < 7 {
            return None;
        }
        let bytes = stripped.as_bytes();
        if !bytes[..5].iter().all(u8::is_ascii_alphanumeric) {
            return None;
        }
        if !bytes[..2].iter().all(u8::is_ascii_alphabetic) {
            return None;
        }
        if !bytes[5].eq_ignore_ascii_case(&b'E') {
            return None;
        }
        let outlet = &stripped[6..];
        if outlet.is_empty() || !outlet.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'*' || b == b'-') {
            return None;
        }
        Some(Self {
            country_code: stripped[..2].to_owned(),
            spot_operator: stripped[2..5].to_owned(),
            power_outlet_id: outlet.to_owned(),
        })
    }

    /// The party that operates this EVSE, according to the ID.
    ///
    /// The spec warns that this need not be the OCPI `party_id` that pushed the object:
    /// *"A party implementing OCPI MAY push EVSE IDs with an eMI3/IDACS spot operator different
    /// from the OCPI party_id."*
    ///
    /// # Errors
    ///
    /// Returns [`InvalidString`] if the parts are not valid `CiString`s, which cannot happen for
    /// a value that came out of [`EvseIdParts::parse`].
    pub fn party(&self) -> Result<PartyRef, InvalidString> {
        PartyRef::new(self.country_code.clone(), self.spot_operator.clone())
    }

    /// Renders the ID in the separated form, `NL*TNM*E1234`.
    #[must_use]
    pub fn to_separated(&self) -> String {
        format!("{}*{}*E{}", self.country_code, self.spot_operator, self.power_outlet_id)
    }
}

/// The parts of an eMI3/IDACS **Contract ID** (eMAID), when it follows the recommended format.
///
/// > *Recommended to follow the specification for eMA ID from "E-mobility ID-codes: the purpose
/// > of IDs, ID usage and ID format".*
///
/// The format is `<country><provider><instance>[<check>]`: two letters, three alphanumerics, nine
/// alphanumerics, and an optional check character. A hyphen may separate all three positions —
/// *"if the hyphenated representation is chosen, the separators must be set at all three
/// places"* — and is for human reading only; the IDACS white paper advises against sending it
/// between systems.
///
/// **This is what makes a whitelist match.** `contract_id` is a [`CiString`], so case already does
/// not matter, but `DE-8AA-CA2B3C4D5-N` and `DE8AACA2B3C4D5N` are the same contract written two
/// ways, and comparing the strings says they are not. [`normalise`](Self::normalise) folds both to
/// one key.
///
/// Like [`EvseIdParts`], parsing is a query rather than a requirement: the format is *recommended*
/// by OCPI, not mandated, so an id in any other shape returns `None` and the crate carries on.
///
/// # What this cannot tell you
///
/// The format has no marker to match on — it is *"two letters, then twelve or thirteen
/// alphanumerics"* — so any id of that shape parses, including one that is not an eMAID at all.
/// `some-internal-id` is fourteen characters once the hyphens go, and comes back as a contract in
/// `SO` issued by provider `MEI`.
///
/// That costs a whitelist nothing, because [`normalise`](Self::normalise) is a *function*: the
/// same id always folds to the same key, whether or not it was really an eMAID. It does mean
/// [`party`](Self::party) is only meaningful for an id you already know follows the format. The
/// instance conventionally begins with `C` — *"strongly recommended to use the type-ID C as first
/// character"* — which is a useful signal, but a recommendation is not something this crate will
/// reject a conformant id over.
///
/// ```
/// use ocpi_kit::types::ContractIdParts;
///
/// let parts = ContractIdParts::parse("DE-8AA-CA2B3C4D5-N").unwrap();
/// assert_eq!(parts.country_code, "DE");
/// assert_eq!(parts.provider_id, "8AA");
/// assert_eq!(parts.instance, "CA2B3C4D5");
/// assert_eq!(parts.check_digit, Some('N'));
///
/// // The same contract, written three ways, folds to one key.
/// let key = ContractIdParts::normalise("DE-8AA-CA2B3C4D5-N").unwrap();
/// assert_eq!(key, "DE8AACA2B3C4D5N");
/// assert_eq!(ContractIdParts::normalise("de8aaca2b3c4d5n").unwrap(), key);
/// assert_eq!(ContractIdParts::normalise("DE*8AA*CA2B3C4D5*N").unwrap(), key);
///
/// // Anything not of that shape is `None`, and so is anything too short or too long.
/// assert!(ContractIdParts::parse("DE8AA").is_none());
/// assert!(ContractIdParts::parse("12-8AA-CA2B3C4D5-N").is_none()); // country is not letters
/// ```
///
/// Spec: 2.3.0 §mod_cdrs_cdr_token_class, §mod_tokens_token_object
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractIdParts {
    /// The two-letter ISO 3166-1 alpha-2 country code of the provider.
    pub country_code: String,
    /// The three-character provider ID, assigned by the eMI3 group.
    pub provider_id: String,
    /// The nine-character instance, whose first character is conventionally `C`.
    pub instance: String,
    /// The check character, which the format marks optional.
    pub check_digit: Option<char>,
}

impl ContractIdParts {
    /// Parses an eMI3/IDACS contract ID, or returns `None` if `id` is in another shape.
    ///
    /// Both separators seen in the field are accepted — `-`, which the contract-ID format
    /// specifies, and `*`, which the EVSE-ID format uses and which some platforms carry over.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        let stripped: String = id.chars().filter(|c| *c != '-' && *c != '*').collect();
        // <2 country><3 provider><9 instance>[<1 check>]
        if !(stripped.len() == 14 || stripped.len() == 15) {
            return None;
        }
        if !stripped.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        if !stripped.as_bytes()[..2].iter().all(u8::is_ascii_alphabetic) {
            return None;
        }
        Some(Self {
            country_code: stripped[..2].to_owned(),
            provider_id: stripped[2..5].to_owned(),
            instance: stripped[5..14].to_owned(),
            check_digit: stripped[14..].chars().next(),
        })
    }

    /// The one key two spellings of the same contract share: upper case, no separators.
    ///
    /// This is the form to key a whitelist on. Returns `None` for an id that does not follow the
    /// format, which a caller should treat as "not comparable" rather than as "no match" — an
    /// eMSP is free to use its own scheme.
    #[must_use]
    pub fn normalise(id: &str) -> Option<String> {
        Self::parse(id).map(|p| p.to_compact())
    }

    /// The provider that issued this contract, according to the ID.
    ///
    /// As with [`EvseIdParts::party`], this need not be the OCPI `party_id` that pushed the
    /// object: *"The `party_id` and `country_code` given here have no direct link with the
    /// eMI3/IDACS format EVSE IDs and Contract IDs."*
    ///
    /// # Errors
    ///
    /// Returns [`InvalidString`] if the parts are not valid `CiString`s, which cannot happen for
    /// a value that came out of [`ContractIdParts::parse`].
    pub fn party(&self) -> Result<PartyRef, InvalidString> {
        PartyRef::new(self.country_code.clone(), self.provider_id.clone())
    }

    /// The ID with no separators and in upper case: `DE8AACA2B3C4D5N`.
    ///
    /// > *Companies are strongly advised NOT to use the optional separators between IT systems as
    /// > they are meant for visibility only.*
    #[must_use]
    pub fn to_compact(&self) -> String {
        let mut out = String::with_capacity(15);
        out.push_str(&self.country_code);
        out.push_str(&self.provider_id);
        out.push_str(&self.instance);
        if let Some(check) = self.check_digit {
            out.push(check);
        }
        out.make_ascii_uppercase();
        out
    }

    /// The ID in the hyphenated form a human reads: `DE-8AA-CA2B3C4D5-N`.
    #[must_use]
    pub fn to_separated(&self) -> String {
        let compact = self.to_compact();
        let mut out = format!("{}-{}-{}", &compact[..2], &compact[2..5], &compact[5..14]);
        if let Some(check) = compact[14..].chars().next() {
            out.push('-');
            out.push(check);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hub_party_id_of_the_wrong_length_says_so() {
        let round_trip = PartyRef::new("NL", "TNM").unwrap();
        assert_eq!(round_trip.to_hub_party_id().as_str(), "NLTNM");
        assert_eq!(PartyRef::from_hub_party_id(&round_trip.to_hub_party_id()).unwrap(), round_trip);

        // A peer that sends a short value gets a message about the length it should have been,
        // not one claiming a two-character string is too long for five.
        let short = CiString::<5>::new_lenient("NL");
        let error = PartyRef::from_hub_party_id(&short).unwrap_err();
        assert!(error.to_string().contains("exactly 5 characters"), "{error}");
        assert!(!error.is_too_long());
    }

    #[test]
    fn party_refs_compare_case_insensitively() {
        assert_eq!(PartyRef::new("NL", "TNM").unwrap(), PartyRef::new("nl", "tnm").unwrap());
        assert_eq!("NL/TNM".parse::<PartyRef>().unwrap(), PartyRef::new("NL", "TNM").unwrap());
        assert!("NLTNM".parse::<PartyRef>().is_err());
    }

    #[test]
    fn hub_party_id_is_the_concatenation() {
        let p = PartyRef::new("NL", "TNM").unwrap();
        let hub = p.to_hub_party_id();
        assert_eq!(hub.as_str(), "NLTNM");
        assert_eq!(PartyRef::from_hub_party_id(&hub).unwrap(), p);
    }

    #[test]
    fn evse_id_parsing_accepts_both_forms_and_declines_others() {
        let sep = EvseIdParts::parse("NL*TNM*E1234").unwrap();
        assert_eq!(sep.to_separated(), "NL*TNM*E1234");
        assert_eq!(EvseIdParts::parse("NLTNME1234").unwrap(), sep);
        assert_eq!(sep.party().unwrap(), PartyRef::new("NL", "TNM").unwrap());
        for other in ["", "short", "12*TNM*E1", "NL*TNM*X1234"] {
            assert!(EvseIdParts::parse(other).is_none(), "{other} should not parse");
        }
    }

    #[test]
    fn country_code_shape_check_is_advisory() {
        assert!(CountryCode::new("NL").unwrap().is_iso_shaped());
        assert!(!CountryCode::new("N1").unwrap().is_iso_shaped());
    }
}
