//! Protocol versions, module identifiers and interface roles.
//!
//! These three enums are version-neutral on purpose: version *discovery* has to be able to name
//! a version or a module that this crate does not implement — that is what the `/versions`
//! endpoint is for — so all three are `OpenEnum`-shaped and keep values they do not know.

use crate::ocpi_enum;
use crate::ocpi_open_enum;

ocpi_open_enum! {
    /// A version of the OCPI protocol, as it appears in `/versions` and in version details.
    ///
    /// Listed as an `OpenEnum` so that discovery against a peer that speaks a version this crate
    /// has never heard of — a future 3.0, or a version the peer invented — lists it and moves on
    /// instead of failing. [`VersionNumber::is_supported`] says which ones this build can
    /// actually talk.
    ///
    /// Spec: 2.3.0 §version_information_endpoint_versionnumber_enum
    pub enum VersionNumber {
        /// OCPI version 2.0. Not modelled by this crate; recognised so discovery can skip it.
        V2_0 = "2.0",
        /// OCPI version 2.1. **Deprecated by the spec**: *"do not use, use 2.1.1 instead"*.
        V2_1 = "2.1",
        /// OCPI version 2.1.1.
        V2_1_1 = "2.1.1",
        /// OCPI version 2.2. **Deprecated by the spec**: *"do not use, use 2.2.1 instead"*.
        V2_2 = "2.2",
        /// OCPI version 2.2.1. Still the most widely deployed version.
        V2_2_1 = "2.2.1",
        /// OCPI version 2.3.0. The canonical model of this crate.
        V2_3_0 = "2.3.0",
    }
}

impl VersionNumber {
    /// Whether this build of `ocpi-kit` has a wire model for this version.
    ///
    /// Depends on the enabled cargo features.
    #[must_use]
    pub fn is_supported(&self) -> bool {
        match self {
            Self::V2_3_0 => cfg!(feature = "v2_3_0"),
            Self::V2_2_1 => cfg!(feature = "v2_2_1"),
            Self::V2_1_1 => cfg!(feature = "v2_1_1"),
            Self::V2_0 | Self::V2_1 | Self::V2_2 | Self::Custom(_) => false,
        }
    }

    /// Whether the specification marks this version as deprecated.
    ///
    /// Spec: 2.3.0 §version_information_endpoint_versionnumber_enum
    #[must_use]
    pub const fn is_deprecated(&self) -> bool {
        matches!(self, Self::V2_1 | Self::V2_2)
    }

    /// Every version this build can talk, newest first.
    ///
    /// This is the preference order used when negotiating with a peer.
    #[must_use]
    #[allow(unused_mut, clippy::vec_init_then_push)]
    pub fn supported() -> Vec<Self> {
        let mut out = Vec::new();
        #[cfg(feature = "v2_3_0")]
        out.push(Self::V2_3_0);
        #[cfg(feature = "v2_2_1")]
        out.push(Self::V2_2_1);
        #[cfg(feature = "v2_1_1")]
        out.push(Self::V2_1_1);
        out
    }

    /// Where this version sits in the release order, oldest first.
    ///
    /// A version this crate does not know ranks last. This is deliberately *not* the [`Ord`]
    /// impl, which sorts by wire value so that `VersionNumber` behaves predictably as a map key
    /// — and note that sorting by wire value is wrong for versions, since `"2.10" < "2.2"`
    /// lexically. Use [`VersionNumber::cmp_by_release`] to order them.
    #[must_use]
    pub const fn release_rank(&self) -> u8 {
        match self {
            Self::V2_0 => 0,
            Self::V2_1 => 1,
            Self::V2_1_1 => 2,
            Self::V2_2 => 3,
            Self::V2_2_1 => 4,
            Self::V2_3_0 => 5,
            Self::Custom(_) => u8::MAX,
        }
    }

    /// Orders two versions by release, oldest first; unknown versions sort last, by their text.
    ///
    /// ```
    /// use ocpi_kit::VersionNumber;
    /// let mut vs = vec![VersionNumber::V2_3_0, VersionNumber::V2_1_1, VersionNumber::V2_2_1];
    /// vs.sort_by(VersionNumber::cmp_by_release);
    /// assert_eq!(vs.first(), Some(&VersionNumber::V2_1_1));
    /// ```
    #[must_use]
    pub fn cmp_by_release(&self, other: &Self) -> core::cmp::Ordering {
        self.release_rank().cmp(&other.release_rank()).then_with(|| self.as_str().cmp(other.as_str()))
    }

    /// Whether the version uses the `OCPI-to-*`/`OCPI-from-*` message routing headers.
    ///
    /// Routing headers were introduced in OCPI 2.2; 2.1.1 and older have no such thing.
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing
    #[must_use]
    pub fn has_routing_headers(&self) -> bool {
        matches!(self.release_rank(), 3..=5)
    }

    /// Whether the version splits `Credentials` into a list of `roles`.
    ///
    /// OCPI 2.1.1 puts `party_id`, `country_code` and `business_details` directly on the
    /// credentials object; 2.2 and later moved them into `CredentialsRole` entries.
    #[must_use]
    pub fn has_credentials_roles(&self) -> bool {
        matches!(self.release_rank(), 3..=5)
    }
}

