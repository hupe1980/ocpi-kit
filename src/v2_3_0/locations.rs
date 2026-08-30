//! The *Locations* module of OCPI 2.3.0: where the EVSEs are and what they can do.
//!
//! *Module Identifier: `locations`* — Data owner: CPO.
//!
//! Spec: 2.3.0 §mod_locations_locations_module

use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::ocpi_enum;
use crate::ocpi_open_enum;
use crate::types::validate_fields;
use crate::types::{
    CiString, CiText, CountryCode, DateTime, DisplayText, EvseId, Extensions, LocalTime, Number, OcpiString,
    PartyId, PartyRef, Url, Validate, Validator, ViolationCode,
};

use super::tokens::TokenType;

/// Where a group of EVSEs that belong together is installed.
///
/// > *Typically, the Location object is the exact location of the group of EVSEs, but it can
/// > also be the entrance of a parking garage which contains these EVSEs.*
///
/// Spec: 2.3.0 §mod_locations_location_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Location {
    /// ISO-3166 alpha-2 country code of the CPO that 'owns' this Location.
    pub country_code: CountryCode,
    /// ID of the CPO that 'owns' this Location.
    pub party_id: PartyId,
    /// Uniquely identifies the location within the CPO's platform. Never changed or renamed.
    pub id: CiString<36>,
    /// Whether the Location may be published on a website or app.
    ///
    /// > *When this is set to `false`, only tokens identified in the field `publish_allowed_to`
    /// > are allowed to be shown this Location.*
    pub publish: bool,
    /// Tokens allowed to be shown this Location when [`publish`](Self::publish) is `false`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub publish_allowed_to: Vec<PublishTokenType>,
    /// Display name of the location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<OcpiString<255>>,
    /// Street/block name and house number if available.
    ///
    /// NOTE: earlier releases of the OCPI 2.3.0 documentation mistakenly gave a maximum of 45.
    pub address: OcpiString<255>,
    /// City or town.
    pub city: OcpiString<45>,
    /// Postal code, omitted only where the location genuinely has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<OcpiString<10>>,
    /// State or province, only where relevant.
    ///
    /// NOTE: earlier releases of the OCPI 2.3.0 documentation mistakenly gave a maximum of 20.
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
    /// Parking places usable by vehicles charging at this Location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub parking_places: Vec<Parking>,
    /// Human-readable directions on how to reach the location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub directions: Vec<DisplayText>,
    /// Information of the operator, when it differs from the party in the Credentials module.
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
    ///
    /// This is the time zone that [`LocalTime`] and [`crate::types::LocalDate`] values elsewhere
    /// in the protocol — opening hours, tariff restrictions — are expressed in.
    pub time_zone: OcpiString<255>,
    /// When the EVSEs at the location can be accessed for charging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening_times: Option<Hours>,
    /// Whether the EVSEs still charge outside the opening hours. Default: `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_when_closed: Option<bool>,
    /// Links to images related to the location such as photos or logos.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub images: Vec<Image>,
    /// Details on the energy supplied at this location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_mix: Option<EnergyMix>,
    /// A telephone number a Driver may call for assistance. New in OCPI 2.3.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_phone: Option<CiString<25>>,
    /// Timestamp when this Location or one of its EVSEs or Connectors was last updated.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Location {
    /// The party that owns this Location.
    #[must_use]
    pub fn owner_party(&self) -> PartyRef {
        PartyRef { country_code: self.country_code.clone(), party_id: self.party_id.clone() }
    }

    /// Whether the EVSEs keep charging outside opening hours, applying the spec's default.
    ///
    /// > *`charging_when_closed` … Default: **true***
    #[must_use]
    pub fn charging_when_closed_or_default(&self) -> bool {
        self.charging_when_closed.unwrap_or(true)
    }

    /// Finds an EVSE by its `uid`, comparing case-insensitively as `CiString` requires.
    #[must_use]
    pub fn evse(&self, uid: &str) -> Option<&Evse> {
        self.evses.iter().find(|e| e.uid.eq_ignore_case(uid))
    }

    /// Whether this Location may be shown to the holder of the given token.
    ///
    /// > *Locations that have this flag set to `false` SHALL not be shown in an app or on a
    /// > website etc. unless it is to the owner of a Token in the `publish_allowed_to` list. …
    /// > If the user … has provided information about his/her Token, and that information
    /// > matches **all the fields** of one of the PublishToken tokens in the list, then they are
    /// > allowed to show this location to their user.*
    ///
    /// Passing `None` asks whether the Location may be shown to the general public.
    ///
    /// Spec: 2.3.0 §mod_locations_location_object
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
            parking_places,
            directions,
            operator,
            suboperator,
            owner,
            facilities,
            time_zone,
            opening_times,
            images,
            energy_mix,
            help_phone,
            last_updated,
        );
        if self.publish && !self.publish_allowed_to.is_empty() {
            v.report_at(
                "publish_allowed_to",
                ViolationCode::Inconsistent,
                "this field may only be used when `publish` is false",
            );
        }
        // `EVSEParking.parking_id` "refers to a Parking object from the containing Location's
        // parking_places field by its id field".
        for (i, evse) in self.evses.iter().enumerate() {
            for (j, parking) in evse.parking.iter().enumerate() {
                if !self.parking_places.iter().any(|p| p.id == parking.parking_id) {
                    v.enter("evses");
                    v.enter(&i.to_string());
                    v.enter("parking");
                    v.enter(&j.to_string());
                    v.report_at(
                        "parking_id",
                        ViolationCode::Inconsistent,
                        format!(
                            "no Parking with id {:?} in this Location's parking_places",
                            parking.parking_id.as_str()
                        ),
                    );
                    v.leave();
                    v.leave();
                    v.leave();
                    v.leave();
                }
            }
        }
    }
}

