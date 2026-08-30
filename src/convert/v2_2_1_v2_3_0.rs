//! Conversions between OCPI 2.2.1 and OCPI 2.3.0.
//!
//! This is the bridge a hub needs today: 2.2.1 is what nearly everything in production speaks,
//! and 2.3.0 is what regulation (EU AFIR, North American tax reporting) is pushing parties
//! towards.
//!
//! Every field that OCPI 2.3.0 added is listed in [`v2_2_1`](crate::v2_2_1)'s module
//! documentation, and each one appears below either as a documented default (going forward) or as
//! a recorded [`Loss`](super::Loss) (going back).

use crate::types::Validate;
use crate::v2_2_1 as old;
use crate::v2_3_0 as new;

use super::{Converted, Downgrade, Lossy, Upgrade};

// -------------------------------------------------------------------------------------------
// Price
// -------------------------------------------------------------------------------------------

/// The tax name this crate writes when turning a 2.2.1 `incl_vat` into a 2.3.0 tax line.
///
/// A 2.2.1 `Price` says only "excluding VAT" and "including VAT"; it does not name the tax. `VAT`
/// is the name the specification itself uses for it throughout the 2.2.1 text.
pub const IMPLIED_VAT_NAME: &str = "VAT";

impl Upgrade<new::types::Price> for old::types::Price {
    /// `before_taxes` is the pre-VAT amount; when `incl_vat` is present, the difference becomes a
    /// single [`TaxAmount`](crate::v2_3_0::types::TaxAmount) named `VAT`.
    ///
    /// A 2.2.1 price with no `incl_vat` becomes a 2.3.0 price with no taxes, which is exactly the
    /// same statement: the amount of tax is not given.
    fn upgrade(self) -> Converted<new::types::Price> {
        let mut taxes = Vec::new();
        if let Some(incl) = self.incl_vat {
            let amount = incl - self.excl_vat;
            taxes.push(new::types::TaxAmount {
                name: crate::types::OcpiText::new_lenient(IMPLIED_VAT_NAME),
                account_number: None,
                percentage: None,
                amount,
                extensions: crate::types::Extensions::new(),
            });
        }
        Converted::lossless(new::types::Price {
            before_taxes: self.excl_vat,
            taxes,
            extensions: self.extensions,
        })
    }
}

impl Downgrade<old::types::Price> for new::types::Price {
    /// `excl_vat` is `before_taxes`; `incl_vat` is the sum of every tax on top of it.
    ///
    /// The 2.2.1 shape can hold one number, so the *names*, percentages and account numbers of
    /// the individual taxes are lost. That matters in Canada, where a receipt must itemise GST
    /// and QST separately, so each one is reported.
    fn downgrade(self) -> Converted<old::types::Price> {
        let mut lossy = Lossy::none();
        let incl_vat = if self.taxes.is_empty() { None } else { Some(self.after_taxes()) };
        for (i, tax) in self.taxes.iter().enumerate() {
            let carries_detail = tax.percentage.is_some() || tax.account_number.is_some();
            if self.taxes.len() > 1 || tax.name.as_str() != IMPLIED_VAT_NAME || carries_detail {
                lossy.record(
                    format!("/taxes/{i}"),
                    format!(
                        "the tax {:?} was folded into `incl_vat`; OCPI 2.2.1 has no way to name \
                         or itemise a tax",
                        tax.name.as_str()
                    ),
                );
            }
        }
        Converted::new(
            old::types::Price { excl_vat: self.before_taxes, incl_vat, extensions: self.extensions },
            lossy,
        )
    }
}

// -------------------------------------------------------------------------------------------
// Role
// -------------------------------------------------------------------------------------------

impl Upgrade<new::types::Role> for old::types::Role {
    /// `HUB` has no counterpart: OCPI 2.3.0 removed it and identifies a hub through
    /// `Credentials.hub_party_id` instead. It maps to `OTHER`, and the
    /// [`Credentials`](crate::v2_2_1::credentials::Credentials) conversion moves the information
    /// to where 2.3.0 keeps it.
    fn upgrade(self) -> Converted<new::types::Role> {
        use new::types::Role as N;
        use old::types::Role as O;
        let mut lossy = Lossy::none();
        let value = match self {
            O::Cpo => N::Cpo,
            O::Emsp => N::Emsp,
            O::Nap => N::Nap,
            O::Nsp => N::Nsp,
            O::Other => N::Other,
            O::Scsp => N::Scsp,
            O::Hub => {
                lossy.record(
                    "",
                    "the HUB role was removed in OCPI 2.3.0 and became OTHER; a hub is identified \
                     by `Credentials.hub_party_id` there",
                );
                N::Other
            }
        };
        Converted::new(value, lossy)
    }
}

impl Downgrade<old::types::Role> for new::types::Role {
    /// Total: every 2.3.0 role exists in 2.2.1.
    fn downgrade(self) -> Converted<old::types::Role> {
        use new::types::Role as N;
        use old::types::Role as O;
        Converted::lossless(match self {
            N::Cpo => O::Cpo,
            N::Emsp => O::Emsp,
            N::Nap => O::Nap,
            N::Nsp => O::Nsp,
            N::Other => O::Other,
            N::Scsp => O::Scsp,
        })
    }
}