ocpi_enum! {
    /// Which side of a module's data flow an endpoint implements.
    ///
    /// > *SENDER: Interface implemented by the owner of data, so the Receiver can Pull
    /// > information from the data Sender/owner.*
    /// >
    /// > *RECEIVER: Interface implemented by the receiver of data, so the Sender/owner can Push
    /// > information to the Receiver.*
    ///
    /// Spec: 2.3.0 §version_information_endpoint_interface_role_enum
    pub enum InterfaceRole {
        /// The data owner's interface, which the other party pulls from.
        Sender = "SENDER",
        /// The interface the data owner pushes to.
        Receiver = "RECEIVER",
    }
}

impl InterfaceRole {
    /// The other side of the same module.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Sender => Self::Receiver,
            Self::Receiver => Self::Sender,
        }
    }
}

ocpi_open_enum! {
    /// The identifier of an OCPI module, as used in `Endpoint.identifier`.
    ///
    /// > *Parties are allowed to create custom modules or customized versions of the existing
    /// > modules. To do so, the ModuleID enum can be extended with additional custom moduleIDs.
    /// > … It is advised to use a prefix (e.g. country-code + party-id) for any custom moduleID.*
    ///
    /// So `ModuleId::Custom("nltnm-tokens")` is a legitimate value, not an error.
    ///
    /// Spec: 2.3.0 §version_information_endpoint_moduleid_enum
    pub enum ModuleId {
        /// Charge Detail Records. Sender: CPO.
        Cdrs = "cdrs",
        /// Smart charging profiles.
        ChargingProfiles = "chargingprofiles",
        /// Remote commands: start, stop, reserve, unlock.
        Commands = "commands",
        /// Credentials and registration. Required for all implementations.
        Credentials = "credentials",
        /// Hub client info: which parties a hub has connected.
        HubClientInfo = "hubclientinfo",
        /// Charging locations, EVSEs and connectors. Sender: CPO.
        Locations = "locations",
        /// Payment terminals and financial advice confirmations. Sender: PTP.
        ///
        /// Added to the protocol in OCPI 2.3.0.
        ///
        /// **Spec erratum.** The Payments module chapter gives *"Module Identifier: `payments`"*,
        /// but the module is missing from the `ModuleID` table in
        /// §version_information_endpoint_moduleid_enum of the same release. The chapter is
        /// normative for its own identifier, so `payments` is treated as a known module here.
        Payments = "payments",
        /// Charging sessions. Sender: CPO.
        Sessions = "sessions",
        /// Tariffs. Sender: CPO.
        Tariffs = "tariffs",
        /// Driver tokens and real-time authorization. Sender: eMSP.
        Tokens = "tokens",
        /// Versions and version details. Every implementation has this, but it is not listed as
        /// an endpoint inside version details.
        Versions = "versions",
        /// Bookings, from the OCPI 2.3.0 `bookings` release branch.
        ///
        /// **Spec quirk.** The identifier really is `Booking`: singular, and the only module ID
        /// in OCPI that is not lower-case. Every other module uses a lower-case plural. It is
        /// also absent from that branch's `ModuleID` table. Peers that guessed `bookings`
        /// exist; see [`ModuleId::matches`].
        Booking = "Booking",
        /// Invoice reconciliation, from the OCPI 2.3.0 `payments` release branch.
        InvoiceReconciliation = "invoicereconciliation",
    }
}

impl ModuleId {
    /// Whether this module carries the message routing headers.
    ///
    /// > *Only requests/responses from Function Modules … SHALL be routed, so need the routing
    /// > headers. The requests/responses to/from Configuration Modules: Credentials, Versions and
    /// > Hub Client Info are not to be routed … Thus routing headers SHALL NOT be used with these
    /// > modules.*
    ///
    /// A custom module is assumed to be functional, since that is what a party would define one
    /// for.
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing
    #[must_use]
    pub const fn is_functional(&self) -> bool {
        !self.is_configuration()
    }

    /// Whether this is one of the three configuration modules, which are never routed.
    ///
    /// Spec: 2.3.0 §transport_and_format_message_routing
    #[must_use]
    pub const fn is_configuration(&self) -> bool {
        matches!(self, Self::Credentials | Self::Versions | Self::HubClientInfo)
    }