/// The part that controls the power supply to a single EV in a single session.
///
/// > *An EVSE object has a list of Connectors which can not be used simultaneously: only one
/// > connector per EVSE can be used at the time.*
///
/// Spec: 2.3.0 §mod_locations_evse_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Evse {
    /// Uniquely identifies the EVSE within the CPO's platform. Never changed or renamed.
    ///
    /// > *Note that in order to fulfill both the requirement that an EVSE's `uid` be unique
    /// > within a CPO's platform and the requirement that EVSEs are never deleted, a CPO will
    /// > typically want to avoid using identifiers of the physical hardware for this `uid`.*
    pub uid: CiString<36>,
    /// The human-readable EVSE ID in the eMI3/IDACS format.
    ///
    /// Optional because *"if an `evse_id` is to be re-used in the real world, the `evse_id` can
    /// be removed from an EVSE object if the `status` is set to `REMOVED`"*.
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
    /// Available connectors on the EVSE. Cardinality `+`: at least one.
    pub connectors: Vec<Connector>,
    /// Level on which the Charge Point is located, in the locally displayed numbering scheme.
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
    /// Restrictions on who can charge at the EVSE, apart from those related to the vehicle type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub parking_restrictions: Vec<ParkingRestriction>,
    /// References to the parking spaces usable when charging at this EVSE. New in OCPI 2.3.0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub parking: Vec<EvseParking>,
    /// Links to images related to the EVSE.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub images: Vec<Image>,
    /// Names of the eMSPs whose contract-based payment options are accepted at this EVSE.
    ///
    /// > *Note that this field is added specifically to allow European CPOs to comply with a
    /// > regulatory requirement to provide this data to National Access Points (NAPs).*
    ///
    /// New in OCPI 2.3.0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub accepted_service_providers: Vec<OcpiString<50>>,
    /// Timestamp when this EVSE or one of its Connectors was last updated.
    pub last_updated: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Evse {
    /// Whether a `StartSession` for this EVSE must carry a `connector_id`.
    ///
    /// > *`START_SESSION_CONNECTOR_REQUIRED`: When a StartSession is sent to this EVSE, the MSP
    /// > is required to add the optional `connector_id` field in the StartSession object.*
    ///
    /// Spec: 2.3.0 §mod_locations_capability_enum
    #[must_use]
    pub fn requires_connector_id_on_start(&self) -> bool {
        self.capabilities.contains(&Capability::StartSessionConnectorRequired)
    }

    /// Whether this EVSE has the given capability.
    #[must_use]
    pub fn has(&self, capability: &Capability) -> bool {
        self.capabilities.contains(capability)
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
            status_schedule,
            capabilities,
            connectors,
            floor_level,
            coordinates,
            physical_reference,
            directions,
            parking_restrictions,
            parking,
            images,
            accepted_service_providers,
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

/// The socket, or cable and plug, available for the EV to use.
///
/// Spec: 2.3.0 §mod_locations_connector_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Connector {
    /// Identifier of the Connector within the EVSE.
    ///
    /// > *Two Connectors may have the same id as long as they do not belong to the same EVSE.*
    pub id: CiString<36>,
    /// The standard of the installed connector.
    pub standard: ConnectorType,
    /// The format (socket/cable) of the installed connector.
    pub format: ConnectorFormat,
    /// Whether the connector supplies AC or DC, and on how many phases.
    pub power_type: PowerType,
    /// Maximum voltage of the connector (line to neutral for `AC_3_PHASE`), in volt.
    pub max_voltage: i32,
    /// Maximum amperage of the connector, in ampere.
    pub max_amperage: i32,
    /// Maximum electric power this connector can deliver, in watt.
    ///
    /// > *When the maximum electric power is lower than the calculated value from `voltage` and
    /// > `amperage`, this value should be set.*
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_electric_power: Option<i32>,
    /// Identifiers of the currently valid charging tariffs.
    ///
    /// > *Multiple tariffs are possible, but only one of each `Tariff.type` can be active at the
    /// > same time. … For a "free of charge" tariff, this field should be set and point to a
    /// > defined "free of charge" tariff.*
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub tariff_ids: Vec<CiString<36>>,
    /// URL to the operator's terms and conditions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms_and_conditions: Option<Url>,
    /// Functionalities the connector is capable of. New in OCPI 2.3.0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub capabilities: Vec<ConnectorCapability>,
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
            capabilities,
            last_updated,
        );
        if self.max_voltage <= 0 {
            v.report_at("max_voltage", ViolationCode::OutOfRange, "must be a positive voltage");
        }
        if self.max_amperage <= 0 {
            v.report_at("max_amperage", ViolationCode::OutOfRange, "must be a positive amperage");
        }
        if let Some(power) = self.max_electric_power
            && power <= 0
        {
            v.report_at("max_electric_power", ViolationCode::OutOfRange, "must be positive");
        }
    }
}

