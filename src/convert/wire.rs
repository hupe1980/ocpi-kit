//! Translating a JSON document from one OCPI version to another.
//!
//! [`Upgrade`] and [`Downgrade`] work on typed objects. A hub, a client talking to a peer on an
//! older version and a server answering one face the problem one step earlier: they hold *bytes*
//! and know the endpoint those bytes came from, not the Rust type they will become.
//!
//! [`ObjectKind`] names the objects whose wire format changed between OCPI 2.2.1 and 2.3.0, says
//! which one an endpoint carries, and translates a [`serde_json::Value`] — one object or a whole
//! page — keeping the [`Lossy`] report.
//!
//! ```
//! use ocpi_kit::convert::wire::{ObjectKind, Payload};
//! use ocpi_kit::{InterfaceRole, ModuleId};
//!
//! // On a Locations Sender interface, `/{location_id}/{evse_uid}` is an EVSE.
//! let kind = ObjectKind::for_endpoint(
//!     &ModuleId::Locations,
//!     InterfaceRole::Sender,
//!     "LOC1/3256",
//!     Payload::Response,
//! );
//! assert_eq!(kind, Some(ObjectKind::Evse));
//! ```
//!
//! Only the 2.2.1 ↔ 2.3.0 crossing exists. OCPI 2.1.1 is modelled and deliberately not bridged:
//! it has no owner fields on objects, no routing and no `Price`, so carrying an object across that
//! boundary is a decision about a deployment rather than a translation. [`bridgeable`] is the
//! single answer to "can this build make this crossing".

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{InterfaceRole, ModuleId, VersionNumber};

use super::{Converted, Downgrade, Lossy, Upgrade};

/// Which half of an exchange a document is, for the two endpoints whose request and response
/// carry different objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    /// The request body.
    Request,
    /// The `data` of the response envelope.
    Response,
}

/// Why a document could not be carried between two versions.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BridgeError {
    /// This build has no conversions between the two versions. See [`bridgeable`].
    #[error("this build cannot translate OCPI {from} to OCPI {to}")]
    Unsupported {
        /// The version the document is written in.
        from: VersionNumber,
        /// The version it was to be translated to.
        to: VersionNumber,
    },
    /// The document is not the object the endpoint is supposed to carry.
    #[error("the document is not a valid OCPI {version} {kind}: {message}")]
    Decode {
        /// The version the document claimed to be written in.
        version: VersionNumber,
        /// The object it was expected to be.
        kind: ObjectKind,
        /// What `serde` said.
        message: String,
    },
}

/// Whether this build can translate documents between two OCPI versions.
///
/// Today that is exactly the 2.2.1 ↔ 2.3.0 crossing, in both directions, plus the trivial case of
/// a version to itself. It is a function rather than a constant because the answer depends on the
/// Cargo features the crate was built with.
#[must_use]
pub fn bridgeable(from: &VersionNumber, to: &VersionNumber) -> bool {
    if from == to {
        return true;
    }
    matches!(
        (from, to),
        (VersionNumber::V2_2_1, VersionNumber::V2_3_0) | (VersionNumber::V2_3_0, VersionNumber::V2_2_1)
    )
}

/// An OCPI object whose wire format changed between the versions this crate bridges.
///
/// Objects that are byte-identical across versions are deliberately absent: there is nothing to
/// do to them, and [`ObjectKind::for_endpoint`] returns `None` for the endpoints that carry them
/// so a caller can forward the bytes untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ObjectKind {
    /// `Location`, which gained `parking_places` and `help_phone` in 2.3.0.
    Location,
    /// `EVSE`, which gained `parking` and `accepted_service_providers`.
    Evse,
    /// `Connector`, which gained `capabilities`.
    Connector,
    /// `Session`, whose `total_cost` is a `Price`.
    Session,
    /// `CDR`, whose costs are `Price`s and whose `tariffs` are `Tariff`s.
    Cdr,
    /// `Tariff`, which gained `tax_included` and replaced `Price` limits with `PriceLimit`.
    Tariff,
    /// `Token`, whose `TokenType` was opened and gained `EMAID`.
    Token,
    /// `AuthorizationInfo`, which embeds a `Token`.
    AuthorizationInfo,
    /// `Credentials`, which gained `hub_party_id` and lost the `HUB` role.
    Credentials,
    /// `ClientInfo`, which carries a `Role`.
    ClientInfo,
    /// The `START_SESSION` command body, which embeds a `Token`.
    StartSession,
    /// The `RESERVE_NOW` command body, which embeds a `Token`.
    ReserveNow,
}

