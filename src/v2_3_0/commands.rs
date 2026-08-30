//! The *Commands* module of OCPI 2.3.0: start, stop, reserve, cancel and unlock.
//!
//! *Module Identifier: `commands`*
//!
//! Commands are the one place in OCPI with an asynchronous callback: the Receiver answers the
//! POST immediately with a [`CommandResponse`] carrying a `timeout`, and later POSTs a
//! [`CommandResult`] to the `response_url` the Sender supplied.
//!
//! Spec: 2.3.0 §mod_commands_commands_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_enum;
use crate::ocpi_open_enum;
use crate::types::validate_fields;
use crate::types::{CiString, DateTime, DisplayText, Extensions, Url, Validate, Validator, ViolationCode};

use super::tokens::Token;

/// A request to start a charging session on a Location, EVSE or Connector.
///
/// > *The Token provided by the eMSP for the `StartSession` SHALL be authorized by the eMSP
/// > before sending it to the CPO. Therefore the CPO SHALL NOT check the validity of the Token
/// > provided before sending the request to the Charge Point.*
///
/// Spec: 2.3.0 §mod_commands_startsession_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct StartSession {
    /// URL that the [`CommandResult`] POST should be sent to.
    ///
    /// > *This URL might contain a unique ID to be able to distinguish between StartSession
    /// > requests.*
    pub response_url: Url,
    /// The Token the Charge Point has to use to start a new session.
    pub token: Token,
    /// `Location.id` on which a session is to be started.
    pub location_id: CiString<36>,
    /// `EVSE.uid` on which a session is to be started. Required when `connector_id` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_uid: Option<CiString<36>>,
    /// `Connector.id` on which a session is to be started.
    ///
    /// > *This field is required when the capability `START_SESSION_CONNECTOR_REQUIRED` is set on
    /// > the EVSE.*
    ///
    /// See [`Evse::requires_connector_id_on_start`](crate::v2_3_0::locations::Evse::requires_connector_id_on_start).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<CiString<36>>,
    /// Reference to the authorization given by the eMSP, echoed in the Session and CDR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_reference: Option<CiString<36>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for StartSession {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            response_url,
            token,
            location_id,
            evse_uid,
            connector_id,
            authorization_reference,
        );
        // "Required when `connector_id` is set."
        if self.connector_id.is_some() && self.evse_uid.is_none() {
            v.report_at(
                "evse_uid",
                ViolationCode::MissingConditional,
                "is required when `connector_id` is set",
            );
        }
    }
}

/// A request to stop an ongoing session.
///
/// Spec: 2.3.0 §mod_commands_stopsession_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct StopSession {
    /// URL that the [`CommandResult`] POST should be sent to.
    pub response_url: Url,
    /// `Session.id` of the Session that is requested to be stopped.
    pub session_id: CiString<36>,
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

/// A request to reserve an EVSE for a Token for a certain time, starting now.
///
/// > *A successful reservation will result in a new `Session` object being created by the CPO.
/// > An unused Reservation of a Charge Point/EVSE MAY result in cost being made, thus also a
/// > CDR.*
///
/// Spec: 2.3.0 §mod_commands_reservenow_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct ReserveNow {
    /// URL that the [`CommandResult`] POST should be sent to.
    pub response_url: Url,
    /// The Token for which to reserve the Charge Point (and specific EVSE).
    pub token: Token,
    /// When this reservation ends, in UTC.
    pub expiry_date: DateTime,
    /// Reservation id, unique for this reservation.
    ///
    /// > *The `reservation_id` sent by the Sender (eMSP) to the Receiver (CPO) SHALL NOT be sent
    /// > directly to a Charge Point. The CPO SHALL make sure the Reservation ID sent to the
    /// > Charge Point is unique and is not used by another Sender.*
    pub reservation_id: CiString<36>,
    /// `Location.id` for which to reserve an EVSE.
    pub location_id: CiString<36>,
    /// `EVSE.uid` if a specific EVSE has to be reserved.
    ///
    /// > *If no EVSE is specified, the Charge Point should keep one EVSE available for the EV
    /// > Driver identified by the given Token.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_uid: Option<CiString<36>>,
    /// Reference to the authorization given by the eMSP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_reference: Option<CiString<36>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for ReserveNow {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            response_url,
            token,
            expiry_date,
            reservation_id,
            location_id,
            evse_uid,
            authorization_reference,
        );
    }
}

