//! The *Locations* module of OCPI 2.1.1.
//!
//! Spec: 2.1.1 §mod_locations

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_lenient_enum;
use crate::types::validate_fields;
use crate::types::{
    DateTime, DisplayText, Extensions, Number, OcpiString, Url, Validate, Validator, ViolationCode,
};

// Wire-identical to OCPI 2.3.0.
pub use crate::v2_3_0::locations::{
    AdditionalGeoLocation, BusinessDetails, ConnectorFormat, EnergySource, EnergySourceCategory,
    EnvironmentalImpactCategory, ExceptionalPeriod, GeoLocation, Image, ImageCategory, RegularHours, Status,
    StatusSchedule,
};

/// Waste produced or emitted per kWh, in OCPI 2.1.1.
///
/// **The field is named `source` here.** OCPI 2.2 renamed it to `category`, which is what
/// [`v2_3_0::locations::EnvironmentalImpact`](crate::v2_3_0::locations::EnvironmentalImpact) uses.
/// Reusing the later type would silently drop a 2.1.1 peer's value into `extensions`.
///
/// Spec: 2.1.1 §mod_locations_environmentalimpact_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EnvironmentalImpact {
    /// The category of this value.
    pub source: EnvironmentalImpactCategory,
    /// Amount of this portion in g/kWh.
    pub amount: Number,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for EnvironmentalImpact {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, source, amount);
    }
}

/// The energy mix and environmental impact of the energy supplied, in OCPI 2.1.1.
///
/// Field-for-field the same as later versions; it is redefined only because its
/// `environ_impact` holds the 2.1.1 [`EnvironmentalImpact`], whose field is `source`.
///
/// Spec: 2.1.1 §mod_locations_energymix_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct EnergyMix {
    /// True if 100% from regenerative sources.
    pub is_green_energy: bool,
    /// Energy sources of this location's tariff.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub energy_sources: Vec<EnergySource>,
    /// Nuclear waste and CO2 exhaust of this location's tariff.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub environ_impact: Vec<EnvironmentalImpact>,
    /// Name of the energy supplier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supplier_name: Option<OcpiString<64>>,
    /// Name of the energy supplier's product or tariff plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_product_name: Option<OcpiString<64>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for EnergyMix {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, energy_sources, environ_impact, supplier_name, energy_product_name);
    }
}

/// Opening and access hours, in OCPI 2.1.1.
///
/// > *Choice: one of two — `regular_hours` … `twentyfourseven`*
///
/// In OCPI 2.1.1 the two are **alternatives**, and a peer that publishes weekday hours sends no
/// `twentyfourseven` at all. OCPI 2.2 made `twentyfourseven` required, so reusing the later type
/// here would fail to decode a perfectly ordinary 2.1.1 Location.
///
/// Spec: 2.1.1 §mod_locations_hours_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Hours {
    /// Regular weekday-based hours.
    ///
    /// > *Should not be set for representing 24/7 as this is the most common case.*
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub regular_hours: Vec<RegularHours>,
    /// True to represent 24 hours a day and 7 days a week, except the given exceptions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twentyfourseven: Option<bool>,
    /// Periods the station is operating or accessible, additional to `regular_hours`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub exceptional_openings: Vec<ExceptionalPeriod>,
    /// Periods the station is not operating or accessible, overriding everything else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub exceptional_closings: Vec<ExceptionalPeriod>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Hours {
    /// Whether the location is open around the clock, applying the 2.1.1 choice.
    #[must_use]
    pub fn is_always_open(&self) -> bool {
        self.twentyfourseven.unwrap_or(false)
    }
}

impl Validate for Hours {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, regular_hours, exceptional_openings, exceptional_closings);
        match (self.regular_hours.is_empty(), self.twentyfourseven.is_some()) {
            (true, false) => v.report(
                ViolationCode::MissingConditional,
                "Hours is a choice of one of two: either `regular_hours` or `twentyfourseven` \
                 must be given",
            ),
            (false, true) => v.report(
                ViolationCode::Inconsistent,
                "Hours is a choice of one of two: `regular_hours` and `twentyfourseven` are \
                 alternatives, not a combination",
            ),
            _ => {}
        }
    }
}