/// A parking space a vehicle can be parked in while charging.
///
/// > *Parking objects were newly added in OCPI 2.3.0 … The purpose of Parking objects is to
/// > allow CPOs in the EU to comply with requirements in the EU's Alternative Fuel Infrastructure
/// > Regulation (AFIR). … All Locations receivers who are not NAPs are free to ignore Parking
/// > objects in the Location data that they receive.*
///
/// Spec: 2.3.0 §mod_locations_parking_object
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Parking {
    /// Identifier for this parking space, unique among the Parking objects of one Location.
    pub id: CiString<36>,
    /// A short identifier physically visible on-site, e.g. painted on the surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_reference: Option<OcpiString<12>>,
    /// The vehicle types the parking is designed to accommodate. Cardinality `+`.
    pub vehicle_types: Vec<VehicleType>,
    /// Maximum vehicle weight that can park at the EVSE, in kilograms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vehicle_weight: Option<Number>,
    /// Maximum vehicle height that can park at the EVSE, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vehicle_height: Option<Number>,
    /// Maximum vehicle length that can park at the EVSE, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vehicle_length: Option<Number>,
    /// Maximum vehicle width that can park at the EVSE, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vehicle_width: Option<Number>,
    /// The length of the parking space, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parking_space_length: Option<Number>,
    /// The width of the parking space, in centimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parking_space_width: Option<Number>,
    /// Whether vehicles loaded with dangerous substances may park at the EVSE.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dangerous_goods_allowed: Option<bool>,
    /// The direction in which the vehicle is to be parked next to the EVSE.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<ParkingDirection>,
    /// Whether a vehicle can stop, charge and proceed without reversing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_through: Option<bool>,
    /// Whether vehicles of a type not listed in `vehicle_types` are forbidden to park here.
    pub restricted_to_type: bool,
    /// Whether a reservation is required for parking at the EVSE.
    pub reservation_required: bool,
    /// A parking time limit, in minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_limit: Option<Number>,
    /// Whether the vehicle will be parked under a roof while charging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roofed: Option<bool>,
    /// Photos of the parking space.
    ///
    /// > *At least one photograph should be provided if the value of `vehicle_types` includes
    /// > the `DISABLED` vehicle type.*
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub images: Vec<Image>,
    /// Whether the parking space is lit by artificial lighting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lighting: Option<bool>,
    /// Whether a power outlet is available for a transport truck's load refrigeration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refrigeration_outlet: Option<bool>,
    /// Standards the parking space conforms to, e.g. PAS 1899 for accessible parking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub standards: Vec<CiString<36>>,
    /// Reference to an Alliance for Parking Data Standards (APDS) element describing this
    /// parking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apds_reference: Option<CiText>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Parking {
    /// Whether the spec expects the vehicle dimension fields to be filled.
    ///
    /// > *A value for this field should be provided unless the value of the `vehicle_types` field
    /// > contains no values other than `PERSONAL_VEHICLE` or `MOTORCYCLE`.*
    #[must_use]
    pub fn expects_dimensions(&self) -> bool {
        self.vehicle_types
            .iter()
            .any(|t| !matches!(t, VehicleType::PersonalVehicle | VehicleType::Motorcycle))
    }
}

impl Validate for Parking {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(
            self,
            v,
            id,
            physical_reference,
            vehicle_types,
            max_vehicle_weight,
            max_vehicle_height,
            max_vehicle_length,
            max_vehicle_width,
            parking_space_length,
            parking_space_width,
            direction,
            time_limit,
            images,
            standards,
            apds_reference,
        );
        if self.vehicle_types.is_empty() {
            v.report_at(
                "vehicle_types",
                ViolationCode::EmptyRequiredList,
                "a Parking has cardinality `+` vehicle_types: at least one is required",
            );
        }
        if self.vehicle_types.contains(&VehicleType::Disabled) && self.images.is_empty() {
            v.report_at(
                "images",
                ViolationCode::MissingConditional,
                "at least one photograph should be provided when vehicle_types includes DISABLED",
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------------------------

/// A geo location relevant to the Charge Point, with a name.
///
/// The geodetic system is WGS 84.
///
/// Spec: 2.3.0 §mod_locations_additionalgeolocation_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AdditionalGeoLocation {
    /// Latitude of the point in decimal degrees.
    pub latitude: OcpiString<10>,
    /// Longitude of the point in decimal degrees.
    pub longitude: OcpiString<11>,
    /// Name of the point in the local language or as written at the location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<DisplayText>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for AdditionalGeoLocation {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, latitude, longitude, name);
        check_coordinate(v, "latitude", self.latitude.as_str(), 2);
        check_coordinate(v, "longitude", self.longitude.as_str(), 3);
    }
}

/// The geo location of a Charge Point. The geodetic system is WGS 84.
///
/// > *Five decimal places is seen as a minimum for GPS coordinates of the Charge Point as this
/// > gives approximately 1 meter precision. More is always better.*
///
/// The spec types both fields as strings with a regex, not as numbers, so this crate keeps them
/// as strings: re-serialising must not turn `"50.770774"` into `50.770774`. Use
/// [`GeoLocation::latitude_decimal`] to compute with them.
///
/// Spec: 2.3.0 §mod_locations_geolocation_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GeoLocation {
    /// Latitude in decimal degrees; regex `-?[0-9]{1,2}\.[0-9]{5,7}`.
    pub latitude: OcpiString<10>,
    /// Longitude in decimal degrees; regex `-?[0-9]{1,3}\.[0-9]{5,7}`.
    pub longitude: OcpiString<11>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl GeoLocation {
    /// Creates a geo location from two coordinate strings.
    ///
    /// # Errors
    ///
    /// Returns [`crate::types::InvalidString`] if either value is too long or not printable.
    pub fn new(
        latitude: impl Into<String>,
        longitude: impl Into<String>,
    ) -> Result<Self, crate::types::InvalidString> {
        Ok(Self {
            latitude: OcpiString::new(latitude)?,
            longitude: OcpiString::new(longitude)?,
            extensions: Extensions::new(),
        })
    }

    /// The latitude as an exact decimal, if it parses.
    #[must_use]
    pub fn latitude_decimal(&self) -> Option<Number> {
        self.latitude.as_str().parse().ok()
    }

    /// The longitude as an exact decimal, if it parses.
    #[must_use]
    pub fn longitude_decimal(&self) -> Option<Number> {
        self.longitude.as_str().parse().ok()
    }
}

impl Validate for GeoLocation {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, latitude, longitude);
        check_coordinate(v, "latitude", self.latitude.as_str(), 2);
        check_coordinate(v, "longitude", self.longitude.as_str(), 3);
    }
}