impl core::fmt::Display for ObjectKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Location => "Location",
            Self::Evse => "EVSE",
            Self::Connector => "Connector",
            Self::Session => "Session",
            Self::Cdr => "CDR",
            Self::Tariff => "Tariff",
            Self::Token => "Token",
            Self::AuthorizationInfo => "AuthorizationInfo",
            Self::Credentials => "Credentials",
            Self::ClientInfo => "ClientInfo",
            Self::StartSession => "StartSession",
            Self::ReserveNow => "ReserveNow",
        })
    }
}

/// Generates the per-kind translation from the typed [`Upgrade`]/[`Downgrade`] impls.
macro_rules! bridge_kinds {
    ($($kind:ident => $old:path, $new:path;)*) => {
        impl ObjectKind {
            fn bridge_one(
                self,
                from: &VersionNumber,
                to: &VersionNumber,
                value: Value,
            ) -> Result<Converted<Value>, BridgeError> {
                use VersionNumber::{V2_2_1, V2_3_0};
                match (from, to) {
                    (V2_2_1, V2_3_0) => match self {
                        $(Self::$kind => up::<$old, $new>(self, value),)*
                    },
                    (V2_3_0, V2_2_1) => match self {
                        $(Self::$kind => down::<$new, $old>(self, value),)*
                    },
                    _ => Err(BridgeError::Unsupported { from: from.clone(), to: to.clone() }),
                }
            }
        }
    };
}

bridge_kinds! {
    Location => crate::v2_2_1::locations::Location, crate::v2_3_0::locations::Location;
    Evse => crate::v2_2_1::locations::Evse, crate::v2_3_0::locations::Evse;
    Connector => crate::v2_2_1::locations::Connector, crate::v2_3_0::locations::Connector;
    Session => crate::v2_2_1::sessions::Session, crate::v2_3_0::sessions::Session;
    Cdr => crate::v2_2_1::cdrs::Cdr, crate::v2_3_0::cdrs::Cdr;
    Tariff => crate::v2_2_1::tariffs::Tariff, crate::v2_3_0::tariffs::Tariff;
    Token => crate::v2_2_1::tokens::Token, crate::v2_3_0::tokens::Token;
    AuthorizationInfo =>
        crate::v2_2_1::tokens::AuthorizationInfo, crate::v2_3_0::tokens::AuthorizationInfo;
    Credentials => crate::v2_2_1::credentials::Credentials, crate::v2_3_0::credentials::Credentials;
    ClientInfo =>
        crate::v2_2_1::hub_client_info::ClientInfo, crate::v2_3_0::hub_client_info::ClientInfo;
    StartSession => crate::v2_2_1::commands::StartSession, crate::v2_3_0::commands::StartSession;
    ReserveNow => crate::v2_2_1::commands::ReserveNow, crate::v2_3_0::commands::ReserveNow;
}

fn up<O, N>(kind: ObjectKind, value: Value) -> Result<Converted<Value>, BridgeError>
where
    O: DeserializeOwned + Upgrade<N>,
    N: Serialize,
{
    let old: O = serde_json::from_value(value).map_err(|e| BridgeError::Decode {
        version: VersionNumber::V2_2_1,
        kind,
        message: e.to_string(),
    })?;
    Ok(reserialise(kind, VersionNumber::V2_3_0, old.upgrade()))
}

fn down<N, O>(kind: ObjectKind, value: Value) -> Result<Converted<Value>, BridgeError>
where
    N: DeserializeOwned + Downgrade<O>,
    O: Serialize,
{
    let new: N = serde_json::from_value(value).map_err(|e| BridgeError::Decode {
        version: VersionNumber::V2_3_0,
        kind,
        message: e.to_string(),
    })?;
    Ok(reserialise(kind, VersionNumber::V2_2_1, new.downgrade()))
}