    /// Compares module identifiers the way a tolerant peer would.
    ///
    /// Two accommodations, both narrow and both deliberate:
    ///
    /// * ASCII case is ignored. Module identifiers are lower-case everywhere except `Booking`, so
    ///   a peer that lower-cased the lot is still understood.
    /// * `Booking` and `bookings` are treated as the same module. The Bookings chapter gives
    ///   *"Module Identifier: `Booking`"* — singular, and the only mixed-case identifier in OCPI
    ///   — while implementations that assumed the lower-case plural exist. Getting this wrong
    ///   means silently not discovering the module, which is worse than accepting both.
    ///
    /// Use this when reading a peer's version details; use `==` when the exact value matters.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        if self.as_str().eq_ignore_ascii_case(other.as_str()) {
            return true;
        }
        let is_bookings = |m: &Self| {
            m.as_str().eq_ignore_ascii_case("Booking") || m.as_str().eq_ignore_ascii_case("bookings")
        };
        is_bookings(self) && is_bookings(other)
    }

    /// Whether this build of `ocpi-kit` has a wire model for this module.
    #[must_use]
    pub fn is_supported(&self) -> bool {
        // Not a `matches!`: under some feature sets the `cfg!`s collapse to the same constant
        // and under others they do not, so the arms must stay separate.
        #[allow(clippy::match_like_matches_macro)]
        match self {
            // The two release branches of OCPI 2.3.0, each behind the feature that models it.
            Self::Booking => cfg!(feature = "bookings"),
            Self::Payments => cfg!(feature = "payments"),
            // Invoice Reconciliation is part of 2.3.0 core, so it needs no feature of its own:
            // a build with the 2.3.0 model has it.
            Self::InvoiceReconciliation => cfg!(feature = "v2_3_0"),
            Self::Custom(_) => false,
            _ => cfg!(any(feature = "v2_3_0", feature = "v2_2_1", feature = "v2_1_1")),
        }
    }

    /// Whether the module exists in `version`.
    ///
    /// Spec: 2.2.1 and 2.3.0 §version_information_endpoint_moduleid_enum; 2.1.1
    /// §version_information_endpoint (which has neither `hubclientinfo` nor `chargingprofiles`).
    #[must_use]
    pub fn exists_in(&self, version: &VersionNumber) -> bool {
        match self {
            // A `Custom` module is by definition not in any published table, so it is left to
            // the peer that advertised it: assume it exists wherever it was offered.
            Self::Credentials
            | Self::Versions
            | Self::Locations
            | Self::Sessions
            | Self::Cdrs
            | Self::Tariffs
            | Self::Tokens
            | Self::Commands
            | Self::Custom(_) => true,
            Self::HubClientInfo | Self::ChargingProfiles => version.release_rank() >= 3,
            Self::Payments | Self::Booking | Self::InvoiceReconciliation => *version == VersionNumber::V2_3_0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_versions_survive_discovery() {
        let v: VersionNumber = "3.0".into();
        assert!(!v.is_known());
        assert!(!v.is_supported());
        assert_eq!(v.as_str(), "3.0");
    }

    #[test]
    fn release_order_sorts_by_release_not_by_text() {
        let mut all =
            vec![VersionNumber::V2_3_0, VersionNumber::V2_1_1, VersionNumber::V2_2_1, VersionNumber::V2_0];
        all.sort_by(VersionNumber::cmp_by_release);
        assert_eq!(
            all,
            vec![VersionNumber::V2_0, VersionNumber::V2_1_1, VersionNumber::V2_2_1, VersionNumber::V2_3_0]
        );
        assert!(VersionNumber::V2_3_0.release_rank() > VersionNumber::V2_2_1.release_rank());
        // An unknown version sorts after every known one.
        let future: VersionNumber = "3.0".into();
        assert_eq!(future.cmp_by_release(&VersionNumber::V2_3_0), core::cmp::Ordering::Greater);
    }

    #[test]
    fn configuration_modules_are_never_routed() {
        for m in [ModuleId::Credentials, ModuleId::Versions, ModuleId::HubClientInfo] {
            assert!(m.is_configuration() && !m.is_functional(), "{m} must not be routed");
        }
        for m in [ModuleId::Locations, ModuleId::Cdrs, ModuleId::Tokens, ModuleId::Payments] {
            assert!(m.is_functional(), "{m} must be routed");
        }
        assert!(ModuleId::Custom("nltnm-tokens".into()).is_functional());
    }

    #[test]
    fn booking_module_id_matches_case_insensitively() {
        let lower: ModuleId = "bookings".into();
        let spec: ModuleId = "Booking".into();
        assert_eq!(spec, ModuleId::Booking);
        assert!(!lower.is_known(), "\"bookings\" is not the identifier the spec gives");
        assert!(ModuleId::Booking.matches(&"BOOKING".into()), "case is ignored");
        assert!(ModuleId::Booking.matches(&lower), "the lower-case plural is accepted too");
        assert!(!ModuleId::Booking.matches(&ModuleId::Cdrs));
        assert_ne!(ModuleId::Booking, lower, "but they are still different values");
    }

    #[test]
    fn module_availability_follows_the_version() {
        assert!(!ModuleId::ChargingProfiles.exists_in(&VersionNumber::V2_1_1));
        assert!(ModuleId::ChargingProfiles.exists_in(&VersionNumber::V2_2_1));
        assert!(!ModuleId::Payments.exists_in(&VersionNumber::V2_2_1));
        assert!(ModuleId::Payments.exists_in(&VersionNumber::V2_3_0));
    }

    #[test]
    fn interface_roles_are_opposites() {
        assert_eq!(InterfaceRole::Sender.opposite(), InterfaceRole::Receiver);
        assert_eq!(serde_json::to_string(&InterfaceRole::Sender).unwrap(), "\"SENDER\"");
    }
}
