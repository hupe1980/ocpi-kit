//! Decode the OCPI response envelope from arbitrary bytes.
//!
//! Every response a client reads goes through this, before anything knows whether the peer is
//! sane. It is also where the four-digit status code and the timestamp are parsed.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ocpi_kit::transport::OcpiResponse;

fuzz_target!(|data: &[u8]| {
    if let Ok(envelope) = serde_json::from_slice::<OcpiResponse<serde_json::Value>>(data) {
        let _ = envelope.is_success();
        let _ = envelope.status_code.class();
        let _ = serde_json::to_string(&envelope);
        let _ = envelope.into_result();
    }
    // The typed form too: a peer's `data` that is not the object the endpoint carries.
    let _ = serde_json::from_slice::<OcpiResponse<Vec<ocpi_kit::v2_3_0::locations::Location>>>(data);
});
