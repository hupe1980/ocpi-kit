//! The *Commands* module of OCPI 2.2.1, as a delta from
//! [`v2_3_0::commands`](crate::v2_3_0::commands).
//!
//! Only [`StartSession`] and [`ReserveNow`] are redefined, because they carry a
//! [`Token`], whose `type` differs between the versions. Everything else —
//! the response and result objects, all three enums, `StopSession`, `CancelReservation`,
//! `UnlockConnector` — is wire-identical.
//!
//! Spec: 2.2.1 §mod_commands_commands_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::types::validate_fields;
use crate::types::{CiString, DateTime, Extensions, Url, Validate, Validator, ViolationCode};

use super::tokens::Token;

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::commands::{
    CancelReservation, CommandResponse, CommandResponseType, CommandResult, CommandResultType, CommandType,
    StopSession, UnlockConnector,
};

/// A request to start a charging session, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_commands_startsession_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct StartSession {
    /// URL that the [`CommandResult`] POST should be sent to.
    pub response_url: Url,
    /// The Token the Charge Point has to use to start a new session.
    pub token: Token,
    /// `Location.id` on which a session is to be started.
    pub location_id: CiString<36>,
    /// `EVSE.uid` on which a session is to be started. Required when `connector_id` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_uid: Option<CiString<36>>,
    /// `Connector.id` on which a session is to be started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<CiString<36>>,
    /// Reference to the authorization given by the eMSP.
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
        if self.connector_id.is_some() && self.evse_uid.is_none() {
            v.report_at(
                "evse_uid",
                ViolationCode::MissingConditional,
                "is required when `connector_id` is set",
            );
        }
    }
}

/// A request to reserve an EVSE for a Token, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_commands_reservenow_object
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
    pub reservation_id: CiString<36>,
    /// `Location.id` for which to reserve an EVSE.
    pub location_id: CiString<36>,
    /// `EVSE.uid` if a specific EVSE has to be reserved.
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

/// Every OCPI 2.2.1 command body, tagged by the [`CommandType`] it belongs to.
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
