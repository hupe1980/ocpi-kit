//! The *Locations* module of OCPI 2.2.1, as a delta from
//! [`v2_3_0::locations`](crate::v2_3_0::locations).
//!
//! What changed in 2.3.0 and is therefore **absent here**:
//!
//! * the [`Parking`](crate::v2_3_0::locations::Parking) object and everything that references it
//!   (`Location.parking_places`, `EVSE.parking`, `EVSEParking`, `EVSEPosition`,
//!   `ParkingDirection`, `VehicleType`) — added for EU AFIR reporting;
//! * `Location.help_phone`;
//! * `EVSE.accepted_service_providers`;
//! * `Connector.capabilities` and the `ConnectorCapability` enum;
//! * the `MCS` and `SAE_J3400` connector types;
//! * the `EMPLOYEES`, `TAXIS` and `TENANTS` parking restrictions.
//!
//! Everything else is wire-identical and re-exported from the 2.3.0 module, so a
//! `GeoLocation` is the same type in both versions and needs no conversion.
//!
//! Spec: 2.2.1 §mod_locations_locations_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_lenient_enum;
use crate::types::validate_fields;
use crate::types::{
    CiString, CountryCode, DateTime, DisplayText, EvseId, Extensions, OcpiString, PartyId, PartyRef, Url,
    Validate, Validator, ViolationCode,
};

use super::tokens::TokenType;

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::locations::{
    AdditionalGeoLocation, BusinessDetails, Capability, ConnectorFormat, EnergyMix, EnergySource,
    EnergySourceCategory, EnvironmentalImpact, EnvironmentalImpactCategory, ExceptionalPeriod, Facility,
    GeoLocation, Hours, Image, ImageCategory, ParkingType, PowerType, RegularHours, Status, StatusSchedule,
};

/// Where a group of EVSEs that belong together is installed, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_locations_location_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Location {
    /// ISO-3166 alpha-2 country code of the CPO that 'owns' this Location.
    pub country_code: CountryCode,
    /// ID of the CPO that 'owns' this Location.
    pub party_id: PartyId,
    /// Uniquely identifies the location within the CPO's platform.
    pub id: CiString<36>,
    /// Whether the Location may be published on a website or app.
    pub publish: bool,
    /// Tokens allowed to be shown this Location when [`publish`](Self::publish) is `false`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub publish_allowed_to: Vec<PublishTokenType>,
    /// Display name of the location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<OcpiString<255>>,
    /// Street/block name and house number if available.
    pub address: OcpiString<255>,
    /// City or town.
    pub city: OcpiString<45>,
    /// Postal code, omitted only where the location genuinely has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<OcpiString<10>>,
    /// State or province, only where relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<OcpiString<45>>,
    /// ISO 3166-1 alpha-3 code for the country of this location.
    pub country: OcpiString<3>,
    /// Coordinates of the location.
    pub coordinates: GeoLocation,
    /// Geographical locations of related points relevant to the user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub related_locations: Vec<AdditionalGeoLocation>,
    /// The general type of parking at the charge point location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parking_type: Option<ParkingType>,
    /// The EVSEs that belong to this Location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub evses: Vec<Evse>,
    /// Human-readable directions on how to reach the location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub directions: Vec<DisplayText>,
    /// Information of the operator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<BusinessDetails>,
    /// Information of the suboperator if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suboperator: Option<BusinessDetails>,
    /// Information of the owner if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<BusinessDetails>,
    /// Facilities this charging location directly belongs to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub facilities: Vec<Facility>,
    /// One of IANA tzdata's TZ values, e.g. `Europe/Oslo`.
    pub time_zone: OcpiString<255>,
    /// When the EVSEs at the location can be accessed for charging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening_times: Option<Hours>,
    /// Whether the EVSEs still charge outside the opening hours. Default: `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_when_closed: Option<bool>,
    /// Links to images related to the location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub images: Vec<Image>,
    /// Details on the energy supplied at this location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_mix: Option<EnergyMix>,
    /// Timestamp when this Location or one of its EVSEs or Connectors was last updated.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Location {
    /// The CPO that owns this Location.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }

    /// Whether the EVSEs keep charging outside opening hours, applying the spec's default.
    #[must_use]
    pub fn charging_when_closed_or_default(&self) -> bool {
        self.charging_when_closed.unwrap_or(true)
    }

    /// Finds an EVSE by its `uid`, comparing case-insensitively.
    #[must_use]
    pub fn evse(&self, uid: &str) -> Option<&Evse> {
        self.evses.iter().find(|e| e.uid.eq_ignore_case(uid))
    }

    /// Whether this Location may be shown to the holder of the given token.
    ///
    /// See [`v2_3_0::locations::Location::may_publish_to`](crate::v2_3_0::locations::Location::may_publish_to).
    #[must_use]
    pub fn may_publish_to(&self, token: Option<&PublishTokenType>) -> bool {
        if self.publish {
            return true;
        }
        token.is_some_and(|t| self.publish_allowed_to.iter().any(|allowed| allowed.matches(t)))
    }
}