/// Checks the coordinate regex the spec gives: `-?[0-9]{1,N}\.[0-9]{5,7}`.
fn check_coordinate(v: &mut Validator, field: &str, value: &str, max_int_digits: usize) {
    let body = value.strip_prefix('-').unwrap_or(value);
    let ok = match body.split_once('.') {
        Some((int, frac)) => {
            !int.is_empty()
                && int.len() <= max_int_digits
                && int.bytes().all(|b| b.is_ascii_digit())
                && (5..=7).contains(&frac.len())
                && frac.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    };
    if !ok {
        v.report_at(
            field,
            ViolationCode::IllegalCharacter,
            format!(
                "{value:?} does not match the OCPI coordinate format \
                 -?[0-9]{{1,{max_int_digits}}}.[0-9]{{5,7}}"
            ),
        );
    }
}

/// Details of a business: an operator, suboperator or owner.
///
/// Spec: 2.3.0 §mod_locations_businessdetails_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct BusinessDetails {
    /// Name of the operator.
    pub name: OcpiString<100>,
    /// Link to the operator's website.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<Url>,
    /// Image link to the operator's logo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<Image>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for BusinessDetails {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, name, website, logo);
    }
}

/// The energy mix and environmental impact of the energy supplied at a location or in a tariff.
///
/// Spec: 2.3.0 §mod_locations_energymix_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct EnergyMix {
    /// True if 100% from regenerative sources: CO2 and nuclear waste are zero.
    pub is_green_energy: bool,
    /// Energy sources of this location's tariff, as category and percentage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub energy_sources: Vec<EnergySource>,
    /// Nuclear waste and CO2 exhaust of this location's tariff.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub environ_impact: Vec<EnvironmentalImpact>,
    /// Name of the energy supplier delivering the energy for this location or tariff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supplier_name: Option<OcpiString<64>>,
    /// Name of the energy supplier's product/tariff plan used at this location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_product_name: Option<OcpiString<64>>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for EnergyMix {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, energy_sources, environ_impact, supplier_name, energy_product_name,);
        // "All given values of all categories should add up to 100 percent."
        if !self.energy_sources.is_empty() {
            let total: Number = self.energy_sources.iter().map(|s| s.percentage).sum();
            if total != Number::from(100u32) {
                v.report_at(
                    "energy_sources",
                    ViolationCode::Inconsistent,
                    format!("percentages add up to {total}, not 100"),
                );
            }
        }
    }
}

/// One energy source and its share of the mix.
///
/// Spec: 2.3.0 §mod_locations_energysource_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EnergySource {
    /// The type of energy source.
    pub source: EnergySourceCategory,
    /// Percentage of this source (0–100) in the mix.
    pub percentage: Number,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for EnergySource {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, source, percentage);
        if self.percentage < Number::ZERO || self.percentage > Number::from(100u32) {
            v.report_at("percentage", ViolationCode::OutOfRange, "must be between 0 and 100");
        }
    }
}

/// Waste produced or emitted per kWh.
///
/// Spec: 2.3.0 §mod_locations_environmentalimpact_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EnvironmentalImpact {
    /// The environmental impact category of this value.
    pub category: EnvironmentalImpactCategory,
    /// Amount of this portion in g/kWh.
    pub amount: Number,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for EnvironmentalImpact {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, category, amount);
    }
}

/// A link between an EVSE and a [`Parking`] object. New in OCPI 2.3.0.
///
/// Spec: 2.3.0 §mod_locations_evseparking_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EvseParking {
    /// The `id` of a [`Parking`] in the containing Location's `parking_places`.
    pub parking_id: CiString<36>,
    /// The position of the EVSE relative to the parking space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evse_position: Option<EvsePosition>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for EvseParking {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, parking_id, evse_position);
    }
}

/// One exceptional period for opening or access hours.
///
/// Spec: 2.3.0 §mod_locations_exceptionalperiod_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExceptionalPeriod {
    /// Begin of the exception, in UTC.
    pub period_begin: DateTime,
    /// End of the exception, in UTC.
    pub period_end: DateTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for ExceptionalPeriod {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, period_begin, period_end);
        if self.period_end < self.period_begin {
            v.report_at(
                "period_end",
                ViolationCode::Inconsistent,
                "the end of an exceptional period cannot precede its beginning",
            );
        }
    }
}

/// Opening and access hours of a location.
///
/// Spec: 2.3.0 §mod_locations_hours_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Hours {
    /// True to represent 24 hours a day and 7 days a week, except the given exceptions.
    pub twentyfourseven: bool,
    /// Regular weekday-based hours. Required to be non-empty when `twentyfourseven` is false.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub regular_hours: Vec<RegularHours>,
    /// Periods the station is operating/accessible, additional to `regular_hours`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub exceptional_openings: Vec<ExceptionalPeriod>,
    /// Periods the station is not operating/accessible, overriding everything else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub exceptional_closings: Vec<ExceptionalPeriod>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Hours {
    /// Whether the location is open at `instant`, given the location's UTC offset in seconds.
    ///
    /// Applies the spec's precedence: an exceptional closing beats an exceptional opening, which
    /// beats the regular hours.
    ///
    /// > *`exceptional_closings`: … Overwriting `regular_hours` and `exceptional_openings`.*
    #[must_use]
    pub fn is_open_at(&self, instant: DateTime, utc_offset_seconds: i32) -> bool {
        let in_period = |p: &ExceptionalPeriod| instant >= p.period_begin && instant < p.period_end;
        if self.exceptional_closings.iter().any(in_period) {
            return false;
        }
        if self.exceptional_openings.iter().any(in_period) {
            return true;
        }
        if self.twentyfourseven {
            return true;
        }
        let local = instant.as_offset_date_time() + time::Duration::seconds(i64::from(utc_offset_seconds));
        let weekday = local.weekday().number_from_monday();
        let Ok(now) = LocalTime::new(local.hour(), local.minute()) else {
            return false;
        };
        self.regular_hours.iter().any(|r| r.weekday == weekday && now.is_within(r.period_begin, r.period_end))
    }
}

impl Validate for Hours {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, regular_hours, exceptional_openings, exceptional_closings);
        if !self.twentyfourseven && self.regular_hours.is_empty() {
            v.report_at(
                "regular_hours",
                ViolationCode::MissingConditional,
                "when `twentyfourseven` is false this field must contain at least one entry",
            );
        }
    }
}

