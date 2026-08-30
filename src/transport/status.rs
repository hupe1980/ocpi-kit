//! OCPI status codes: the four-digit codes that live inside a 200 OK.
//!
//! > *The transport layer ends after a message is correctly parsed into a (semantically
//! > unvalidated) JSON structure. … If a request is syntactically valid JSON and addresses an
//! > existing resource, and comes from a sender that is successfully authenticated and
//! > authorized, this request is supposed to have reached the OCPI layer. To such a request, an
//! > HTTP error status code MUST NOT be returned.*
//!
//! That sentence is the single most misunderstood rule in OCPI. A server that answers a
//! semantically invalid Location with `HTTP 422` is not doing OCPI. [`StatusCode`] plus
//! [`OcpiError::http_status`](super::OcpiError::http_status) encode the rule so it is hard to
//! get wrong.
//!
//! Spec: 2.3.0 §status_codes_status_codes

use core::fmt;

use serde::{Deserialize, Serialize};

/// A four-digit OCPI status code.
///
/// Modelled as a newtype over `u16` rather than an enum because the spec reserves custom ranges
/// (`19xx`, `29xx`, `39xx`, `49xx`) that a party may define values in, and because a peer may
/// send a code from a newer version.
///
/// ```
/// use ocpi_kit::transport::{StatusCode, StatusClass};
///
/// assert_eq!(StatusCode::SUCCESS.class(), StatusClass::Success);
/// assert_eq!(StatusCode::INVALID_PARAMETERS.get(), 2001);
/// assert!(StatusCode::new(2901).is_custom());
/// ```
///
/// Spec: 2.3.0 §status_codes_status_codes
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StatusCode(u16);

impl StatusCode {
    // --- 1xxx: Success -----------------------------------------------------------------------
    /// `1000` — Generic success code.
    pub const SUCCESS: Self = Self(1000);

    // --- 2xxx: Client errors -----------------------------------------------------------------
    /// `2000` — Generic client error.
    pub const CLIENT_ERROR: Self = Self(2000);
    /// `2001` — Invalid or missing parameters, e.g. a missing `last_updated` in a PATCH.
    pub const INVALID_PARAMETERS: Self = Self(2001);
    /// `2002` — Not enough information, e.g. an authorization request with too little information.
    pub const NOT_ENOUGH_INFORMATION: Self = Self(2002);
    /// `2003` — Unknown Location, e.g. a `START_SESSION` with an unknown location.
    pub const UNKNOWN_LOCATION: Self = Self(2003);
    /// `2004` — Unknown Token, e.g. a real-time authorization of an unknown Token.
    pub const UNKNOWN_TOKEN: Self = Self(2004);

    // --- 3xxx: Server errors -----------------------------------------------------------------
    /// `3000` — Generic server error.
    pub const SERVER_ERROR: Self = Self(3000);
    /// `3001` — Unable to use the client's API, e.g. a failed call-back during registration.
    pub const UNABLE_TO_USE_CLIENT_API: Self = Self(3001);
    /// `3002` — Unsupported version.
    pub const UNSUPPORTED_VERSION: Self = Self(3002);
    /// `3003` — No matching endpoints or expected endpoints missing between parties.
    pub const NO_MATCHING_ENDPOINTS: Self = Self(3003);

    // --- 4xxx: Hub errors --------------------------------------------------------------------
    /// `4000` — Generic hub error.
    pub const HUB_ERROR: Self = Self(4000);
    /// `4001` — Unknown receiver: the `OCPI-to-*` address is unknown.
    pub const UNKNOWN_RECEIVER: Self = Self(4001);
    /// `4002` — Timeout on a forwarded request.
    pub const TIMEOUT_ON_FORWARDED_REQUEST: Self = Self(4002);
    /// `4003` — Connection problem: the receiving party is not connected.
    pub const CONNECTION_PROBLEM: Self = Self(4003);

    /// Wraps a raw code.
    #[must_use]
    pub const fn new(code: u16) -> Self {
        Self(code)
    }

    /// The raw code.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Which of the four ranges this code falls in.
    #[must_use]
    pub const fn class(self) -> StatusClass {
        match self.0 {
            1000..=1999 => StatusClass::Success,
            2000..=2999 => StatusClass::ClientError,
            3000..=3999 => StatusClass::ServerError,
            4000..=4999 => StatusClass::HubError,
            _ => StatusClass::Unknown,
        }
    }