impl Validate for Location {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            country_code,
            party_id,
            id,
            publish_allowed_to,
            name,
            address,
            city,
            postal_code,
            state,
            country,
            coordinates,
            related_locations,
            parking_type,
            evses,
            directions,
            operator,
            suboperator,
            owner,
            facilities,
            time_zone,
            opening_times,
            images,
            energy_mix,
            last_updated,
        );
        if self.publish && !self.publish_allowed_to.is_empty() {
            v.report_at(
                "publish_allowed_to",
                ViolationCode::Inconsistent,
                "this field may only be used when `publish` is false",
            );
        }
    }
}

/// The part that controls the power supply to a single EV, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_locations_evse_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Evse {
    /// Uniquely identifies the EVSE within the CPO's platform.
    pub uid: CiString<36>,
    /// The human-readable EVSE ID in the eMI3 format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_id: Option<EvseId>,
    /// The current status of the EVSE.
    pub status: Status,
    /// Planned status updates of the EVSE.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub status_schedule: Vec<StatusSchedule>,
    /// Functionalities that the EVSE is capable of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub capabilities: Vec<Capability>,
    /// Available connectors on the EVSE. Cardinality `+`.
    pub connectors: Vec<Connector>,
    /// Level on which the Charge Point is located.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_level: Option<OcpiString<4>>,
    /// Coordinates of the EVSE.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<GeoLocation>,
    /// A number/string printed on the outside of the EVSE for visual identification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_reference: Option<OcpiString<16>>,
    /// Directions on how to reach the EVSE from the Location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub directions: Vec<DisplayText>,
    /// The restrictions that apply to the parking spot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub parking_restrictions: Vec<ParkingRestriction>,
    /// Links to images related to the EVSE.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub images: Vec<Image>,
    /// Timestamp when this EVSE or one of its Connectors was last updated.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Evse {
    /// Whether a `StartSession` for this EVSE must carry a `connector_id`.
    #[must_use]
    pub fn requires_connector_id_on_start(&self) -> bool {
        self.capabilities.contains(&Capability::StartSessionConnectorRequired)
    }

    /// Finds a Connector by its `id`, comparing case-insensitively.
    #[must_use]
    pub fn connector(&self, id: &str) -> Option<&Connector> {
        self.connectors.iter().find(|c| c.id.eq_ignore_case(id))
    }
}

impl Validate for Evse {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            uid,
            evse_id,
            status,
            status_schedule,
            capabilities,
            connectors,
            floor_level,
            coordinates,
            physical_reference,
            directions,
            parking_restrictions,
            images,
            last_updated,
        );
        if self.connectors.is_empty() {
            v.report_at(
                "connectors",
                ViolationCode::EmptyRequiredList,
                "an EVSE has cardinality `+` connectors: at least one is required",
            );
        }
    }
}

/// The socket, or cable and plug, available for the EV to use, in OCPI 2.2.1.
///
/// Spec: 2.2.1 §mod_locations_connector_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Connector {
    /// Identifier of the Connector within the EVSE.
    pub id: CiString<36>,
    /// The standard of the installed connector.
    pub standard: ConnectorType,
    /// The format (socket/cable) of the installed connector.
    pub format: ConnectorFormat,
    /// Whether the connector supplies AC or DC, and on how many phases.
    pub power_type: PowerType,
    /// Maximum voltage of the connector, in volt.
    pub max_voltage: i32,
    /// Maximum amperage of the connector, in ampere.
    pub max_amperage: i32,
    /// Maximum electric power this connector can deliver, in watt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_electric_power: Option<i32>,
    /// Identifiers of the currently valid charging tariffs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tariff_ids: Vec<CiString<36>>,
    /// URL to the operator's terms and conditions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms_and_conditions: Option<Url>,
    /// Timestamp when this Connector was last updated.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for Connector {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            id,
            standard,
            format,
            power_type,
            tariff_ids,
            terms_and_conditions,
            last_updated,
        );
        if self.max_voltage <= 0 {
            v.report_at("max_voltage", ViolationCode::OutOfRange, "must be a positive voltage");
        }
        if self.max_amperage <= 0 {
            v.report_at("max_amperage", ViolationCode::OutOfRange, "must be a positive amperage");
        }
    }
}