/// Regular recurring operation or access hours.
///
/// Spec: 2.3.0 §mod_locations_regularhours_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RegularHours {
    /// Number of the day in the week, from Monday (1) till Sunday (7).
    pub weekday: u8,
    /// Begin of the regular period, in local time.
    pub period_begin: LocalTime,
    /// End of the regular period, in local time. Must be later than `period_begin`.
    pub period_end: LocalTime,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for RegularHours {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, period_begin, period_end);
        if !(1..=7).contains(&self.weekday) {
            v.report_at(
                "weekday",
                ViolationCode::OutOfRange,
                format!("{} is not a day of the week: Monday (1) till Sunday (7)", self.weekday),
            );
        }
        // "Must be later than `period_begin`." Unlike TariffRestrictions, RegularHours does not
        // define a wrap-around, so this is a genuine constraint.
        if self.period_end <= self.period_begin {
            v.report_at(
                "period_end",
                ViolationCode::Inconsistent,
                format!("{} must be later than period_begin {}", self.period_end, self.period_begin),
            );
        }
    }
}

/// An image related to an EVSE, in terms of a file name or URL.
///
/// Spec: 2.3.0 §mod_locations_image_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Builder)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[builder(on(_, into))]
pub struct Image {
    /// URL from where the image data can be fetched through a web browser.
    pub url: Url,
    /// URL from where a thumbnail of the image can be fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<Url>,
    /// Describes what the image is used for.
    pub category: ImageCategory,
    /// Image type, e.g. `gif`, `jpeg`, `png`, `svg`.
    #[serde(rename = "type")]
    pub image_type: CiString<4>,
    /// Width of the full scale image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Height of the full scale image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    #[builder(default)]
    pub extensions: Extensions,
}

impl Validate for Image {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, url, thumbnail, category, image_type as "type");
        // int(5) in the property table.
        for (name, value) in [("width", self.width), ("height", self.height)] {
            if value.is_some_and(|x| x > 99_999) {
                v.report_at(name, ViolationCode::OutOfRange, "int(5): at most five digits");
            }
        }
    }
}

/// The set of values that identify a token to which a Location might be published.
///
/// > *At least one of the following fields SHALL be set: `uid`, `visual_number`, or `group_id`.
/// > When `uid` is set, `type` SHALL also be set. When `visual_number` is set, `issuer` SHALL
/// > also be set.*
///
/// Spec: 2.3.0 §mod_locations_publish_token_class
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
    /// Whether `candidate` matches **all the set fields** of this publish token.
    ///
    /// > *If the user of their app/website has provided information about his/her Token, and that
    /// > information matches all the fields of one of the PublishToken tokens in the list, then
    /// > they are allowed to show this location to their user.*
    ///
    /// A field this publish token leaves unset places no requirement on `candidate`.
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