fn reserialise<T: Serialize>(
    kind: ObjectKind,
    into: VersionNumber,
    converted: Converted<T>,
) -> Converted<Value> {
    // Serialising a wire struct this crate defines cannot fail: every field is a JSON-shaped type
    // and no map has non-string keys. Falling back to `null` rather than unwrapping keeps the
    // whole hub path panic-free even if that ever stopped being true.
    let value = serde_json::to_value(&converted.value).unwrap_or(Value::Null);
    debug_assert!(!value.is_null(), "a bridged {kind} serialised to null on the way to {into}");
    Converted::new(value, converted.lossy)
}

impl ObjectKind {
    /// Translates one object, or a whole page of them, from `from` to `to`.
    ///
    /// An array is translated element by element with each element's losses reported under its own
    /// index (`/17/help_phone`). A `null` — an envelope with no `data` — is returned unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`BridgeError::Unsupported`] when this build has no conversions between the two
    /// versions, and [`BridgeError::Decode`] when the document is not the object the endpoint is
    /// supposed to carry.
    pub fn bridge(
        self,
        from: &VersionNumber,
        to: &VersionNumber,
        value: Value,
    ) -> Result<Converted<Value>, BridgeError> {
        if from == to {
            return Ok(Converted::lossless(value));
        }
        match value {
            Value::Null => Ok(Converted::lossless(Value::Null)),
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                let mut lossy = Lossy::none();
                for (index, item) in items.into_iter().enumerate() {
                    let converted = self.bridge_one(from, to, item)?;
                    lossy.absorb(&format!("/{index}"), converted.lossy);
                    out.push(converted.value);
                }
                Ok(Converted::new(Value::Array(out), lossy))
            }
            other => self.bridge_one(from, to, other),
        }
    }

    /// The top-level fields of this object whose shape or presence differs between the versions.
    ///
    /// Everything else is byte-identical, which is what makes a **merge patch** translatable: a
    /// patch is not an object, so it cannot go through [`bridge`](Self::bridge), but one writing
    /// only fields outside this list means the same thing in both versions and crosses unchanged.
    ///
    /// Checked against the fixture corpus: every 2.2.1 spec example is carried to 2.3.0 and back,
    /// and no field outside its object's list may move.
    #[must_use]
    pub const fn divergent_fields(self) -> &'static [&'static str] {
        match self {
            Self::Location => &["evses", "parking_places", "help_phone"],
            Self::Evse => &["connectors", "parking", "accepted_service_providers"],
            Self::Connector => &["capabilities"],
            Self::Session => &["total_cost"],
            Self::Cdr => &[
                "tariffs",
                "booking_id",
                "total_cost",
                "total_fixed_cost",
                "total_energy_cost",
                "total_time_cost",
                "total_parking_cost",
                "total_reservation_cost",
            ],
            // `preauthorize_amount` is on this list whether or not the `payments` feature is on:
            // a patch that writes it must not cross a version boundary either way, and a build
            // that cannot model the field is in no better position to carry it.
            Self::Tariff => &["min_price", "max_price", "tax_included", "preauthorize_amount"],
            Self::Credentials => &["roles", "hub_party_id"],
            Self::ClientInfo => &["role"],
            // A `TokenType` that 2.2.1 does not know keeps its text in a `Custom` variant, so the
            // string on the wire is the same in both versions and nothing here moves.
            Self::Token | Self::AuthorizationInfo | Self::StartSession | Self::ReserveNow => &[],
        }
    }

    /// Whether a merge patch written against one version means the same thing in the other.
    ///
    /// See [`divergent_fields`](Self::divergent_fields).
    #[must_use]
    pub fn patch_crosses_unchanged(self, fields: &[&str]) -> bool {
        let divergent = self.divergent_fields();
        !fields.iter().any(|f| divergent.contains(f))
    }

    /// The object an endpoint carries, or `None` when it is the same in both versions.
    ///
    /// `path` is what follows the module's endpoint URL, with or without surrounding slashes and
    /// without a query string — `LOC1/3256` on a Locations Sender interface, `NL/TNM/LOC1` on the
    /// Receiver one. `None` means "nothing to do": either the endpoint carries an object that did
    /// not change between the versions, or it carries no object at all.
    ///
    /// Spec: 2.3.0 §mod_locations, §mod_sessions, §mod_cdrs, §mod_tariffs, §mod_tokens,
    /// §mod_commands, §credentials, §mod_hub_client_info
    #[must_use]
    pub fn for_endpoint(
        module: &ModuleId,
        interface: InterfaceRole,
        path: &str,
        payload: Payload,
    ) -> Option<Self> {
        let segments: Vec<&str> =
            path.split('?').next().unwrap_or("").split('/').filter(|s| !s.is_empty()).collect();
        // A Receiver interface addresses a client-owned object, so its path starts with the two
        // owner segments the Sender interface does not have.
        let owned = interface == InterfaceRole::Receiver;
        match module {
            ModuleId::Locations => match (segments.len(), owned) {
                (0 | 1, false) | (3, true) => Some(Self::Location),
                (2, false) | (4, true) => Some(Self::Evse),
                (3, false) | (5, true) => Some(Self::Connector),
                _ => None,
            },
            ModuleId::Sessions => match (segments.len(), owned) {
                (0, false) | (3, true) => Some(Self::Session),
                // `{session_id}/charging_preferences` carries a ChargingPreferences, unchanged.
                _ => None,
            },
            // The Receiver interface takes a POST of one CDR and a GET of one by id; the Sender
            // interface lists them.
            ModuleId::Cdrs => (segments.len() <= 1).then_some(Self::Cdr),
            ModuleId::Tariffs => match (segments.len(), owned) {
                (0, false) | (3, true) => Some(Self::Tariff),
                _ => None,
            },
            ModuleId::Tokens => match (segments.last(), owned) {
                // The request is a `LocationReferences`, unchanged; the response is the decision.
                (Some(&"authorize"), false) => {
                    (payload == Payload::Response).then_some(Self::AuthorizationInfo)
                }
                _ => match (segments.len(), owned) {
                    (0, false) | (3, true) => Some(Self::Token),
                    _ => None,
                },
            },
            // `CommandResponse` and `CommandResult` are unchanged; only two of the five request
            // bodies carry a Token.
            ModuleId::Commands if payload == Payload::Request => match segments.first() {
                Some(&"START_SESSION") => Some(Self::StartSession),
                Some(&"RESERVE_NOW") => Some(Self::ReserveNow),
                _ => None,
            },
            ModuleId::Credentials => segments.is_empty().then_some(Self::Credentials),
            ModuleId::HubClientInfo => match (segments.len(), owned) {
                (0, false) | (2, true) => Some(Self::ClientInfo),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn location_2_2_1() -> Value {
        json!({
            "country_code": "BE", "party_id": "BEC", "id": "LOC1", "publish": true,
            "address": "F.Rooseveltlaan 3A", "city": "Gent", "country": "BEL",
            "coordinates": {"latitude": "51.047599", "longitude": "3.729944"},
            "time_zone": "Europe/Brussels", "last_updated": "2019-06-24T12:39:09Z"
        })
    }

    #[test]
    fn a_page_reports_each_objects_losses_under_its_own_index() {
        let mut location = location_2_2_1();
        let page = Value::Array(vec![location.clone(), location.clone()]);
        let up = ObjectKind::Location.bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_3_0, page).unwrap();
        assert!(up.lossy.is_empty(), "2.2.1 → 2.3.0 adds fields, it does not drop them");

        // Give the second one something 2.2.1 cannot hold, and carry the page back.
        location["help_phone"] = json!("+3212345678");
        let up = ObjectKind::Location
            .bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_3_0, location_2_2_1())
            .unwrap();
        let mut with_phone = up.value.clone();
        with_phone["help_phone"] = json!("+3212345678");
        let page = Value::Array(vec![up.value, with_phone]);
        let down = ObjectKind::Location.bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_2_1, page).unwrap();
        assert_eq!(down.lossy.len(), 1);
        assert_eq!(down.lossy.as_slice()[0].pointer, "/1/help_phone");
    }

    #[test]
    fn a_version_to_itself_is_the_identity_and_costs_nothing() {
        let value = location_2_2_1();
        let same = ObjectKind::Location
            .bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_2_1, value.clone())
            .unwrap();
        assert_eq!(same.value, value);
        assert!(same.lossy.is_empty());
    }

    #[test]
    fn a_crossing_this_build_cannot_make_is_refused_rather_than_guessed_at() {
        let error = ObjectKind::Location
            .bridge(&VersionNumber::V2_1_1, &VersionNumber::V2_3_0, location_2_2_1())
            .unwrap_err();
        assert!(matches!(error, BridgeError::Unsupported { .. }), "{error}");
        assert!(!bridgeable(&VersionNumber::V2_1_1, &VersionNumber::V2_3_0));
        assert!(bridgeable(&VersionNumber::V2_2_1, &VersionNumber::V2_3_0));
        assert!(bridgeable(&VersionNumber::V2_1_1, &VersionNumber::V2_1_1));
    }

    #[test]
    fn a_document_that_is_not_the_object_the_endpoint_carries_is_named_as_such() {
        let error = ObjectKind::Tariff
            .bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_3_0, json!({"id": "1"}))
            .unwrap_err();
        match error {
            BridgeError::Decode { kind, version, .. } => {
                assert_eq!(kind, ObjectKind::Tariff);
                assert_eq!(version, VersionNumber::V2_2_1);
            }
            other => panic!("{other}"),
        }
    }

    #[test]
    fn an_absent_data_field_survives() {
        let out =
            ObjectKind::Cdr.bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_2_1, Value::Null).unwrap();
        assert_eq!(out.value, Value::Null);
    }

    #[test]
    fn the_locations_url_shapes_name_the_object_they_carry() {
        let sender = |p: &str| {
            ObjectKind::for_endpoint(&ModuleId::Locations, InterfaceRole::Sender, p, Payload::Response)
        };
        assert_eq!(sender(""), Some(ObjectKind::Location));
        assert_eq!(sender("LOC1"), Some(ObjectKind::Location));
        assert_eq!(sender("LOC1/3256"), Some(ObjectKind::Evse));
        assert_eq!(sender("/LOC1/3256/1/"), Some(ObjectKind::Connector));

        let receiver = |p: &str| {
            ObjectKind::for_endpoint(&ModuleId::Locations, InterfaceRole::Receiver, p, Payload::Request)
        };
        assert_eq!(receiver("NL/TNM/LOC1"), Some(ObjectKind::Location));
        assert_eq!(receiver("NL/TNM/LOC1/3256"), Some(ObjectKind::Evse));
        assert_eq!(receiver("NL/TNM/LOC1/3256/1"), Some(ObjectKind::Connector));
    }

    #[test]
    fn the_two_endpoints_whose_halves_differ_are_told_apart() {
        // `POST {tokens}/{uid}/authorize` sends a LocationReferences and gets a decision back.
        let authorize = |payload| {
            ObjectKind::for_endpoint(&ModuleId::Tokens, InterfaceRole::Sender, "012345/authorize", payload)
        };
        assert_eq!(authorize(Payload::Request), None);
        assert_eq!(authorize(Payload::Response), Some(ObjectKind::AuthorizationInfo));

        // A command's response is a `CommandResponse`, which is unchanged.
        let command = |name: &str, payload| {
            ObjectKind::for_endpoint(&ModuleId::Commands, InterfaceRole::Receiver, name, payload)
        };
        assert_eq!(command("START_SESSION", Payload::Request), Some(ObjectKind::StartSession));
        assert_eq!(command("RESERVE_NOW", Payload::Request), Some(ObjectKind::ReserveNow));
        assert_eq!(command("STOP_SESSION", Payload::Request), None);
        assert_eq!(command("START_SESSION", Payload::Response), None);
    }

    #[test]
    fn an_endpoint_whose_object_did_not_change_asks_for_no_work() {
        let query = |module| ObjectKind::for_endpoint(module, InterfaceRole::Sender, "", Payload::Response);
        assert_eq!(query(&ModuleId::ChargingProfiles), None);
        assert_eq!(query(&ModuleId::Payments), None);
        assert_eq!(query(&ModuleId::Versions), None);
        assert_eq!(
            ObjectKind::for_endpoint(
                &ModuleId::Sessions,
                InterfaceRole::Sender,
                "SESS1/charging_preferences",
                Payload::Request,
            ),
            None,
        );
    }
}
