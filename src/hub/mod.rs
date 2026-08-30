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
//! | the hub itself | anything else | [`BroadcastPush`](crate::transport::RoutingScenario::BroadcastPush) — fan out to the opposite roles |
//! | nothing | any | [`OpenRoutingRequest`](crate::transport::RoutingScenario::OpenRoutingRequest) — decide from the content |
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
//! A 2.2.1 CPO and a 2.3.0 eMSP can talk through a hub built on [`convert`](crate::convert),
//! which reports what a translation cost rather than dropping it silently. [`bridge`] wires that
//! into a forwarding decision.
//!
//! Spec: 2.3.0 §transport_and_format_message_routing, §status_codes_4xxx_hub_errors

mod forwarder;
mod routing_table;

pub use forwarder::{
    AggregatePolicy, BodyOwnerRouter, Forwardable, Forwarder, OpenRouter, Relayed, aggregate,
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
    /// One of the versions is not one this build models, so the object cannot be translated.
    ///
    /// Forwarding the bytes unchanged is still an option and is usually the right one — a hub
    /// that refuses to relay a 3.0 message between two 3.0 parties is worse than useless — but
    /// the decision belongs to the operator, so it is surfaced rather than assumed.
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
    if !sender.is_supported() || !receiver.is_supported() {
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
    fn an_unmodelled_version_is_surfaced_rather_than_guessed_at() {
        assert_eq!(bridge(&"3.0".into(), &VersionNumber::V2_3_0), Bridge::Unsupported);
        assert_eq!(bridge(&VersionNumber::V2_0, &VersionNumber::V2_3_0), Bridge::Unsupported);
    }
}
