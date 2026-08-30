//! A roaming hub: routing, broadcast push, open routing, GET All, and version bridging.
//!
//! # The four routing arrangements
//!
//! A hub can tell which one it is looking at from the headers and the method alone, which is what
//! [`Forwardable::scenario`] does:
//!
//! | The `OCPI-to-*` headers say | Method | It is |
//! |---|---|---|
//! | another party | any | [`Direct`](crate::transport::RoutingScenario::Direct) — relay it |
//! | the hub itself | `GET` on a Sender interface | [`GetAllViaHub`](crate::transport::RoutingScenario::GetAllViaHub) — merge every party's objects |
//! | the hub itself | a write on a Receiver interface | [`BroadcastPush`](crate::transport::RoutingScenario::BroadcastPush) — fan out to the opposite roles |
//! | nothing | any | [`OpenRoutingRequest`](crate::transport::RoutingScenario::OpenRoutingRequest) — decide from the content |
//! | the hub itself | anything else | refused: [`OcpiError::NotRoutable`](crate::transport::OcpiError::NotRoutable), a `2001` |
//!
//! Addressing the hub is the one ambiguous case, and two of its four combinations are not
//! scenarios at all: a `GET` on a Receiver interface is not a Broadcast Push, because *"GET SHALL
//! NOT be used in combination with Broadcast Push"*, and a write on a Sender interface is neither
//! a push to the connected parties nor a read to merge. Guessing the nearest scenario would mean
//! a hub quietly broadcasting a read, so [`Forwardable::scenario`] refuses, with the advice the
//! specification itself gives: omit the `OCPI-to-` headers and make it an Open Routing Request.
//!
//! # The rules the hub must not break
//!
//! * **A new `X-Request-ID`, the same `X-Correlation-ID`.** [`RequestIds::forwarded`](crate::transport::RequestIds::forwarded).
//! * **`last_updated` is never touched.** *"When OCPI Objects are sent via Hubs, the
//!   `last_updated` fields SHALL NOT be updated by the Hub."* Nothing in this module writes it.
//! * **`GET` is never broadcast.** *"GET SHALL NOT be used in combination with Broadcast Push."*
//! * **Configuration modules are never routed.** `credentials`, `versions` and `hubclientinfo`
//!   are platform-to-hub conversations.
//! * **Vendor data survives.** Because every object keeps its unknown fields in
//!   [`Extensions`](crate::types::Extensions) and every `OpenEnum` keeps values it does not know,
//!   a hub built on this crate forwards an extension it has never seen without damaging it. That
//!   is what OCPI 2.3.0's extensibility chapter asks for, and it is the single most common way a
//!   hub loses data.
//!
//! # Version bridging
//!
//! A 2.2.1 CPO and a 2.3.0 eMSP talk through the hub without either of them knowing: [`Forwarder`]
//! translates the request body into the version the receiving platform speaks, translates the
//! response back, and appends what the crossing cost to the `status_message` so the requesting
//! party can see it. That is [`convert`](crate::convert) doing the work and
//! [`Lossy`](crate::convert::Lossy) doing the reporting; no object is ever handed to a party in a
//! version it did not ask for, and nothing is dropped silently.
//!
//! Only the 2.2.1 ↔ 2.3.0 crossing has conversions today. A message between two versions this
//! build cannot translate is **refused** by default rather than relayed — see [`Unbridgeable`] for
//! the reasoning and for how to relay it anyway. [`bridge`] exposes the same classification for a
//! hub that wants to translate an object itself.
//!
//! Spec: 2.3.0 §transport_and_format_message_routing, §status_codes_4xxx_hub_errors

mod forwarder;
mod routing_table;

pub use forwarder::{
    AggregatePolicy, BodyOwnerRouter, Forwardable, Forwarder, OpenRouter, Relayed, Unbridgeable, aggregate,
};
pub use routing_table::{ConnectedPlatform, RoutingTable};

use crate::VersionNumber;

/// What a hub must do to a message crossing between two OCPI versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Bridge {
    /// Both sides speak the same version; forward the bytes unchanged.
    ///
    /// This is the fast path, and the one that cannot lose anything.
    Passthrough,
    /// The receiver speaks a newer version; upgrade.
    Upgrade,
    /// The receiver speaks an older version; downgrade, and report what did not fit.
    Downgrade,
    /// This build has no conversions between the two versions, so nothing can be translated.
    ///
    /// Today that is any crossing involving OCPI 2.1.1 — which has no owner fields, no routing
    /// and no `Price`, so carrying an object across it is a decision about the deployment rather
    /// than a translation — or a version this crate does not model at all.
    ///
    /// Relaying the bytes unchanged is still an option, and for two parties that understand each
    /// other by some arrangement outside this crate it is the right one; the decision belongs to
    /// the operator, so [`Forwarder`] takes it as [`Unbridgeable`] rather than assuming.
    Unsupported,
}

/// What the hub must do to carry a message from `sender` to `receiver`.
///
/// ```
/// use ocpi_kit::hub::{bridge, Bridge};
/// use ocpi_kit::VersionNumber;
///
/// assert_eq!(bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_3_0), Bridge::Passthrough);
/// assert_eq!(bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_3_0), Bridge::Upgrade);
/// assert_eq!(bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_2_1), Bridge::Downgrade);
/// assert_eq!(bridge(&VersionNumber::V2_3_0, &"3.0".into()), Bridge::Unsupported);
/// ```
#[must_use]
pub fn bridge(sender: &VersionNumber, receiver: &VersionNumber) -> Bridge {
    if sender == receiver {
        return Bridge::Passthrough;
    }
    // Whether the *models* exist is not the question; whether the *conversions* do is. A build
    // with `v2_1_1` on models 2.1.1 perfectly well and still cannot carry an object out of it.
    if !crate::convert::wire::bridgeable(sender, receiver) {
        return Bridge::Unsupported;
    }
    if receiver.release_rank() > sender.release_rank() { Bridge::Upgrade } else { Bridge::Downgrade }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_version_is_a_passthrough() {
        assert_eq!(bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_2_1), Bridge::Passthrough);
        // Even for a version this build does not model: two 3.0 parties can still talk.
        let future: VersionNumber = "3.0".into();
        assert_eq!(bridge(&future, &future), Bridge::Passthrough);
    }

    #[test]
    fn direction_follows_the_release_order() {
        assert_eq!(bridge(&VersionNumber::V2_2_1, &VersionNumber::V2_3_0), Bridge::Upgrade);
        assert_eq!(bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_2_1), Bridge::Downgrade);
    }

    #[test]
    fn a_crossing_with_no_conversions_is_surfaced_rather_than_guessed_at() {
        assert_eq!(bridge(&"3.0".into(), &VersionNumber::V2_3_0), Bridge::Unsupported);
        assert_eq!(bridge(&VersionNumber::V2_0, &VersionNumber::V2_3_0), Bridge::Unsupported);
        // 2.1.1 is modelled by this crate and still has no conversions: claiming an `Upgrade`
        // here would promise a translation that does not exist.
        assert_eq!(bridge(&VersionNumber::V2_1_1, &VersionNumber::V2_3_0), Bridge::Unsupported);
        assert_eq!(bridge(&VersionNumber::V2_3_0, &VersionNumber::V2_1_1), Bridge::Unsupported);
    }
}
