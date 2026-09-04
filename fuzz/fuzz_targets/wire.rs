//! Decode arbitrary bytes as each OCPI object a peer can send, then validate and re-encode.
//!
//! A peer controls every byte of these documents, and this crate forbids `unsafe`, so the leverage
//! a hostile one has is a panic — in a hub's forwarder, that is a task holding somebody else's
//! message. The property tests cover the same surface with generated *values*; this covers it with
//! generated *bytes*, which is what finds the decoder that panics on input no generator would
//! think to build.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ocpi_kit::types::Validate;
use ocpi_kit::{v2_1_1, v2_2_1, v2_3_0};

/// Decode, and if that worked, validate and re-encode. Every step must not panic.
macro_rules! exercise {
    ($ty:ty, $text:expr) => {
        if let Ok(value) = serde_json::from_str::<$ty>($text) {
            let _ = value.validate();
            let _ = serde_json::to_string(&value);
        }
    };
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };

    exercise!(v2_3_0::locations::Location, text);
    exercise!(v2_3_0::locations::Evse, text);
    exercise!(v2_3_0::cdrs::Cdr, text);
    exercise!(v2_3_0::sessions::Session, text);
    exercise!(v2_3_0::tariffs::Tariff, text);
    exercise!(v2_3_0::tokens::Token, text);
    exercise!(v2_3_0::credentials::Credentials, text);
    exercise!(v2_3_0::versions::VersionDetails, text);
    // `Command` is an enum the router builds from the URL rather than a document a peer sends;
    // its five bodies are the documents, and `StartSession` carries the interesting one.
    exercise!(v2_3_0::commands::StartSession, text);
    exercise!(v2_3_0::commands::ReserveNow, text);
    exercise!(v2_3_0::commands::CommandResult, text);
    exercise!(v2_3_0::payments::Terminal, text);
    exercise!(v2_3_0::bookings::Booking, text);
    exercise!(v2_3_0::invoice_reconciliation::InvoiceReconciliationRecord, text);

    exercise!(v2_2_1::locations::Location, text);
    exercise!(v2_2_1::cdrs::Cdr, text);
    exercise!(v2_2_1::tariffs::Tariff, text);

    exercise!(v2_1_1::locations::Location, text);
    exercise!(v2_1_1::cdrs::Cdr, text);
    exercise!(v2_1_1::sessions::Session, text);
});
