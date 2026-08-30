//! The *Commands* module of OCPI 2.1.1.
//!
//! # The single most important difference
//!
//! OCPI 2.1.1 has **no `CommandResult` object**. The Charge Point's eventual answer is POSTed to
//! the `response_url` as another [`CommandResponse`], whose [`CommandResponseType`] therefore
//! carries `TIMEOUT` — a value that makes no sense in a synchronous reply and which OCPI 2.2
//! moved to the separate `CommandResultType`.
//!
//! An integration that treats the 2.1.1 callback as a 2.2 `CommandResult` will fail to decode
//! every one of them, which is why the two are separate types in this crate.
//!
//! There is also no `CANCEL_RESERVATION` command, and `ReserveNow.reservation_id` is an **`int`**
//! rather than a string.
//!
//! Spec: 2.1.1 §mod_commands

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_lenient_enum;
use crate::types::validate_fields;
use crate::types::{DateTime, Extensions, OcpiString, Url, Validate, Validator};

use super::tokens::Token;

/// The answer to a command request — used **both** synchronously and on the `response_url`.
///
/// Spec: 2.1.1 §mod_commands_commandresponse_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CommandResponse {
    /// Result of the command request as sent by the Charge Point to the CPO.
    pub result: CommandResponseType,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl CommandResponse {
    /// Creates a response.
    #[must_use]
    pub fn new(result: CommandResponseType) -> Self {
        Self { result, extensions: Extensions::new() }
    }
}

impl Validate for CommandResponse {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, result);
    }
}

/// A request to start a charging session, in OCPI 2.1.1.
///
/// There is no `connector_id` and no `authorization_reference`; both arrived in OCPI 2.2.
///
/// Spec: 2.1.1 §mod_commands_startsession_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct StartSession {
    /// URL that the [`CommandResponse`] POST should be sent to.
    pub response_url: Url,
    /// The Token the Charge Point has to use to start a new session.
    pub token: Token,
    /// `Location.id` on which a session is to be started.
    pub location_id: OcpiString<39>,
    /// `EVSE.uid` on which a session is to be started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_uid: Option<OcpiString<39>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for StartSession {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, response_url, token, location_id, evse_uid);
    }
}

/// A request to stop an ongoing session, in OCPI 2.1.1.
///
/// Spec: 2.1.1 §mod_commands_stopsession_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct StopSession {
    /// URL that the [`CommandResponse`] POST should be sent to.
    pub response_url: Url,
    /// `Session.id` of the Session that is requested to be stopped.
    pub session_id: OcpiString<36>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for StopSession {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, response_url, session_id);
    }
}

/// A request to reserve an EVSE, in OCPI 2.1.1.
///
/// Note that `reservation_id` is an **integer** here; OCPI 2.2 made it a `CiString(36)`.
///
/// Spec: 2.1.1 §mod_commands_reservenow_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct ReserveNow {
    /// URL that the [`CommandResponse`] POST should be sent to.
    pub response_url: Url,
    /// The Token for which to reserve the Charge Point.
    pub token: Token,
    /// When this reservation ends.
    pub expiry_date: DateTime,
    /// Reservation id, unique for this reservation.
    pub reservation_id: i64,
    /// `Location.id` for which to reserve an EVSE.
    pub location_id: OcpiString<39>,
    /// `EVSE.uid` if a specific EVSE has to be reserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_uid: Option<OcpiString<39>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for ReserveNow {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, response_url, token, expiry_date, location_id, evse_uid);
    }
}

/// A request to unlock a connector, in OCPI 2.1.1.
///
/// Spec: 2.1.1 §mod_commands_unlockconnector_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct UnlockConnector {
    /// URL that the [`CommandResponse`] POST should be sent to.
    pub response_url: Url,
    /// `Location.id` of which it is requested to unlock the connector.
    pub location_id: OcpiString<39>,
    /// `EVSE.uid` of which it is requested to unlock the connector.
    pub evse_uid: OcpiString<39>,
    /// `Connector.id` which it is requested to unlock.
    pub connector_id: OcpiString<36>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for UnlockConnector {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, response_url, location_id, evse_uid, connector_id);
    }
}