// -------------------------------------------------------------------------------------------
// Enumerations that were opened in 2.3.0
//
// Both sides keep the wire value verbatim, so these conversions are total in both directions:
// a 2.3.0 `ConnectorType::Mcs` becomes a 2.2.1 `ConnectorType::Custom("MCS")`, which is the same
// string on the wire, and upgrades straight back.
// -------------------------------------------------------------------------------------------

macro_rules! convert_by_wire_value {
    ($($old:path => $new:path),* $(,)?) => {$(
        impl Upgrade<$new> for $old {
            fn upgrade(self) -> Converted<$new> {
                Converted::lossless(<$new>::from(self.as_str()))
            }
        }
        impl Downgrade<$old> for $new {
            fn downgrade(self) -> Converted<$old> {
                Converted::lossless(<$old>::from(self.as_str()))
            }
        }
    )*};
}

convert_by_wire_value! {
    old::locations::ConnectorType => new::locations::ConnectorType,
    old::locations::ParkingRestriction => new::locations::ParkingRestriction,
    old::tokens::TokenType => new::tokens::TokenType,
}

// -------------------------------------------------------------------------------------------
// Locations
// -------------------------------------------------------------------------------------------

impl Upgrade<new::locations::PublishTokenType> for old::locations::PublishTokenType {
    fn upgrade(self) -> Converted<new::locations::PublishTokenType> {
        let token_type = self.token_type.map(|t| t.upgrade().value);
        Converted::lossless(new::locations::PublishTokenType {
            uid: self.uid,
            token_type,
            visual_number: self.visual_number,
            issuer: self.issuer,
            group_id: self.group_id,
            extensions: self.extensions,
        })
    }
}

impl Downgrade<old::locations::PublishTokenType> for new::locations::PublishTokenType {
    fn downgrade(self) -> Converted<old::locations::PublishTokenType> {
        let token_type = self.token_type.map(|t| t.downgrade().value);
        Converted::lossless(old::locations::PublishTokenType {
            uid: self.uid,
            token_type,
            visual_number: self.visual_number,
            issuer: self.issuer,
            group_id: self.group_id,
            extensions: self.extensions,
        })
    }
}

impl Upgrade<new::locations::Connector> for old::locations::Connector {
    /// `capabilities` is new in 2.3.0 and starts empty: a 2.2.1 CPO said nothing about Plug &
    /// Charge support, and claiming it on their behalf would be wrong.
    fn upgrade(self) -> Converted<new::locations::Connector> {
        Converted::lossless(new::locations::Connector {
            id: self.id,
            standard: self.standard.upgrade().value,
            format: self.format,
            power_type: self.power_type,
            max_voltage: self.max_voltage,
            max_amperage: self.max_amperage,
            max_electric_power: self.max_electric_power,
            tariff_ids: self.tariff_ids,
            terms_and_conditions: self.terms_and_conditions,
            capabilities: Vec::new(),
            last_updated: self.last_updated,
            extensions: self.extensions,
        })
    }
}