/// The set of values that identify a token to which a Location might be published.
///
/// Identical in shape to the 2.3.0 object, but its `type` is the 2.2.1
/// [`TokenType`], which has no `EMAID`.
///
/// Spec: 2.2.1 §mod_locations_publish_token_class
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct PublishTokenType {
    /// Unique ID by which this Token can be identified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<CiString<36>>,
    /// Type of the token.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<TokenType>,
    /// Visual readable number/identification as printed on the Token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_number: Option<OcpiString<64>>,
    /// Issuing company, most of the time the name printed on the token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<OcpiString<64>>,
    /// Groups a couple of tokens so that two or more tokens work as one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<CiString<36>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl PublishTokenType {
    /// Whether `candidate` matches all the set fields of this publish token.
    #[must_use]
    pub fn matches(&self, candidate: &Self) -> bool {
        fn agree<T: PartialEq>(required: Option<&T>, given: Option<&T>) -> bool {
            required.is_none_or(|r| given == Some(r))
        }
        agree(self.uid.as_ref(), candidate.uid.as_ref())
            && agree(self.token_type.as_ref(), candidate.token_type.as_ref())
            && agree(self.visual_number.as_ref(), candidate.visual_number.as_ref())
            && agree(self.issuer.as_ref(), candidate.issuer.as_ref())
            && agree(self.group_id.as_ref(), candidate.group_id.as_ref())
    }
}

impl Validate for PublishTokenType {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, uid, token_type as "type", visual_number, issuer, group_id);
        if self.uid.is_none() && self.visual_number.is_none() && self.group_id.is_none() {
            v.report(
                ViolationCode::MissingConditional,
                "at least one of `uid`, `visual_number` or `group_id` SHALL be set",
            );
        }
        if self.uid.is_some() && self.token_type.is_none() {
            v.report_at("type", ViolationCode::MissingConditional, "SHALL be set when `uid` is set");
        }
        if self.visual_number.is_some() && self.issuer.is_none() {
            v.report_at(
                "issuer",
                ViolationCode::MissingConditional,
                "SHALL be set when `visual_number` is set",
            );
        }
    }
}

ocpi_lenient_enum! {
    /// The socket or plug standard of the charging point, in OCPI 2.2.1.
    ///
    /// The 2.3.0 list plus `MCS` and `SAE_J3400`, minus those two. OCPI 2.2.1 declares this a
    /// closed enum; OCPI 2.3.0 reclassified it as an `OpenEnum`, which is why this crate keeps an
    /// unrecognised value instead of failing the object. See [`ocpi_lenient_enum!`].
    ///
    /// Spec: 2.2.1 §mod_locations_connectortype_enum
    pub enum ConnectorType {
        /// CHAdeMO, DC.
        Chademo = "CHADEMO",
        /// The ChaoJi connector, harmonized between CHAdeMO and GB/T. DC.
        ChaoJi = "CHAOJI",
        /// Standard/Domestic household, type "A", NEMA 1-15, 2 pins.
        DomesticA = "DOMESTIC_A",
        /// Standard/Domestic household, type "B", NEMA 5-15, 3 pins.
        DomesticB = "DOMESTIC_B",
        /// Standard/Domestic household, type "C", CEE 7/17, 2 pins.
        DomesticC = "DOMESTIC_C",
        /// Standard/Domestic household, type "D", 3 pin.
        DomesticD = "DOMESTIC_D",
        /// Standard/Domestic household, type "E", CEE 7/5, 3 pins.
        DomesticE = "DOMESTIC_E",
        /// Standard/Domestic household, type "F", CEE 7/4, Schuko, 3 pins.
        DomesticF = "DOMESTIC_F",
        /// Standard/Domestic household, type "G", BS 1363, Commonwealth, 3 pins.
        DomesticG = "DOMESTIC_G",
        /// Standard/Domestic household, type "H", SI-32, 3 pins.
        DomesticH = "DOMESTIC_H",
        /// Standard/Domestic household, type "I", AS 3112, 3 pins.
        DomesticI = "DOMESTIC_I",
        /// Standard/Domestic household, type "J", SEV 1011, 3 pins.
        DomesticJ = "DOMESTIC_J",
        /// Standard/Domestic household, type "K", DS 60884-2-D1, 3 pins.
        DomesticK = "DOMESTIC_K",
        /// Standard/Domestic household, type "L", CEI 23-16-VII, 3 pins.
        DomesticL = "DOMESTIC_L",
        /// Standard/Domestic household, type "M", BS 546, 3 pins.
        DomesticM = "DOMESTIC_M",
        /// Standard/Domestic household, type "N", NBR 14136, 3 pins.
        DomesticN = "DOMESTIC_N",
        /// Standard/Domestic household, type "O", TIS 166-2549, 3 pins.
        DomesticO = "DOMESTIC_O",
        /// Guobiao GB/T 20234.2 AC socket/connector.
        GbtAc = "GBT_AC",
        /// Guobiao GB/T 20234.3 DC connector.
        GbtDc = "GBT_DC",
        /// IEC 60309-2 Industrial Connector single phase 16 A.
        Iec603092Single16 = "IEC_60309_2_single_16",
        /// IEC 60309-2 Industrial Connector three phases 16 A.
        Iec603092Three16 = "IEC_60309_2_three_16",
        /// IEC 60309-2 Industrial Connector three phases 32 A.
        Iec603092Three32 = "IEC_60309_2_three_32",
        /// IEC 60309-2 Industrial Connector three phases 64 A.
        Iec603092Three64 = "IEC_60309_2_three_64",
        /// IEC 62196 Type 1 "SAE J1772".
        Iec62196T1 = "IEC_62196_T1",
        /// Combo Type 1 based, DC.
        Iec62196T1Combo = "IEC_62196_T1_COMBO",
        /// IEC 62196 Type 2 "Mennekes".
        Iec62196T2 = "IEC_62196_T2",
        /// Combo Type 2 based, DC.
        Iec62196T2Combo = "IEC_62196_T2_COMBO",
        /// IEC 62196 Type 3A.
        Iec62196T3A = "IEC_62196_T3A",
        /// IEC 62196 Type 3C "Scame".
        Iec62196T3C = "IEC_62196_T3C",
        /// NEMA 5-20, 3 pins.
        Nema520 = "NEMA_5_20",
        /// NEMA 6-30, 3 pins.
        Nema630 = "NEMA_6_30",
        /// NEMA 6-50, 3 pins.
        Nema650 = "NEMA_6_50",
        /// NEMA 10-30, 3 pins.
        Nema1030 = "NEMA_10_30",
        /// NEMA 10-50, 3 pins.
        Nema1050 = "NEMA_10_50",
        /// NEMA 14-30, 3 pins, rating of 30 A.
        Nema1430 = "NEMA_14_30",
        /// NEMA 14-50, 3 pins, rating of 50 A.
        Nema1450 = "NEMA_14_50",
        /// On-board bottom-up pantograph, typically for bus charging.
        PantographBottomUp = "PANTOGRAPH_BOTTOM_UP",
        /// Off-board top-down pantograph, typically for bus charging.
        PantographTopDown = "PANTOGRAPH_TOP_DOWN",
        /// Tesla Connector "Roadster"-type (round, 4 pin).
        TeslaR = "TESLA_R",
        /// Tesla Connector "Model-S"-type (oval, 5 pin).
        TeslaS = "TESLA_S",
    }
}