/// Where a group of EVSEs is installed, in OCPI 2.1.1.
///
/// Compared with later versions this object has **no owner fields**: `country_code` and
/// `party_id` came in with OCPI 2.2. In 2.1.1 the owner is known only from the URL a
/// client-owned object is pushed to, and from the credentials handshake.
///
/// It also has a required [`LocationType`], which 2.2 replaced with the optional
/// [`ParkingType`](crate::v2_3_0::locations::ParkingType), and its `id` is a `string(39)` rather
/// than a `CiString(36)`.
///
/// Spec: 2.1.1 §mod_locations_location_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Location {
    /// Uniquely identifies the location within the CPO's platform.
    pub id: OcpiString<39>,
    /// The general type of the charge point location.
    #[serde(rename = "type")]
    pub location_type: LocationType,
    /// Display name of the location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<OcpiString<255>>,
    /// Street/block name and house number if available.
    pub address: OcpiString<45>,
    /// City or town.
    pub city: OcpiString<45>,
    /// Postal code of the location. **Required** in 2.1.1; optional from 2.2 onwards.
    pub postal_code: OcpiString<10>,
    /// ISO 3166-1 alpha-3 code for the country of this location.
    pub country: OcpiString<3>,
    /// Coordinates of the location.
    pub coordinates: GeoLocation,
    /// Geographical locations of related points relevant to the user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub related_locations: Vec<AdditionalGeoLocation>,
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
    /// One of IANA tzdata's TZ values. **Optional** in 2.1.1; required from 2.2 onwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<OcpiString<255>>,
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
    /// Whether the EVSEs keep charging outside opening hours, applying the spec's default.
    #[must_use]
    pub fn charging_when_closed_or_default(&self) -> bool {
        self.charging_when_closed.unwrap_or(true)
    }

    /// Finds an EVSE by its `uid`.
    ///
    /// The comparison is **case-sensitive**: 2.1.1 types `EVSE.uid` as `string(39)`, not as a
    /// `CiString`, and this crate follows the specification of each version exactly.
    #[must_use]
    pub fn evse(&self, uid: &str) -> Option<&Evse> {
        self.evses.iter().find(|e| e.uid.as_str() == uid)
    }
}

impl Validate for Location {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self, v, id, location_type as "type", name, address, city, postal_code, country,
            coordinates, related_locations, evses, directions, operator, suboperator, owner,
            facilities, time_zone, opening_times, images, energy_mix, last_updated,
        );
    }
}

/// The part that controls the power supply to a single EV, in OCPI 2.1.1.
///
/// Spec: 2.1.1 §mod_locations_evse_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Evse {
    /// Uniquely identifies the EVSE within the CPO's platform.
    pub uid: OcpiString<39>,
    /// The human-readable EVSE ID in the eMI3 format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_id: Option<OcpiString<48>>,
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
    /// Level on which the charging station is located.
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

impl Validate for Evse {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            uid,
            evse_id,
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

/// The socket, or cable and plug, available for the EV to use, in OCPI 2.1.1.
///
/// The electrical fields are named `voltage` and `amperage` here; OCPI 2.2 renamed them to
/// `max_voltage` and `max_amperage` and added `max_electric_power`. `tariff_id` is a single
/// optional value; 2.2 made it the list `tariff_ids`.
///
/// Spec: 2.1.1 §mod_locations_connector_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Connector {
    /// Identifier of the connector within the EVSE.
    pub id: OcpiString<36>,
    /// The standard of the installed connector.
    pub standard: ConnectorType,
    /// The format (socket/cable) of the installed connector.
    pub format: ConnectorFormat,
    /// Whether the connector supplies AC or DC, and on how many phases.
    pub power_type: PowerType,
    /// Voltage of the connector (line to neutral for `AC_3_PHASE`), in volt.
    pub voltage: i32,
    /// Maximum amperage of the connector, in ampere.
    pub amperage: i32,
    /// Identifier of the current charging tariff structure.
    ///
    /// > *For a "Free of Charge" tariff this field should be set, and point to a defined "Free of
    /// > Charge" tariff.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tariff_id: Option<OcpiString<36>>,
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
            tariff_id,
            terms_and_conditions,
            last_updated,
        );
        if self.voltage <= 0 {
            v.report_at("voltage", ViolationCode::OutOfRange, "must be a positive voltage");
        }
        if self.amperage <= 0 {
            v.report_at("amperage", ViolationCode::OutOfRange, "must be a positive amperage");
        }
    }
}

ocpi_lenient_enum! {
    /// The general type of the charge point location.
    ///
    /// Removed in OCPI 2.2, which replaced it with the optional
    /// [`ParkingType`](crate::v2_3_0::locations::ParkingType) and dropped the `OTHER`/`UNKNOWN`
    /// escape hatches.
    ///
    /// Spec: 2.1.1 §mod_locations_locationtype_enum
    pub enum LocationType {
        /// Parking in public space.
        OnStreet = "ON_STREET",
        /// Multistorey car park.
        ParkingGarage = "PARKING_GARAGE",
        /// Multistorey car park, mainly underground.
        UndergroundGarage = "UNDERGROUND_GARAGE",
        /// A cleared area intended for parking vehicles.
        ParkingLot = "PARKING_LOT",
        /// None of the given possibilities.
        Other = "OTHER",
        /// Not known by the operator. The default.
        Unknown = "UNKNOWN",
    }
}