/// A request to cancel an existing reservation.
///
/// > *As there might be cost involved for a Reservation, canceling a reservation might still
/// > result in a CDR being sent for the reservation.*
///
/// Spec: 2.3.0 §mod_commands_cancelreservation_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct CancelReservation {
    /// URL that the [`CommandResult`] POST should be sent to.
    pub response_url: Url,
    /// The `reservation_id` that was given to the [`ReserveNow`].
    pub reservation_id: CiString<36>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for CancelReservation {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, response_url, reservation_id);
    }
}

/// A request to unlock a connector.
///
/// > *This functionality is for help desk operators only! … This command SHALL never be allowed
/// > to be sent directly by the EV-Driver.*
///
/// Spec: 2.3.0 §mod_commands_unlockconnector_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct UnlockConnector {
    /// URL that the [`CommandResult`] POST should be sent to.
    pub response_url: Url,
    /// `Location.id` of which it is requested to unlock the connector.
    pub location_id: CiString<36>,
    /// `EVSE.uid` of which it is requested to unlock the connector.
    pub evse_uid: CiString<36>,
    /// `Connector.id` which it is requested to unlock.
    pub connector_id: CiString<36>,
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

/// The synchronous answer to a command request.
///
/// > *Because OCPI does not allow/require retries, it could happen that the asynchronous result
/// > url given by the eMSP is never successfully called. … it is important for the eMSP to know
/// > the timeout on a certain command.*
///
/// Spec: 2.3.0 §mod_commands_commandresponse_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct CommandResponse {
    /// Response from the CPO on the command request.
    pub result: CommandResponseType,
    /// Timeout for this command in seconds.
    ///
    /// > *When the Result is not received within this timeout, the eMSP can assume that the
    /// > message might never be sent.*
    pub timeout: u32,
    /// Human-readable description of the result, in one or more languages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub message: Vec<DisplayText>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl CommandResponse {
    /// The timeout as a [`std::time::Duration`], for awaiting the asynchronous result.
    #[must_use]
    pub const fn timeout_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout as u64)
    }

    /// Whether a [`CommandResult`] should be expected on the `response_url`.
    ///
    /// Only an `ACCEPTED` command has been forwarded to the Charge Point; the other outcomes are
    /// final already.
    #[must_use]
    pub fn expects_result(&self) -> bool {
        self.result == CommandResponseType::Accepted
    }
}

impl Validate for CommandResponse {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, result, message);
        if self.result == CommandResponseType::Accepted && self.timeout == 0 {
            v.report_at(
                "timeout",
                ViolationCode::OutOfRange,
                "an accepted command needs a non-zero timeout for the eMSP to wait on",
            );
        }
    }
}

/// The asynchronous result, POSTed by the CPO to the `response_url`.
///
/// Spec: 2.3.0 §mod_commands_commandresult_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct CommandResult {
    /// Result of the command request as sent by the Charge Point to the CPO.
    pub result: CommandResultType,
    /// Human-readable description of the reason, in one or more languages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub message: Vec<DisplayText>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for CommandResult {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, result, message);
    }
}

/// Every command body, tagged by the [`CommandType`] it belongs to.
///
/// The wire format is one POST per command with the command name in the URL, so this enum is not
/// itself serialised as a tagged union; it exists so a server handler can take one argument and
/// a client can queue heterogeneous commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Command {
    /// `POST {commands_endpoint}/CANCEL_RESERVATION`
    CancelReservation(CancelReservation),
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
            Self::CancelReservation(_) => CommandType::CancelReservation,
            Self::ReserveNow(_) => CommandType::ReserveNow,
            Self::StartSession(_) => CommandType::StartSession,
            Self::StopSession(_) => CommandType::StopSession,
            Self::UnlockConnector(_) => CommandType::UnlockConnector,
        }
    }

    /// The URL the [`CommandResult`] must be POSTed to.
    #[must_use]
    pub fn response_url(&self) -> &Url {
        match self {
            Self::CancelReservation(c) => &c.response_url,
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
            Self::CancelReservation(c) => c.validate_in(v),
            Self::ReserveNow(c) => c.validate_in(v),
            Self::StartSession(c) => c.validate_in(v),
            Self::StopSession(c) => c.validate_in(v),
            Self::UnlockConnector(c) => c.validate_in(v),
        }
    }
}

ocpi_enum! {
    /// The CPO's immediate answer to a command request.
    ///
    /// Spec: 2.3.0 §mod_commands_commandresponsetype_enum
    pub enum CommandResponseType {
        /// The requested command is not supported by this CPO, Charge Point or EVSE.
        NotSupported = "NOT_SUPPORTED",
        /// Rejected by the CPO; the Session might not be from a customer of the sending eMSP.
        Rejected = "REJECTED",
        /// Accepted by the CPO and forwarded to the EVSE.
        Accepted = "ACCEPTED",
        /// The Session in the requested command is not known by this CPO.
        UnknownSession = "UNKNOWN_SESSION",
    }
}