/// Every OCPI 2.1.1 command body, tagged by the [`CommandType`] it belongs to.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Command {
    /// `POST {commands_endpoint}/RESERVE_NOW`
    ReserveNow(Box<ReserveNow>),
    /// `POST {commands_endpoint}/START_SESSION`
    StartSession(Box<StartSession>),
    /// `POST {commands_endpoint}/STOP_SESSION`
    StopSession(StopSession),
    /// `POST {commands_endpoint}/UNLOCK_CONNECTOR`
    UnlockConnector(UnlockConnector),
}

impl Command {
    /// Which command this is.
    #[must_use]
    pub fn command_type(&self) -> CommandType {
        match self {
            Self::ReserveNow(_) => CommandType::ReserveNow,
            Self::StartSession(_) => CommandType::StartSession,
            Self::StopSession(_) => CommandType::StopSession,
            Self::UnlockConnector(_) => CommandType::UnlockConnector,
        }
    }

    /// The URL the eventual [`CommandResponse`] must be POSTed to.
    #[must_use]
    pub fn response_url(&self) -> &Url {
        match self {
            Self::ReserveNow(c) => &c.response_url,
            Self::StartSession(c) => &c.response_url,
            Self::StopSession(c) => &c.response_url,
            Self::UnlockConnector(c) => &c.response_url,
        }
    }
}

impl Validate for Command {
    fn validate_in(&self, v: &mut Validator) {
        match self {
            Self::ReserveNow(c) => c.validate_in(v),
            Self::StartSession(c) => c.validate_in(v),
            Self::StopSession(c) => c.validate_in(v),
            Self::UnlockConnector(c) => c.validate_in(v),
        }
    }
}

ocpi_lenient_enum! {
    /// The answer to a command request, in OCPI 2.1.1.
    ///
    /// `TIMEOUT` belongs here because 2.1.1 has no separate result object: the asynchronous
    /// callback reuses this enum.
    ///
    /// Spec: 2.1.1 §mod_commands_commandresponsetype_enum
    pub enum CommandResponseType {
        /// The requested command is not supported by this CPO, Charge Point or EVSE.
        NotSupported = "NOT_SUPPORTED",
        /// Rejected by the CPO or the Charge Point.
        Rejected = "REJECTED",
        /// Accepted.
        Accepted = "ACCEPTED",
        /// No response from the Charge Point in a reasonable time.
        Timeout = "TIMEOUT",
        /// The Session in the requested command is not known.
        UnknownSession = "UNKNOWN_SESSION",
    }
}

ocpi_lenient_enum! {
    /// The command being requested, in OCPI 2.1.1.
    ///
    /// `CANCEL_RESERVATION` arrived in OCPI 2.2.
    ///
    /// Spec: 2.1.1 §mod_commands_commandtype_enum
    pub enum CommandType {
        /// Reserve a (specific) EVSE for a Token, starting now.
        ReserveNow = "RESERVE_NOW",
        /// Start a transaction on the given EVSE.
        StartSession = "START_SESSION",
        /// Stop an ongoing session.
        StopSession = "STOP_SESSION",
        /// Unlock the connector. Help desk operators only.
        UnlockConnector = "UNLOCK_CONNECTOR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_is_a_response_type_here_because_there_is_no_result_object() {
        let response: CommandResponse = serde_json::from_str(r#"{"result":"TIMEOUT"}"#).unwrap();
        assert_eq!(response.result, CommandResponseType::Timeout);
        // In OCPI 2.2 and later, TIMEOUT is a CommandResultType and not a CommandResponseType.
        assert!("TIMEOUT".parse::<crate::v2_3_0::commands::CommandResponseType>().is_err());
        assert!("TIMEOUT".parse::<crate::v2_3_0::commands::CommandResultType>().is_ok());
    }

    #[test]
    fn cancel_reservation_arrived_after_2_1_1() {
        assert_eq!(CommandType::ALL_KNOWN.len(), 4);
        assert!(!CommandType::from("CANCEL_RESERVATION").is_known());
    }

    #[test]
    fn the_reservation_id_is_an_integer() {
        #[derive(serde::Deserialize)]
        struct OnlyId {
            reservation_id: i64,
        }
        let json = r#"{"reservation_id":42}"#;
        let parsed: OnlyId = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.reservation_id, 42);
    }
}