impl Downgrade<old::locations::Connector> for new::locations::Connector {
    fn downgrade(self) -> Converted<old::locations::Connector> {
        let mut lossy = Lossy::none();
        if !self.capabilities.is_empty() {
            lossy.record(
                "/capabilities",
                "OCPI 2.2.1 has no Connector.capabilities, so ISO 15118 Plug & Charge support \
                 cannot be expressed",
            );
        }
        Converted::new(
            old::locations::Connector {
                id: self.id,
                standard: self.standard.downgrade().value,
                format: self.format,
                power_type: self.power_type,
                max_voltage: self.max_voltage,
                max_amperage: self.max_amperage,
                max_electric_power: self.max_electric_power,
                tariff_ids: self.tariff_ids,
                terms_and_conditions: self.terms_and_conditions,
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

impl Upgrade<new::locations::Evse> for old::locations::Evse {
    fn upgrade(self) -> Converted<new::locations::Evse> {
        let mut lossy = Lossy::none();
        let mut connectors = Vec::with_capacity(self.connectors.len());
        for (i, connector) in self.connectors.into_iter().enumerate() {
            let converted = connector.upgrade();
            lossy.absorb(&format!("/connectors/{i}"), converted.lossy);
            connectors.push(converted.value);
        }
        Converted::new(
            new::locations::Evse {
                uid: self.uid,
                evse_id: self.evse_id,
                status: self.status,
                status_schedule: self.status_schedule,
                capabilities: self.capabilities,
                connectors,
                floor_level: self.floor_level,
                coordinates: self.coordinates,
                physical_reference: self.physical_reference,
                directions: self.directions,
                parking_restrictions: self
                    .parking_restrictions
                    .into_iter()
                    .map(|p| p.upgrade().value)
                    .collect(),
                parking: Vec::new(),
                images: self.images,
                accepted_service_providers: Vec::new(),
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

impl Downgrade<old::locations::Evse> for new::locations::Evse {
    fn downgrade(self) -> Converted<old::locations::Evse> {
        let mut lossy = Lossy::none();
        if !self.parking.is_empty() {
            lossy.record(
                "/parking",
                format!(
                    "{} EVSEParking reference(s) dropped: OCPI 2.2.1 has no Parking object, which \
                     is what EU AFIR reporting to a National Access Point needs",
                    self.parking.len()
                ),
            );
        }
        if !self.accepted_service_providers.is_empty() {
            lossy.record(
                "/accepted_service_providers",
                "OCPI 2.2.1 has no EVSE.accepted_service_providers; the list of eMSPs accepted at \
                 this EVSE cannot be expressed",
            );
        }
        let mut connectors = Vec::with_capacity(self.connectors.len());
        for (i, connector) in self.connectors.into_iter().enumerate() {
            let converted = connector.downgrade();
            lossy.absorb(&format!("/connectors/{i}"), converted.lossy);
            connectors.push(converted.value);
        }
        Converted::new(
            old::locations::Evse {
                uid: self.uid,
                evse_id: self.evse_id,
                status: self.status,
                status_schedule: self.status_schedule,
                capabilities: self.capabilities,
                connectors,
                floor_level: self.floor_level,
                coordinates: self.coordinates,
                physical_reference: self.physical_reference,
                directions: self.directions,
                parking_restrictions: self
                    .parking_restrictions
                    .into_iter()
                    .map(|p| p.downgrade().value)
                    .collect(),
                images: self.images,
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

impl Upgrade<new::locations::Location> for old::locations::Location {
    fn upgrade(self) -> Converted<new::locations::Location> {
        let mut lossy = Lossy::none();
        let mut evses = Vec::with_capacity(self.evses.len());
        for (i, evse) in self.evses.into_iter().enumerate() {
            let converted = evse.upgrade();
            lossy.absorb(&format!("/evses/{i}"), converted.lossy);
            evses.push(converted.value);
        }
        let mut publish_allowed_to = Vec::with_capacity(self.publish_allowed_to.len());
        for token in self.publish_allowed_to {
            publish_allowed_to.push(token.upgrade().value);
        }
        Converted::new(
            new::locations::Location {
                country_code: self.country_code,
                party_id: self.party_id,
                id: self.id,
                publish: self.publish,
                publish_allowed_to,
                name: self.name,
                address: self.address,
                city: self.city,
                postal_code: self.postal_code,
                state: self.state,
                country: self.country,
                coordinates: self.coordinates,
                related_locations: self.related_locations,
                parking_type: self.parking_type,
                evses,
                parking_places: Vec::new(),
                directions: self.directions,
                operator: self.operator,
                suboperator: self.suboperator,
                owner: self.owner,
                facilities: self.facilities,
                time_zone: self.time_zone,
                opening_times: self.opening_times,
                charging_when_closed: self.charging_when_closed,
                images: self.images,
                energy_mix: self.energy_mix,
                help_phone: None,
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

impl Downgrade<old::locations::Location> for new::locations::Location {
    fn downgrade(self) -> Converted<old::locations::Location> {
        let mut lossy = Lossy::none();
        if !self.parking_places.is_empty() {
            lossy.record(
                "/parking_places",
                format!(
                    "{} Parking object(s) dropped: OCPI 2.2.1 has no such object",
                    self.parking_places.len()
                ),
            );
        }
        if self.help_phone.is_some() {
            lossy.record("/help_phone", "OCPI 2.2.1 has no Location.help_phone");
        }
        let mut evses = Vec::with_capacity(self.evses.len());
        for (i, evse) in self.evses.into_iter().enumerate() {
            let converted = evse.downgrade();
            lossy.absorb(&format!("/evses/{i}"), converted.lossy);
            evses.push(converted.value);
        }
        let publish_allowed_to = self.publish_allowed_to.into_iter().map(|t| t.downgrade().value).collect();
        Converted::new(
            old::locations::Location {
                country_code: self.country_code,
                party_id: self.party_id,
                id: self.id,
                publish: self.publish,
                publish_allowed_to,
                name: self.name,
                address: self.address,
                city: self.city,
                postal_code: self.postal_code,
                state: self.state,
                country: self.country,
                coordinates: self.coordinates,
                related_locations: self.related_locations,
                parking_type: self.parking_type,
                evses,
                directions: self.directions,
                operator: self.operator,
                suboperator: self.suboperator,
                owner: self.owner,
                facilities: self.facilities,
                time_zone: self.time_zone,
                opening_times: self.opening_times,
                charging_when_closed: self.charging_when_closed,
                images: self.images,
                energy_mix: self.energy_mix,
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

// -------------------------------------------------------------------------------------------
// Tariffs
// -------------------------------------------------------------------------------------------

impl Upgrade<new::tariffs::Tariff> for old::tariffs::Tariff {
    /// `tax_included` becomes `NO`.
    ///
    /// This is not a guess: a 2.2.1 `PriceComponent.price` is defined as *"Price per unit
    /// (excl. VAT) for this dimension"*, so every amount in a 2.2.1 Tariff is pre-tax by
    /// construction, which is exactly what `NO` means.
    ///
    /// `preauthorize_amount` starts absent — 2.2.1 has no Payments module to preauthorize for.
    fn upgrade(self) -> Converted<new::tariffs::Tariff> {
        let to_limit = |p: old::types::Price| new::tariffs::PriceLimit {
            before_taxes: p.excl_vat,
            after_taxes: p.incl_vat,
            extensions: p.extensions,
        };
        Converted::lossless(new::tariffs::Tariff {
            country_code: self.country_code,
            party_id: self.party_id,
            id: self.id,
            currency: self.currency,
            tariff_type: self.tariff_type,
            tariff_alt_text: self.tariff_alt_text,
            tariff_alt_url: self.tariff_alt_url,
            min_price: self.min_price.map(to_limit),
            max_price: self.max_price.map(to_limit),
            preauthorize_amount: None,
            elements: self.elements,
            tax_included: new::tariffs::TaxIncluded::No,
            start_date_time: self.start_date_time,
            end_date_time: self.end_date_time,
            energy_mix: self.energy_mix,
            last_updated: self.last_updated,
            extensions: self.extensions,
        })
    }
}

impl Downgrade<old::tariffs::Tariff> for new::tariffs::Tariff {
    /// A 2.2.1 Tariff's prices are excluding VAT by definition, so a 2.3.0 Tariff that says
    /// `tax_included: YES` **cannot be represented**: the same numbers would mean a different
    /// amount of money. That is recorded as a loss rather than silently reinterpreted.
    fn downgrade(self) -> Converted<old::tariffs::Tariff> {
        let mut lossy = Lossy::none();
        match self.tax_included {
            new::tariffs::TaxIncluded::No => {}
            new::tariffs::TaxIncluded::Yes => lossy.record(
                "/tax_included",
                "this Tariff's prices include tax, but a 2.2.1 `PriceComponent.price` is defined \
                 as excluding VAT; the receiving party will read the same numbers as pre-tax \
                 amounts",
            ),
            new::tariffs::TaxIncluded::NotApplicable => lossy.record(
                "/tax_included",
                "N/A (no taxes are applicable) cannot be expressed in OCPI 2.2.1, which always \
                 treats prices as excluding VAT",
            ),
        }
        if self.preauthorize_amount.is_some() {
            lossy.record(
                "/preauthorize_amount",
                "OCPI 2.2.1 has no Payments module, so the preauthorization amount is dropped",
            );
        }
        let mut to_price = |p: new::tariffs::PriceLimit, field: &str| {
            if p.after_taxes.is_none() {
                lossy.record(
                    format!("/{field}"),
                    "the pre-tax limit is kept as `excl_vat`; OCPI 2.2.1 has no separate \
                     after-tax bound",
                );
            }
            old::types::Price { excl_vat: p.before_taxes, incl_vat: p.after_taxes, extensions: p.extensions }
        };
        let min_price = self.min_price.map(|p| to_price(p, "min_price"));
        let max_price = self.max_price.map(|p| to_price(p, "max_price"));
        Converted::new(
            old::tariffs::Tariff {
                country_code: self.country_code,
                party_id: self.party_id,
                id: self.id,
                currency: self.currency,
                tariff_type: self.tariff_type,
                tariff_alt_text: self.tariff_alt_text,
                tariff_alt_url: self.tariff_alt_url,
                min_price,
                max_price,
                elements: self.elements,
                start_date_time: self.start_date_time,
                end_date_time: self.end_date_time,
                energy_mix: self.energy_mix,
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

// -------------------------------------------------------------------------------------------
// Tokens
// -------------------------------------------------------------------------------------------

impl Upgrade<new::tokens::Token> for old::tokens::Token {
    fn upgrade(self) -> Converted<new::tokens::Token> {
        Converted::lossless(new::tokens::Token {
            country_code: self.country_code,
            party_id: self.party_id,
            uid: self.uid,
            token_type: self.token_type.upgrade().value,
            contract_id: self.contract_id,
            visual_number: self.visual_number,
            issuer: self.issuer,
            group_id: self.group_id,
            valid: self.valid,
            whitelist: self.whitelist,
            language: self.language,
            default_profile_type: self.default_profile_type,
            energy_contract: self.energy_contract,
            last_updated: self.last_updated,
            extensions: self.extensions,
        })
    }
}

impl Downgrade<old::tokens::Token> for new::tokens::Token {
    fn downgrade(self) -> Converted<old::tokens::Token> {
        Converted::lossless(old::tokens::Token {
            country_code: self.country_code,
            party_id: self.party_id,
            uid: self.uid,
            token_type: self.token_type.downgrade().value,
            contract_id: self.contract_id,
            visual_number: self.visual_number,
            issuer: self.issuer,
            group_id: self.group_id,
            valid: self.valid,
            whitelist: self.whitelist,
            language: self.language,
            default_profile_type: self.default_profile_type,
            energy_contract: self.energy_contract,
            last_updated: self.last_updated,
            extensions: self.extensions,
        })
    }
}

// -------------------------------------------------------------------------------------------
// CDRs and Sessions
// -------------------------------------------------------------------------------------------

impl Upgrade<new::cdrs::CdrToken> for old::cdrs::CdrToken {
    fn upgrade(self) -> Converted<new::cdrs::CdrToken> {
        Converted::lossless(new::cdrs::CdrToken {
            country_code: self.country_code,
            party_id: self.party_id,
            uid: self.uid,
            token_type: self.token_type.upgrade().value,
            contract_id: self.contract_id,
            extensions: self.extensions,
        })
    }
}

impl Downgrade<old::cdrs::CdrToken> for new::cdrs::CdrToken {
    fn downgrade(self) -> Converted<old::cdrs::CdrToken> {
        Converted::lossless(old::cdrs::CdrToken {
            country_code: self.country_code,
            party_id: self.party_id,
            uid: self.uid,
            token_type: self.token_type.downgrade().value,
            contract_id: self.contract_id,
            extensions: self.extensions,
        })
    }
}

impl Upgrade<new::cdrs::CdrLocation> for old::cdrs::CdrLocation {
    fn upgrade(self) -> Converted<new::cdrs::CdrLocation> {
        Converted::lossless(new::cdrs::CdrLocation {
            id: self.id,
            name: self.name,
            address: self.address,
            city: self.city,
            postal_code: self.postal_code,
            state: self.state,
            country: self.country,
            coordinates: self.coordinates,
            evse_uid: self.evse_uid,
            evse_id: self.evse_id,
            connector_id: self.connector_id,
            connector_standard: self.connector_standard.upgrade().value,
            connector_format: self.connector_format,
            connector_power_type: self.connector_power_type,
            extensions: self.extensions,
        })
    }
}

impl Downgrade<old::cdrs::CdrLocation> for new::cdrs::CdrLocation {
    fn downgrade(self) -> Converted<old::cdrs::CdrLocation> {
        Converted::lossless(old::cdrs::CdrLocation {
            id: self.id,
            name: self.name,
            address: self.address,
            city: self.city,
            postal_code: self.postal_code,
            state: self.state,
            country: self.country,
            coordinates: self.coordinates,
            evse_uid: self.evse_uid,
            evse_id: self.evse_id,
            connector_id: self.connector_id,
            connector_standard: self.connector_standard.downgrade().value,
            connector_format: self.connector_format,
            connector_power_type: self.connector_power_type,
            extensions: self.extensions,
        })
    }
}

/// Converts an optional price field, lifting any loss into the parent's coordinates.
macro_rules! price_field {
    ($lossy:ident, $field:literal, $value:expr, $dir:ident) => {
        $value.map(|p| {
            let converted = Downgrade::<old::types::Price>::$dir(p);
            $lossy.absorb(concat!("/", $field), converted.lossy);
            converted.value
        })
    };
}

impl Upgrade<new::cdrs::Cdr> for old::cdrs::Cdr {
    fn upgrade(self) -> Converted<new::cdrs::Cdr> {
        let mut lossy = Lossy::none();
        let mut tariffs = Vec::with_capacity(self.tariffs.len());
        for (i, tariff) in self.tariffs.into_iter().enumerate() {
            let converted = tariff.upgrade();
            lossy.absorb(&format!("/tariffs/{i}"), converted.lossy);
            tariffs.push(converted.value);
        }
        let up = |p: old::types::Price| Upgrade::<new::types::Price>::upgrade(p).value;
        Converted::new(
            new::cdrs::Cdr {
                country_code: self.country_code,
                party_id: self.party_id,
                id: self.id,
                start_date_time: self.start_date_time,
                end_date_time: self.end_date_time,
                session_id: self.session_id,
                cdr_token: self.cdr_token.upgrade().value,
                auth_method: self.auth_method,
                authorization_reference: self.authorization_reference,
                // 2.2.1 has no Bookings module, so there is nothing to carry over.
                #[cfg(feature = "bookings")]
                booking_id: None,
                cdr_location: self.cdr_location.upgrade().value,
                meter_id: self.meter_id,
                currency: self.currency,
                tariffs,
                charging_periods: self.charging_periods,
                signed_data: self.signed_data,
                total_cost: up(self.total_cost),
                total_fixed_cost: self.total_fixed_cost.map(up),
                total_energy: self.total_energy,
                total_energy_cost: self.total_energy_cost.map(up),
                total_time: self.total_time,
                total_time_cost: self.total_time_cost.map(up),
                total_parking_time: self.total_parking_time,
                total_parking_cost: self.total_parking_cost.map(up),
                total_reservation_cost: self.total_reservation_cost.map(up),
                remark: self.remark,
                invoice_reference_id: self.invoice_reference_id,
                credit: self.credit,
                credit_reference_id: self.credit_reference_id,
                home_charging_compensation: self.home_charging_compensation,
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

impl Downgrade<old::cdrs::Cdr> for new::cdrs::Cdr {
    fn downgrade(self) -> Converted<old::cdrs::Cdr> {
        let mut lossy = Lossy::none();
        let mut tariffs = Vec::with_capacity(self.tariffs.len());
        for (i, tariff) in self.tariffs.into_iter().enumerate() {
            let converted = tariff.downgrade();
            lossy.absorb(&format!("/tariffs/{i}"), converted.lossy);
            tariffs.push(converted.value);
        }
        let total_cost = {
            let converted = self.total_cost.downgrade();
            lossy.absorb("/total_cost", converted.lossy);
            converted.value
        };
        let total_fixed_cost = price_field!(lossy, "total_fixed_cost", self.total_fixed_cost, downgrade);
        let total_energy_cost = price_field!(lossy, "total_energy_cost", self.total_energy_cost, downgrade);
        let total_time_cost = price_field!(lossy, "total_time_cost", self.total_time_cost, downgrade);
        let total_parking_cost =
            price_field!(lossy, "total_parking_cost", self.total_parking_cost, downgrade);
        let total_reservation_cost =
            price_field!(lossy, "total_reservation_cost", self.total_reservation_cost, downgrade);
        Converted::new(
            old::cdrs::Cdr {
                country_code: self.country_code,
                party_id: self.party_id,
                id: self.id,
                start_date_time: self.start_date_time,
                end_date_time: self.end_date_time,
                session_id: self.session_id,
                cdr_token: self.cdr_token.downgrade().value,
                auth_method: self.auth_method,
                authorization_reference: self.authorization_reference,
                cdr_location: self.cdr_location.downgrade().value,
                meter_id: self.meter_id,
                currency: self.currency,
                tariffs,
                charging_periods: self.charging_periods,
                signed_data: self.signed_data,
                total_cost,
                total_fixed_cost,
                total_energy: self.total_energy,
                total_energy_cost,
                total_time: self.total_time,
                total_time_cost,
                total_parking_time: self.total_parking_time,
                total_parking_cost,
                total_reservation_cost,
                remark: self.remark,
                invoice_reference_id: self.invoice_reference_id,
                credit: self.credit,
                credit_reference_id: self.credit_reference_id,
                home_charging_compensation: self.home_charging_compensation,
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

impl Upgrade<new::sessions::Session> for old::sessions::Session {
    fn upgrade(self) -> Converted<new::sessions::Session> {
        Converted::lossless(new::sessions::Session {
            country_code: self.country_code,
            party_id: self.party_id,
            id: self.id,
            start_date_time: self.start_date_time,
            end_date_time: self.end_date_time,
            kwh: self.kwh,
            cdr_token: self.cdr_token.upgrade().value,
            auth_method: self.auth_method,
            authorization_reference: self.authorization_reference,
            location_id: self.location_id,
            evse_uid: self.evse_uid,
            connector_id: self.connector_id,
            meter_id: self.meter_id,
            currency: self.currency,
            charging_periods: self.charging_periods,
            total_cost: self.total_cost.map(|p| Upgrade::<new::types::Price>::upgrade(p).value),
            status: self.status,
            last_updated: self.last_updated,
            extensions: self.extensions,
        })
    }
}

impl Downgrade<old::sessions::Session> for new::sessions::Session {
    fn downgrade(self) -> Converted<old::sessions::Session> {
        let mut lossy = Lossy::none();
        let total_cost = price_field!(lossy, "total_cost", self.total_cost, downgrade);
        Converted::new(
            old::sessions::Session {
                country_code: self.country_code,
                party_id: self.party_id,
                id: self.id,
                start_date_time: self.start_date_time,
                end_date_time: self.end_date_time,
                kwh: self.kwh,
                cdr_token: self.cdr_token.downgrade().value,
                auth_method: self.auth_method,
                authorization_reference: self.authorization_reference,
                location_id: self.location_id,
                evse_uid: self.evse_uid,
                connector_id: self.connector_id,
                meter_id: self.meter_id,
                currency: self.currency,
                charging_periods: self.charging_periods,
                total_cost,
                status: self.status,
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

// -------------------------------------------------------------------------------------------
// Credentials
// -------------------------------------------------------------------------------------------

impl Upgrade<new::credentials::Credentials> for old::credentials::Credentials {
    /// A 2.2.1 `HUB` role becomes 2.3.0's `hub_party_id`, and the role entry is dropped: that is
    /// exactly where OCPI 2.3.0 moved the information.
    ///
    /// > *A Platform that supports Hub functionality with the Message routing headers SHALL give
    /// > the country code and party ID of the Hub in the `hub_party_id` field.*
    fn upgrade(self) -> Converted<new::credentials::Credentials> {
        let mut lossy = Lossy::none();
        let hub_party_id =
            self.roles.iter().find(|r| r.role == old::types::Role::Hub).map(|r| r.party().to_hub_party_id());
        let mut roles = Vec::with_capacity(self.roles.len());
        for (i, role) in self.roles.into_iter().enumerate() {
            if role.role == old::types::Role::Hub {
                // The information is not lost: it moved to `hub_party_id`.
                continue;
            }
            let converted = Upgrade::<new::types::Role>::upgrade(role.role);
            lossy.absorb(&format!("/roles/{i}/role"), converted.lossy);
            roles.push(new::credentials::CredentialsRole {
                role: converted.value,
                business_details: role.business_details,
                party_id: role.party_id,
                country_code: role.country_code,
                extensions: role.extensions,
            });
        }
        Converted::new(
            new::credentials::Credentials {
                token: self.token,
                url: self.url,
                hub_party_id,
                roles,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

impl Downgrade<old::credentials::Credentials> for new::credentials::Credentials {
    /// A 2.3.0 `hub_party_id` becomes a 2.2.1 `HUB` role.
    ///
    /// The 2.2.1 role needs `business_details`, which 2.3.0 does not carry for the hub party
    /// itself; the first role's details are reused and the substitution is reported.
    fn downgrade(self) -> Converted<old::credentials::Credentials> {
        let mut lossy = Lossy::none();
        let mut roles: Vec<old::credentials::CredentialsRole> = self
            .roles
            .iter()
            .map(|role| old::credentials::CredentialsRole {
                role: Downgrade::<old::types::Role>::downgrade(role.role).value,
                business_details: role.business_details.clone(),
                party_id: role.party_id.clone(),
                country_code: role.country_code.clone(),
                extensions: role.extensions.clone(),
            })
            .collect();

        if let Some(hub) = self.hub_party() {
            match self.roles.first() {
                Some(first) => {
                    lossy.record(
                        "/hub_party_id",
                        format!(
                            "re-expressed as a 2.2.1 HUB role for {hub}; its business_details were \
                             copied from {} because OCPI 2.3.0 does not carry them for the hub \
                             party itself",
                            first.party()
                        ),
                    );
                    roles.push(old::credentials::CredentialsRole {
                        role: old::types::Role::Hub,
                        business_details: first.business_details.clone(),
                        party_id: hub.party_id,
                        country_code: hub.country_code,
                        extensions: crate::types::Extensions::new(),
                    });
                }
                None => lossy.record(
                    "/hub_party_id",
                    format!("cannot be expressed in OCPI 2.2.1: no role to model {hub} on"),
                ),
            }
        }

        Converted::new(
            old::credentials::Credentials {
                token: self.token,
                url: self.url,
                roles,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

// -------------------------------------------------------------------------------------------
// Hub Client Info
// -------------------------------------------------------------------------------------------

impl Upgrade<new::hub_client_info::ClientInfo> for old::hub_client_info::ClientInfo {
    fn upgrade(self) -> Converted<new::hub_client_info::ClientInfo> {
        let converted = Upgrade::<new::types::Role>::upgrade(self.role);
        let mut lossy = Lossy::none();
        lossy.absorb("/role", converted.lossy);
        Converted::new(
            new::hub_client_info::ClientInfo {
                party_id: self.party_id,
                country_code: self.country_code,
                role: converted.value,
                status: self.status,
                last_updated: self.last_updated,
                extensions: self.extensions,
            },
            lossy,
        )
    }
}

impl Downgrade<old::hub_client_info::ClientInfo> for new::hub_client_info::ClientInfo {
    fn downgrade(self) -> Converted<old::hub_client_info::ClientInfo> {
        Converted::lossless(old::hub_client_info::ClientInfo {
            party_id: self.party_id,
            country_code: self.country_code,
            role: Downgrade::<old::types::Role>::downgrade(self.role).value,
            status: self.status,
            last_updated: self.last_updated,
            extensions: self.extensions,
        })
    }
}

/// Checks that a bridged object still conforms to its own version's rules.
///
/// A hub should call this on what it is about to forward: a conversion that produced a
/// non-conformant object is a bug worth catching before it reaches a partner.
///
/// # Errors
///
/// Returns the violations of the converted object.
pub fn check_bridged<T: Validate>(converted: &Converted<T>) -> Result<(), crate::types::Violations> {
    converted.value.validate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DateTime;

    fn dt() -> DateTime {
        "2024-01-01T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn price_survives_a_round_trip_when_it_has_at_most_one_vat_line() {
        let original = old::types::Price::with_vat("5.00".parse().unwrap(), "5.50".parse().unwrap());
        let up: new::types::Price = original.clone().upgrade().expect_lossless();
        assert_eq!(up.taxes.len(), 1);
        assert_eq!(up.taxes[0].name.as_str(), "VAT");
        let back: old::types::Price = up.downgrade().expect_lossless();
        assert_eq!(back, original);
    }

    #[test]
    fn several_named_taxes_collapse_and_say_so() {
        let mut price = new::types::Price::new("5.00".parse().unwrap());
        price.taxes.push(new::types::TaxAmount::new("GST", None, "0.25".parse().unwrap()).unwrap());
        price.taxes.push(new::types::TaxAmount::new("QST", None, "0.50".parse().unwrap()).unwrap());
        let converted = price.downgrade();
        assert_eq!(converted.value.excl_vat.to_string(), "5.00");
        assert_eq!(converted.value.incl_vat.unwrap().to_string(), "5.75");
        assert_eq!(converted.lossy.len(), 2, "both tax names are reported");
        assert!(converted.lossy.as_slice()[0].reason.contains("GST"));
    }

    #[test]
    fn connector_types_added_in_2_3_0_survive_a_downgrade_as_their_wire_value() {
        let mcs = new::locations::ConnectorType::Mcs;
        let old_type: old::locations::ConnectorType = mcs.clone().downgrade().expect_lossless();
        assert!(!old_type.is_known(), "2.2.1 does not define MCS");
        assert_eq!(old_type.as_str(), "MCS", "but the wire value is unchanged");
        let round_trip: new::locations::ConnectorType = old_type.upgrade().expect_lossless();
        assert_eq!(round_trip, mcs);
    }

    #[test]
    fn a_2_2_1_hub_role_becomes_hub_party_id_and_back() {
        let business = old::locations::BusinessDetails::builder().name("Example Hub").build();
        let credentials = old::credentials::Credentials::builder()
            .token("token")
            .url(crate::types::Url::new("https://hub.example.com/versions").unwrap())
            .roles(vec![
                old::credentials::CredentialsRole::builder()
                    .role(old::types::Role::Cpo)
                    .business_details(business.clone())
                    .party_id("TNM")
                    .country_code("NL")
                    .build(),
                old::credentials::CredentialsRole::builder()
                    .role(old::types::Role::Hub)
                    .business_details(business)
                    .party_id("HUB")
                    .country_code("NL")
                    .build(),
            ])
            .build();

        let up: new::credentials::Credentials = credentials.upgrade().expect_lossless();
        assert_eq!(up.hub_party_id.as_ref().unwrap().as_str(), "NLHUB");
        assert_eq!(up.roles.len(), 1, "the HUB role became hub_party_id");
        assert!(up.is_routing_platform());

        let back = up.downgrade();
        assert!(back.value.is_hub());
        assert_eq!(back.value.roles.len(), 2);
        // The business details had to be borrowed from another role, and that is reported.
        assert_eq!(back.lossy.len(), 1);
        assert_eq!(back.lossy.as_slice()[0].pointer, "/hub_party_id");
    }

    #[test]
    fn a_tax_inclusive_tariff_cannot_be_downgraded_faithfully() {
        let tariff = new::tariffs::Tariff::builder()
            .country_code("CA")
            .party_id("ABC")
            .id("1")
            .currency("CAD")
            .elements(vec![
                new::tariffs::TariffElement::builder()
                    .price_components(vec![new::tariffs::PriceComponent::new(
                        new::tariffs::TariffDimensionType::Time,
                        "2.10".parse().unwrap(),
                    )])
                    .build(),
            ])
            .tax_included(new::tariffs::TaxIncluded::Yes)
            .last_updated(dt())
            .build();
        let converted = tariff.downgrade();
        assert_eq!(converted.lossy.as_slice()[0].pointer, "/tax_included");
        assert!(converted.lossy.as_slice()[0].reason.contains("excluding VAT"));
    }

    #[test]
    fn a_2_2_1_tariff_upgrades_to_tax_excluded() {
        let tariff = old::tariffs::Tariff::builder()
            .country_code("DE")
            .party_id("ALL")
            .id("12")
            .currency("EUR")
            .elements(vec![
                old::tariffs::TariffElement::builder()
                    .price_components(vec![old::tariffs::PriceComponent::new(
                        old::tariffs::TariffDimensionType::Time,
                        "2.00".parse().unwrap(),
                    )])
                    .build(),
            ])
            .last_updated(dt())
            .build();
        let up: new::tariffs::Tariff = tariff.upgrade().expect_lossless();
        assert_eq!(up.tax_included, new::tariffs::TaxIncluded::No);
        assert!(up.validate().is_ok());
    }

    #[test]
    fn location_fields_added_in_2_3_0_are_reported_when_dropped() {
        let mut location = new::locations::Location::builder()
            .country_code("NL")
            .party_id("TNM")
            .id("LOC1")
            .publish(true)
            .address("Street 1")
            .city("Amsterdam")
            .country("NLD")
            .coordinates(new::locations::GeoLocation::new("52.010000", "4.350000").unwrap())
            .time_zone("Europe/Amsterdam")
            .help_phone(crate::types::CiString::<25>::new("+31201234567").unwrap())
            .last_updated(dt())
            .build();
        location.parking_places.push(
            new::locations::Parking::builder()
                .id("P1")
                .vehicle_types(vec![new::locations::VehicleType::PersonalVehicle])
                .restricted_to_type(false)
                .reservation_required(false)
                .build(),
        );
        let converted = location.downgrade();
        let pointers: Vec<&str> = converted.lossy.as_slice().iter().map(|l| l.pointer.as_str()).collect();
        assert!(pointers.contains(&"/parking_places"), "{pointers:?}");
        assert!(pointers.contains(&"/help_phone"), "{pointers:?}");
        assert!(converted.value.validate().is_ok(), "the downgraded object still conforms");
    }
}