ocpi_enum! {
    /// The Charge Point's eventual answer, delivered to the `response_url`.
    ///
    /// Kept deliberately distinct from [`CommandResponseType`]: OCPI 2.1.1 had only one enum, and
    /// conflating them is a common source of interoperability bugs.
    ///
    /// Spec: 2.3.0 §mod_commands_commandresulttype_enum
    pub enum CommandResultType {
        /// Accepted by the Charge Point.
        Accepted = "ACCEPTED",
        /// The Reservation has been canceled by the CPO.
        CanceledReservation = "CANCELED_RESERVATION",
        /// The EVSE is currently occupied; another session is ongoing.
        EvseOccupied = "EVSE_OCCUPIED",
        /// The EVSE is currently inoperative or faulted.
        EvseInoperative = "EVSE_INOPERATIVE",
        /// Execution of the command failed at the Charge Point.
        Failed = "FAILED",
        /// The requested command is not supported by this Charge Point or EVSE.
        NotSupported = "NOT_SUPPORTED",
        /// Rejected by the Charge Point.
        Rejected = "REJECTED",
        /// No response received from the Charge Point in a reasonable time.
        Timeout = "TIMEOUT",
        /// The Reservation in the requested command is not known by this Charge Point.
        UnknownReservation = "UNKNOWN_RESERVATION",
    }
}

ocpi_open_enum! {
    /// The command being requested, as it appears in the URL.
    ///
    /// Spec: 2.3.0 §mod_commands_commandtype_enum
    pub enum CommandType {
        /// Cancel a specific reservation.
        CancelReservation = "CANCEL_RESERVATION",
        /// Reserve a (specific) EVSE for a Token, starting now.
        ReserveNow = "RESERVE_NOW",
        /// Start a transaction on the given EVSE/Connector.
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
    use crate::types::Url;

    fn url() -> Url {
        Url::new("https://msp.example.com/ocpi/emsp/2.3.0/commands/START_SESSION/1234").unwrap()
    }

    #[test]
    fn a_connector_id_without_an_evse_uid_is_incomplete() {
        let token = crate::v2_3_0::tokens::Token::builder()
            .country_code("NL")
            .party_id("TNM")
            .uid("012345678")
            .token_type(crate::v2_3_0::tokens::TokenType::AppUser)
            .contract_id("NL-TNM-C12345678-X")
            .issuer("TheNewMotion")
            .valid(true)
            .whitelist(crate::v2_3_0::tokens::WhitelistType::Never)
            .last_updated("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .build();
        let cmd = StartSession::builder()
            .response_url(url())
            .token(token)
            .location_id("LOC1")
            .connector_id("1")
            .build();
        let err = cmd.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].pointer, "/evse_uid");
    }

    #[test]
    fn only_an_accepted_response_promises_a_result() {
        let accepted =
            CommandResponse::builder().result(CommandResponseType::Accepted).timeout(30u32).build();
        assert!(accepted.expects_result());
        assert_eq!(accepted.timeout_duration(), std::time::Duration::from_secs(30));
        assert!(accepted.validate().is_ok());

        let rejected = CommandResponse::builder().result(CommandResponseType::Rejected).timeout(0u32).build();
        assert!(!rejected.expects_result());
        assert!(rejected.validate().is_ok(), "a rejected command needs no timeout");

        let bad = CommandResponse::builder().result(CommandResponseType::Accepted).timeout(0u32).build();
        assert_eq!(bad.validate().unwrap_err().as_slice()[0].pointer, "/timeout");
    }

    #[test]
    fn response_and_result_enums_stay_distinct() {
        assert!("CANCELED_RESERVATION".parse::<CommandResponseType>().is_err());
        assert!("UNKNOWN_SESSION".parse::<CommandResultType>().is_err());
        assert_eq!(
            "CANCELED_RESERVATION".parse::<CommandResultType>().unwrap(),
            CommandResultType::CanceledReservation
        );
    }

    #[test]
    fn the_command_enum_names_its_own_type_and_callback() {
        let cmd = Command::StopSession(StopSession::builder().response_url(url()).session_id("101").build());
        assert_eq!(cmd.command_type(), CommandType::StopSession);
        assert_eq!(cmd.response_url(), &url());
    }
}