ocpi_lenient_enum! {
    /// The capabilities of an EVSE, in OCPI 2.1.1.
    ///
    /// Six values; OCPI 2.2 grew this to thirteen.
    ///
    /// Spec: 2.1.1 §mod_locations_capability_enum
    pub enum Capability {
        /// The EVSE supports charging profiles.
        ChargingProfileCapable = "CHARGING_PROFILE_CAPABLE",
        /// Payment of a charging session can be done using a credit card.
        CreditCardPayable = "CREDIT_CARD_PAYABLE",
        /// The EVSE can remotely be started/stopped.
        RemoteStartStopCapable = "REMOTE_START_STOP_CAPABLE",
        /// The EVSE can be reserved.
        Reservable = "RESERVABLE",
        /// Charging at this EVSE can be authorized with an RFID token.
        RfidReader = "RFID_READER",
        /// Connectors have a mechanical lock that can be requested to be unlocked.
        UnlockCapable = "UNLOCK_CAPABLE",
    }
}

ocpi_lenient_enum! {
    /// The socket or plug standard of the charging point, in OCPI 2.1.1.
    ///
    /// Twenty values. Everything OCPI 2.2 and 2.3.0 added — the GB/T, IEC 60309, NEMA,
    /// pantograph, ChaoJi, MCS and SAE J3400 families — is absent, which is precisely why
    /// [`ocpi_lenient_enum!`] is used here: a 2.1.1 peer that has installed a CCS-adjacent plug
    /// invented in the last decade will send a value this list does not have.
    ///
    /// Spec: 2.1.1 §mod_locations_connectortype_enum
    pub enum ConnectorType {
        /// CHAdeMO, DC.
        Chademo = "CHADEMO",
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
        /// IEC 60309-2 Industrial Connector, single phase 16 A (usually blue).
        Iec603092Single16 = "IEC_60309_2_single_16",
        /// IEC 60309-2 Industrial Connector, three phases 16 A (usually red).
        Iec603092Three16 = "IEC_60309_2_three_16",
        /// IEC 60309-2 Industrial Connector, three phases 32 A (usually red).
        Iec603092Three32 = "IEC_60309_2_three_32",
        /// IEC 60309-2 Industrial Connector, three phases 64 A (usually red).
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
        /// Tesla Connector "Roadster"-type (round, 4 pin).
        TeslaR = "TESLA_R",
        /// Tesla Connector "Model-S"-type (oval, 5 pin).
        TeslaS = "TESLA_S",
    }
}

ocpi_lenient_enum! {
    /// Facilities a charging location directly belongs to, in OCPI 2.1.1.
    ///
    /// Spec: 2.1.1 §mod_locations_facility_enum
    pub enum Facility {
        /// A hotel.
        Hotel = "HOTEL",
        /// A restaurant.
        Restaurant = "RESTAURANT",
        /// A cafe.
        Cafe = "CAFE",
        /// A mall or shopping center.
        Mall = "MALL",
        /// A supermarket.
        Supermarket = "SUPERMARKET",
        /// Sport facilities.
        Sport = "SPORT",
        /// A recreation area.
        RecreationArea = "RECREATION_AREA",
        /// Located in, or close to, a park or nature reserve.
        Nature = "NATURE",
        /// A museum.
        Museum = "MUSEUM",
        /// A bus stop.
        BusStop = "BUS_STOP",
        /// A taxi stand.
        TaxiStand = "TAXI_STAND",
        /// A train station.
        TrainStation = "TRAIN_STATION",
        /// An airport.
        Airport = "AIRPORT",
        /// A carpool parking.
        CarpoolParking = "CARPOOL_PARKING",
        /// A fuel station.
        FuelStation = "FUEL_STATION",
        /// Wifi or other type of internet available.
        Wifi = "WIFI",
    }
}