ocpi_lenient_enum! {
    /// Restrictions on the parking spot, in OCPI 2.2.1.
    ///
    /// OCPI 2.3.0 added `EMPLOYEES`, `TAXIS` and `TENANTS` and made the enum open.
    ///
    /// Spec: 2.2.1 §mod_locations_parkingrestriction_enum
    pub enum ParkingRestriction {
        /// Reserved parking spot for electric vehicles.
        EvOnly = "EV_ONLY",
        /// Parking is only allowed while plugged in (charging).
        Plugged = "PLUGGED",
        /// Reserved parking spot for disabled people with a valid ID.
        Disabled = "DISABLED",
        /// Parking spot for customers or guests only.
        Customers = "CUSTOMERS",
        /// Parking spot only suitable for (electric) motorcycles or scooters.
        Motorcycles = "MOTORCYCLES",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_2_3_0_connector_types_are_absent_but_still_decode() {
        assert!(ConnectorType::ALL_KNOWN_WIRE.iter().all(|v| *v != "MCS"));
        let mcs: ConnectorType = "MCS".into();
        assert!(!mcs.is_known(), "MCS arrived in OCPI 2.3.0");
        // Decoding must still succeed: one unknown plug cannot lose a page of Locations …
        assert_eq!(serde_json::to_string(&mcs).unwrap(), "\"MCS\"");
        // … but a conformance report says the peer sent something 2.2.1 does not define.
        assert!(mcs.validate().is_err());
        assert!(ConnectorType::Iec62196T2.validate().is_ok());
    }

    #[test]
    fn the_2_3_0_parking_restrictions_are_absent() {
        assert_eq!(ConnectorType::ALL_KNOWN.len(), 40);
        assert_eq!(ParkingRestriction::ALL_KNOWN.len(), 5);
        assert!(!ParkingRestriction::from("TENANTS").is_known());
    }

    #[test]
    fn wire_identical_types_are_the_same_rust_type_in_both_versions() {
        // A GeoLocation needs no conversion between 2.2.1 and 2.3.0 because it is one type.
        let geo: GeoLocation = crate::v2_3_0::locations::GeoLocation::new("52.010", "4.35000").unwrap();
        let _: crate::v2_3_0::locations::GeoLocation = geo;
    }
}