    /// Whether this is a success code, so `data` carries the documented payload.
    ///
    /// > *When the status code is in the success range (1xxx), the `data` field in the response
    /// > message SHOULD contain the information as specified in the protocol. Otherwise the
    /// > `data` field is unspecified.*
    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self.class(), StatusClass::Success)
    }

    /// Whether the code falls in one of the reserved custom sub-ranges (`x900`–`x999`).
    ///
    /// > *Custom status code range values SHALL NOT be used by standard OCPI module … When custom
    /// > status codes are used, keep in mind that different custom modules could use the same
    /// > values with a different meaning, as they are not standardized.*
    #[must_use]
    pub const fn is_custom(self) -> bool {
        !matches!(self.class(), StatusClass::Unknown) && self.0 % 1000 >= 900
    }

    /// The description the specification gives for this code, when it defines one.
    #[must_use]
    pub const fn description(self) -> Option<&'static str> {
        Some(match self.0 {
            1000 => "Generic success code",
            2000 => "Generic client error",
            2001 => "Invalid or missing parameters",
            2002 => "Not enough information",
            2003 => "Unknown Location",
            2004 => "Unknown Token",
            3000 => "Generic server error",
            3001 => "Unable to use the client's API",
            3002 => "Unsupported version",
            3003 => "No matching endpoints or expected endpoints missing between parties",
            4000 => "Generic error",
            4001 => "Unknown receiver (TO address is unknown)",
            4002 => "Timeout on forwarded request",
            4003 => "Connection problem (receiving party is not connected)",
            _ => return None,
        })
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.description() {
            Some(d) => write!(f, "{} ({d})", self.0),
            None => write!(f, "{}", self.0),
        }
    }
}

impl fmt::Debug for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StatusCode({self})")
    }
}

impl From<u16> for StatusCode {
    fn from(value: u16) -> Self {
        Self(value)
    }
}
impl From<StatusCode> for u16 {
    fn from(value: StatusCode) -> Self {
        value.0
    }
}

/// The four status code ranges the specification defines.
///
/// Spec: 2.3.0 §status_codes_status_codes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StatusClass {
    /// `1xxx` — the request was handled as documented.
    Success,
    /// `2xxx` — the data sent by the client cannot be processed by the server.
    ClientError,
    /// `3xxx` — the server encountered an internal error.
    ServerError,
    /// `4xxx` — a hub failed to route the message.
    HubError,
    /// A code outside `1000`–`4999`, which the specification does not define.
    Unknown,
}

impl StatusClass {
    /// Whether this class means the request succeeded.
    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }

    /// Whether the fault lies with the party that sent the request.
    #[must_use]
    pub const fn is_client_fault(self) -> bool {
        matches!(self, Self::ClientError)
    }
}

impl fmt::Display for StatusClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Success => "success",
            Self::ClientError => "client error",
            Self::ServerError => "server error",
            Self::HubError => "hub error",
            Self::Unknown => "unknown status class",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_follow_the_ranges() {
        assert_eq!(StatusCode::new(1000).class(), StatusClass::Success);
        assert_eq!(StatusCode::new(1999).class(), StatusClass::Success);
        assert_eq!(StatusCode::new(2001).class(), StatusClass::ClientError);
        assert_eq!(StatusCode::new(3002).class(), StatusClass::ServerError);
        assert_eq!(StatusCode::new(4003).class(), StatusClass::HubError);
        assert_eq!(StatusCode::new(5000).class(), StatusClass::Unknown);
        assert_eq!(StatusCode::new(999).class(), StatusClass::Unknown);
    }

    #[test]
    fn custom_ranges_are_recognised() {
        for code in [1900, 1999, 2900, 3950, 4999] {
            assert!(StatusCode::new(code).is_custom(), "{code} is in a reserved custom range");
        }
        for code in [1000, 2001, 3003, 4000, 2899] {
            assert!(!StatusCode::new(code).is_custom(), "{code} is a standard code");
        }
        assert!(!StatusCode::new(5900).is_custom(), "outside every defined class");
    }

    #[test]
    fn serialises_as_a_bare_number() {
        assert_eq!(serde_json::to_string(&StatusCode::SUCCESS).unwrap(), "1000");
        let parsed: StatusCode = serde_json::from_str("2001").unwrap();
        assert_eq!(parsed, StatusCode::INVALID_PARAMETERS);
    }

    #[test]
    fn display_includes_the_spec_description() {
        assert_eq!(StatusCode::INVALID_PARAMETERS.to_string(), "2001 (Invalid or missing parameters)");
        assert_eq!(StatusCode::new(2901).to_string(), "2901");
    }
}