ocpi_lenient_enum! {
    /// Restrictions on the parking spot, in OCPI 2.1.1.
    ///
    /// Spec: 2.1.1 §mod_locations_parkingrestriction_enum
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

ocpi_lenient_enum! {
    /// Whether a connector supplies AC or DC, in OCPI 2.1.1.
    ///
    /// The two-phase variants arrived in OCPI 2.2.
    ///
    /// Spec: 2.1.1 §mod_locations_powertype_enum
    pub enum PowerType {
        /// AC single phase.
        Ac1Phase = "AC_1_PHASE",
        /// AC three phases.
        Ac3Phase = "AC_3_PHASE",
        /// Direct current.
        Dc = "DC",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_2_1_1_enums_are_much_smaller_than_the_later_ones() {
        assert_eq!(ConnectorType::ALL_KNOWN.len(), 25);
        // The four IEC 60309 industrial sockets are the only lower-case wire values in OCPI.
        let blue: ConnectorType = "IEC_60309_2_single_16".into();
        assert!(blue.is_known(), "the blue 16 A industrial socket is a 2.1.1 value");
        assert_eq!(serde_json::to_string(&blue).unwrap(), "\"IEC_60309_2_single_16\"");
        assert_eq!(Capability::ALL_KNOWN.len(), 6);
        assert_eq!(PowerType::ALL_KNOWN.len(), 3);
        // A connector standard invented after 2.1.1 still decodes …
        let mcs: ConnectorType = "MCS".into();
        assert_eq!(serde_json::to_string(&mcs).unwrap(), "\"MCS\"");
        // … and is reported, because 2.1.1 declares the enum closed.
        assert!(mcs.validate().is_err());
    }

    #[test]
    fn a_2_1_1_location_has_no_owner_fields() {
        let json = r#"{"id":"LOC1","type":"ON_STREET","address":"F.Rooseveltlaan 3A","city":"Gent","postal_code":"9000","country":"BEL","coordinates":{"latitude":"51.047599","longitude":"3.729944"},"last_updated":"2015-06-29T20:39:09Z"}"#;
        let location: Location = serde_json::from_str(json).unwrap();
        assert_eq!(location.location_type, LocationType::OnStreet);
        assert!(location.validate().is_ok());
        assert_eq!(serde_json::to_string(&location).unwrap(), json);
    }

    #[test]
    fn hours_is_a_choice_of_one_of_two_in_2_1_1() {
        // A 2.1.1 peer publishing weekday hours sends no `twentyfourseven` at all, which the
        // OCPI 2.2 shape would refuse to decode.
        let weekdays: Hours = serde_json::from_str(
            r#"{"regular_hours":[{"weekday":1,"period_begin":"08:00","period_end":"20:00"}]}"#,
        )
        .unwrap();
        assert!(weekdays.validate().is_ok());
        assert!(!weekdays.is_always_open());

        let always: Hours = serde_json::from_str(r#"{"twentyfourseven":true}"#).unwrap();
        assert!(always.validate().is_ok());
        assert!(always.is_always_open());

        // Neither, or both, is a violation: they are alternatives.
        assert!(serde_json::from_str::<Hours>("{}").unwrap().validate().is_err());
        let both: Hours = serde_json::from_str(
            r#"{"twentyfourseven":true,"regular_hours":[{"weekday":1,"period_begin":"08:00","period_end":"20:00"}]}"#,
        )
        .unwrap();
        assert!(both.validate().is_err());
    }

    #[test]
    fn the_environmental_impact_field_is_named_source_in_2_1_1() {
        // OCPI 2.2 renamed it to `category`; reusing the later type would drop the value.
        let json = r#"{"source":"CARBON_DIOXIDE","amount":230}"#;
        let impact: EnvironmentalImpact = serde_json::from_str(json).unwrap();
        assert_eq!(impact.source, EnvironmentalImpactCategory::CarbonDioxide);
        assert_eq!(serde_json::to_string(&impact).unwrap(), json);
        assert!(impact.extensions.is_empty(), "nothing fell through into extensions");
    }

    #[test]
    fn evse_uids_compare_case_sensitively_because_2_1_1_says_string() {
        let location: Location = serde_json::from_str(
            r#"{"id":"LOC1","type":"ON_STREET","address":"a","city":"b","postal_code":"c","country":"NLD","coordinates":{"latitude":"51.047599","longitude":"3.729944"},"evses":[{"uid":"AB123","status":"AVAILABLE","connectors":[{"id":"1","standard":"IEC_62196_T2","format":"SOCKET","power_type":"AC_3_PHASE","voltage":400,"amperage":32,"last_updated":"2015-06-29T20:39:09Z"}],"last_updated":"2015-06-29T20:39:09Z"}],"last_updated":"2015-06-29T20:39:09Z"}"#,
        )
        .unwrap();
        assert!(location.evse("AB123").is_some());
        assert!(location.evse("ab123").is_none(), "2.1.1 types EVSE.uid as string, not CiString");
    }
}