/// A scheduled status period in the future.
///
/// > *The scheduled status is purely informational. When the status actually changes, the CPO
/// > must push an update to the EVSEs `status` field itself.*
///
/// Spec: 2.3.0 §mod_locations_statusschedule_class
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StatusSchedule {
    /// Begin of the scheduled period.
    pub period_begin: DateTime,
    /// End of the scheduled period, if known. A period MAY have no end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_end: Option<DateTime>,
    /// Status value during the scheduled period.
    pub status: Status,
    /// Undocumented JSON fields, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Validate for StatusSchedule {
    fn validate_in(&self, v: &mut Validator) {
        validate_fields!(self, v, period_begin, period_end, status);
        if self.period_end.is_some_and(|end| end < self.period_begin) {
            v.report_at(
                "period_end",
                ViolationCode::Inconsistent,
                "the end of a scheduled period cannot precede its beginning",
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------------------------

ocpi_open_enum! {
    /// The capabilities of an EVSE.
    ///
    /// Spec: 2.3.0 §mod_locations_capability_enum
    pub enum Capability {
        /// The EVSE supports charging profiles.
        ChargingProfileCapable = "CHARGING_PROFILE_CAPABLE",
        /// The EVSE supports charging preferences.
        ChargingPreferencesCapable = "CHARGING_PREFERENCES_CAPABLE",
        /// EVSE has a payment terminal that supports chip cards.
        ChipCardSupport = "CHIP_CARD_SUPPORT",
        /// EVSE has a payment terminal that supports contactless cards.
        ContactlessCardSupport = "CONTACTLESS_CARD_SUPPORT",
        /// EVSE has a payment terminal that accepts credit cards.
        CreditCardPayable = "CREDIT_CARD_PAYABLE",
        /// EVSE has a payment terminal that accepts debit cards.
        DebitCardPayable = "DEBIT_CARD_PAYABLE",
        /// EVSE has a payment terminal with a pin-code entry device.
        PedTerminal = "PED_TERMINAL",
        /// The EVSE can remotely be started/stopped.
        RemoteStartStopCapable = "REMOTE_START_STOP_CAPABLE",
        /// The EVSE can be reserved.
        Reservable = "RESERVABLE",
        /// Charging at this EVSE can be authorized with an RFID token.
        RfidReader = "RFID_READER",
        /// A `StartSession` for this EVSE must carry the optional `connector_id` field.
        StartSessionConnectorRequired = "START_SESSION_CONNECTOR_REQUIRED",
        /// This EVSE supports token groups: two or more tokens work as one.
        TokenGroupCapable = "TOKEN_GROUP_CAPABLE",
        /// Connectors have a mechanical lock the eMSP can request to be unlocked.
        UnlockCapable = "UNLOCK_CAPABLE",
    }
}

ocpi_open_enum! {
    /// Functionalities that a Connector may or may not support. New in OCPI 2.3.0.
    ///
    /// > *NOTE: these capabilities are meant to signal to eMSPs and their Drivers that a Driver
    /// > can indeed use these functionalities at a Connector. Mere support for a standard by the
    /// > charging hardware is not enough to warrant the presence of these capabilities.*
    ///
    /// Spec: 2.3.0 §mod_locations_connectorcapability_enum
    pub enum ConnectorCapability {
        /// Driver authentication with a contract certificate per ISO 15118-2.
        Iso151182PlugAndCharge = "ISO_15118_2_PLUG_AND_CHARGE",
        /// Driver authentication with a contract certificate per ISO 15118-20.
        Iso1511820PlugAndCharge = "ISO_15118_20_PLUG_AND_CHARGE",
    }
}

ocpi_enum! {
    /// The format of the connector: whether it is a socket or an attached cable.
    ///
    /// Spec: 2.3.0 §mod_locations_connectorformat_enum
    pub enum ConnectorFormat {
        /// The connector is a socket; the EV user needs to bring a fitting plug.
        Socket = "SOCKET",
        /// The connector is an attached cable; the EV user's car needs a fitting inlet.
        Cable = "CABLE",
    }
}

ocpi_open_enum! {
    /// The socket or plug standard of the charging point.
    ///
    /// This became an `OpenEnum` in OCPI 2.3.0 — in 2.2.1 it was a closed enum — which is the
    /// single most important reason not to reject unknown enum values: new plug standards appear
    /// faster than OCPI releases.
    ///
    /// Spec: 2.3.0 §mod_locations_connectortype_enum
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
        /// IEC 60309-2 Industrial Connector single phase 16 A (usually blue).
        Iec603092Single16 = "IEC_60309_2_single_16",
        /// IEC 60309-2 Industrial Connector three phases 16 A (usually red).
        Iec603092Three16 = "IEC_60309_2_three_16",
        /// IEC 60309-2 Industrial Connector three phases 32 A (usually red).
        Iec603092Three32 = "IEC_60309_2_three_32",
        /// IEC 60309-2 Industrial Connector three phases 64 A (usually red).
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
        /// The MegaWatt Charging System (MCS) connector developed by CharIN. New in 2.3.0.
        Mcs = "MCS",
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
        /// SAE J3400, also known as the North American Charging Standard (NACS).
        SaeJ3400 = "SAE_J3400",
        /// Tesla Connector "Roadster"-type (round, 4 pin).
        TeslaR = "TESLA_R",
        /// Tesla Connector "Model-S"-type (oval, 5 pin), mechanically compatible with SAE J3400.
        TeslaS = "TESLA_S",
    }
}

ocpi_enum! {
    /// Categories of energy sources.
    ///
    /// Spec: 2.3.0 §mod_locations_energysourcecategory_enum
    pub enum EnergySourceCategory {
        /// Nuclear power sources.
        Nuclear = "NUCLEAR",
        /// All kinds of fossil power sources.
        GeneralFossil = "GENERAL_FOSSIL",
        /// Fossil power from coal.
        Coal = "COAL",
        /// Fossil power from gas.
        Gas = "GAS",
        /// All kinds of regenerative power sources.
        GeneralGreen = "GENERAL_GREEN",
        /// Regenerative power from PV.
        Solar = "SOLAR",
        /// Regenerative power from wind turbines.
        Wind = "WIND",
        /// Regenerative power from water turbines.
        Water = "WATER",
    }
}

ocpi_open_enum! {
    /// Categories of environmental impact values.
    ///
    /// Spec: 2.3.0 §mod_locations_environmentalimpactcategory_enum
    pub enum EnvironmentalImpactCategory {
        /// Produced nuclear waste in grams per kilowatt-hour.
        NuclearWaste = "NUCLEAR_WASTE",
        /// Exhausted carbon dioxide in grams per kilowatt-hour.
        CarbonDioxide = "CARBON_DIOXIDE",
    }
}

ocpi_enum! {
    /// The position of an EVSE relative to the EVSE's parking space. New in OCPI 2.3.0.
    ///
    /// Spec: 2.3.0 §mod_locations_evseposition_enum
    pub enum EvsePosition {
        /// The EVSE is to the left of the vehicle.
        Left = "LEFT",
        /// The EVSE is to the right of the vehicle when parked.
        Right = "RIGHT",
        /// The EVSE is at the center of the impassable narrow end of a parking space.
        Center = "CENTER",
    }
}

ocpi_open_enum! {
    /// Facilities a charging location directly belongs to.
    ///
    /// Spec: 2.3.0 §mod_locations_facility_enum
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
        /// Sport facilities: gym, field etc.
        Sport = "SPORT",
        /// A recreation area.
        RecreationArea = "RECREATION_AREA",
        /// Located in, or close to, a park or nature reserve.
        Nature = "NATURE",
        /// A museum.
        Museum = "MUSEUM",
        /// A bike/e-bike/e-scooter sharing location.
        BikeSharing = "BIKE_SHARING",
        /// A bus stop.
        BusStop = "BUS_STOP",
        /// A taxi stand.
        TaxiStand = "TAXI_STAND",
        /// A tram stop/station.
        TramStop = "TRAM_STOP",
        /// A metro station.
        MetroStation = "METRO_STATION",
        /// A train station.
        TrainStation = "TRAIN_STATION",
        /// An airport.
        Airport = "AIRPORT",
        /// A parking lot.
        ParkingLot = "PARKING_LOT",
        /// A carpool parking.
        CarpoolParking = "CARPOOL_PARKING",
        /// A fuel station.
        FuelStation = "FUEL_STATION",
        /// Wifi or other type of internet available.
        Wifi = "WIFI",
    }
}

ocpi_open_enum! {
    /// The category of an image, so it can be used correctly in a presentation.
    ///
    /// Spec: 2.3.0 §mod_locations_imagecategory_enum
    pub enum ImageCategory {
        /// Photo of the physical device that contains one or more EVSEs.
        Charger = "CHARGER",
        /// Location entrance photo, showing the car entrance from the street side.
        Entrance = "ENTRANCE",
        /// Location overview photo.
        Location = "LOCATION",
        /// Logo of an associated roaming network.
        Network = "NETWORK",
        /// Logo of the charge point operator.
        Operator = "OPERATOR",
        /// Other.
        Other = "OTHER",
        /// Logo of the charge point owner, for example a local store.
        Owner = "OWNER",
    }
}

ocpi_enum! {
    /// The direction in which parking occurs relative to the approach roadway. New in 2.3.0.
    ///
    /// Spec: 2.3.0 §mod_locations_parkingdirection_enum
    pub enum ParkingDirection {
        /// Parking happens parallel to the roadway.
        Parallel = "PARALLEL",
        /// Parking happens perpendicular to the roadway.
        Perpendicular = "PERPENDICULAR",
        /// Parking happens at an angle to the roadway (echelon parking).
        Angle = "ANGLE",
    }
}

ocpi_open_enum! {
    /// Restrictions on the parking spot for different purposes.
    ///
    /// `EMPLOYEES`, `TAXIS` and `TENANTS` are new in OCPI 2.3.0.
    ///
    /// Spec: 2.3.0 §mod_locations_parkingrestriction_enum
    pub enum ParkingRestriction {
        /// Parking spot for customers or guests only.
        Customers = "CUSTOMERS",
        /// Reserved parking spot for disabled people with a valid ID.
        Disabled = "DISABLED",
        /// Parking only for people who work at the site the Location belongs to.
        Employees = "EMPLOYEES",
        /// Reserved parking spot for electric vehicles.
        EvOnly = "EV_ONLY",
        /// Parking spot only suitable for (electric) motorcycles or scooters.
        Motorcycles = "MOTORCYCLES",
        /// Parking is only allowed while plugged in (charging).
        Plugged = "PLUGGED",
        /// Parking only for taxi vehicles.
        Taxis = "TAXIS",
        /// Parking only for people who live in a complex the Location belongs to.
        Tenants = "TENANTS",
    }
}

ocpi_open_enum! {
    /// The general type of the charge point's location.
    ///
    /// Spec: 2.3.0 §mod_locations_parkingtype_enum
    pub enum ParkingType {
        /// A parking facility or rest area along a motorway, freeway, interstate or highway.
        AlongMotorway = "ALONG_MOTORWAY",
        /// Multistorey car park.
        ParkingGarage = "PARKING_GARAGE",
        /// A cleared area intended for parking vehicles, e.g. at supermarkets or bars.
        ParkingLot = "PARKING_LOT",
        /// Location is on the driveway of a house or building.
        OnDriveway = "ON_DRIVEWAY",
        /// Parking in public space along a street.
        OnStreet = "ON_STREET",
        /// Multistorey car park, mainly underground.
        UndergroundGarage = "UNDERGROUND_GARAGE",
    }
}

ocpi_enum! {
    /// Whether a connector supplies AC or DC, and on how many phases.
    ///
    /// Spec: 2.3.0 §mod_locations_powertype_enum
    pub enum PowerType {
        /// AC single phase.
        Ac1Phase = "AC_1_PHASE",
        /// AC two phases, only two of the three available phases connected.
        Ac2Phase = "AC_2_PHASE",
        /// AC two phases using a split phase system.
        Ac2PhaseSplit = "AC_2_PHASE_SPLIT",
        /// AC three phases.
        Ac3Phase = "AC_3_PHASE",
        /// Direct current.
        Dc = "DC",
    }
}

ocpi_enum! {
    /// The status of an EVSE.
    ///
    /// > *An EVSE is never deleted; a removed EVSE gets `status` `REMOVED`.*
    ///
    /// Spec: 2.3.0 §mod_locations_status_enum
    pub enum Status {
        /// The EVSE/Connector is able to start a new charging session.
        Available = "AVAILABLE",
        /// Not accessible because of a physical barrier, e.g. a car.
        Blocked = "BLOCKED",
        /// The EVSE/Connector is in use.
        Charging = "CHARGING",
        /// Not yet active, or temporarily unavailable, but not broken.
        Inoperative = "INOPERATIVE",
        /// Currently out of order; some parts may be broken or defective.
        OutOfOrder = "OUTOFORDER",
        /// Planned, will be operating soon.
        Planned = "PLANNED",
        /// Discontinued or removed.
        Removed = "REMOVED",
        /// Reserved for a particular EV driver and unavailable for other drivers.
        Reserved = "RESERVED",
        /// No status information available; also used when offline.
        Unknown = "UNKNOWN",
    }
}

ocpi_open_enum! {
    /// Which type of vehicles can use a certain EVSE. New in OCPI 2.3.0.
    ///
    /// Spec: 2.3.0 §mod_locations_vehicletype_enum
    pub enum VehicleType {
        /// A motorcycle. Approximate UNECE code: L.
        Motorcycle = "MOTORCYCLE",
        /// A personal vehicle, a passenger car. UNECE: M1.
        PersonalVehicle = "PERSONAL_VEHICLE",
        /// A personal vehicle with a trailer attached. UNECE: M1 + O.
        PersonalVehicleWithTrailer = "PERSONAL_VEHICLE_WITH_TRAILER",
        /// A light-duty van with a height smaller than 275 cm. UNECE: N1.
        Van = "VAN",
        /// A heavy-duty tractor unit without a trailer. UNECE: T.
        SemiTractor = "SEMI_TRACTOR",
        /// A heavy-duty truck without an articulation point. UNECE: N2/N3.
        Rigid = "RIGID",
        /// A heavy-duty truck with a trailer attached. UNECE: N2/N3 + O.
        TruckWithTrailer = "TRUCK_WITH_TRAILER",
        /// A bus or a motor coach. UNECE: M2/M3.
        Bus = "BUS",
        /// A vehicle with a permit for parking spaces for people with disabilities.
        Disabled = "DISABLED",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo() -> GeoLocation {
        GeoLocation::new("50.770774", "-126.104965").unwrap()
    }

    #[test]
    fn coordinate_format_is_checked_against_the_spec_regex() {
        assert!(geo().validate().is_ok());
        for (lat, lon) in [
            ("50.77", "-126.104965"),      // too few decimals
            ("50", "-126.104965"),         // no decimal point
            ("50.7707745678", "-126.1"),   // too many decimals, and too few
            ("50.770774", "-1261.104965"), // too many integer digits (also over string(11))
        ] {
            let g = GeoLocation {
                latitude: OcpiString::new_lenient(lat),
                longitude: OcpiString::new_lenient(lon),
                extensions: Extensions::new(),
            };
            assert!(g.validate().is_err(), "{lat}/{lon} should be reported");
        }
    }

    #[test]
    fn publish_allowed_to_requires_publish_false() {
        let mut loc = Location::builder()
            .country_code("NL")
            .party_id("TNM")
            .id("LOC1")
            .publish(true)
            .address("Street 1")
            .city("Amsterdam")
            .country("NLD")
            .coordinates(geo())
            .time_zone("Europe/Amsterdam")
            .last_updated("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .build();
        assert!(loc.validate().is_ok());

        loc.publish_allowed_to =
            vec![PublishTokenType { group_id: Some(CiString::new("G1").unwrap()), ..Default::default() }];
        let err = loc.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].pointer, "/publish_allowed_to");
        assert_eq!(err.as_slice()[0].code, ViolationCode::Inconsistent);
    }

    #[test]
    fn publish_token_matching_requires_all_set_fields_to_agree() {
        let allowed = PublishTokenType {
            visual_number: Some(OcpiString::new("12345").unwrap()),
            issuer: Some(OcpiString::new("TheNewMotion").unwrap()),
            ..Default::default()
        };
        let same = allowed.clone();
        let wrong_issuer =
            PublishTokenType { issuer: Some(OcpiString::new("Other").unwrap()), ..allowed.clone() };
        let extra_fields = PublishTokenType {
            uid: Some(CiString::new("ABC").unwrap()),
            token_type: Some(TokenType::Rfid),
            ..allowed.clone()
        };
        assert!(allowed.matches(&same));
        assert!(!allowed.matches(&wrong_issuer));
        assert!(allowed.matches(&extra_fields), "extra information on the candidate is fine");
    }

    #[test]
    fn publish_token_conditional_requirements_are_reported() {
        let only_uid = PublishTokenType { uid: Some(CiString::new("ABC").unwrap()), ..Default::default() };
        let err = only_uid.validate().unwrap_err();
        assert_eq!(err.as_slice()[0].pointer, "/type");
        assert!(PublishTokenType::default().validate().is_err(), "one of three must be set");
    }

    #[test]
    fn regular_hours_must_be_a_forward_interval_on_a_real_weekday() {
        let ok = RegularHours {
            weekday: 1,
            period_begin: "08:00".parse().unwrap(),
            period_end: "20:00".parse().unwrap(),
            extensions: Extensions::new(),
        };
        assert!(ok.validate().is_ok());
        let backwards = RegularHours { period_end: "07:00".parse().unwrap(), ..ok.clone() };
        assert!(backwards.validate().is_err());
        let no_such_day = RegularHours { weekday: 8, ..ok };
        assert!(no_such_day.validate().is_err());
    }

    #[test]
    fn hours_apply_the_precedence_the_spec_gives() {
        let dt = |s: &str| s.parse::<DateTime>().unwrap();
        let hours = Hours {
            twentyfourseven: true,
            regular_hours: vec![],
            exceptional_openings: vec![],
            exceptional_closings: vec![ExceptionalPeriod {
                period_begin: dt("2018-12-25T03:00:00Z"),
                period_end: dt("2018-12-25T05:00:00Z"),
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };
        assert!(hours.is_open_at(dt("2018-12-25T02:59:59Z"), 0));
        assert!(!hours.is_open_at(dt("2018-12-25T04:00:00Z"), 0), "closing beats 24/7");
    }

    #[test]
    fn twentyfourseven_false_needs_regular_hours() {
        let empty = Hours {
            twentyfourseven: false,
            regular_hours: vec![],
            exceptional_openings: vec![],
            exceptional_closings: vec![],
            extensions: Extensions::new(),
        };
        assert_eq!(empty.validate().unwrap_err().as_slice()[0].code, ViolationCode::MissingConditional);
    }

    #[test]
    fn evse_parking_must_point_at_a_parking_place_of_the_same_location() {
        let evse = Evse::builder()
            .uid("E1")
            .status(Status::Available)
            .connectors(vec![])
            .parking(vec![EvseParking {
                parking_id: CiString::new("P9").unwrap(),
                evse_position: None,
                extensions: Extensions::new(),
            }])
            .last_updated("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .build();
        let loc = Location::builder()
            .country_code("NL")
            .party_id("TNM")
            .id("LOC1")
            .publish(true)
            .address("Street 1")
            .city("Amsterdam")
            .country("NLD")
            .coordinates(geo())
            .time_zone("Europe/Amsterdam")
            .evses(vec![evse])
            .last_updated("2024-01-01T00:00:00Z".parse::<DateTime>().unwrap())
            .build();
        let err = loc.validate().unwrap_err();
        assert!(err.as_slice().iter().any(|x| x.pointer == "/evses/0/parking/0/parking_id"), "{err}");
        // The empty connector list is the other violation the same object has.
        assert!(err.as_slice().iter().any(|x| x.code == ViolationCode::EmptyRequiredList));
    }

    #[test]
    fn unknown_connector_types_survive_a_round_trip() {
        let json = r#"{"id":"1","standard":"nltnm-PLUG_X","format":"SOCKET","power_type":"DC","max_voltage":920,"max_amperage":400,"last_updated":"2024-01-01T00:00:00Z"}"#;
        let c: Connector = serde_json::from_str(json).unwrap();
        assert!(!c.standard.is_known());
        assert_eq!(serde_json::to_string(&c).unwrap(), json);
    }
}
