//! Apply an arbitrary RFC 7396 merge patch to an arbitrary document.
//!
//! `merge` recurses over two peer-supplied values, and `Patch::apply` re-decodes the result into
//! the object it is supposed to be. Both are reachable from a `PATCH` on any Receiver interface.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ocpi_kit::transport::{Patch, merge};
use ocpi_kit::v2_3_0::locations::Location;

fuzz_target!(|data: &[u8]| {
    // Split the input in two: the target document and the patch.
    let middle = data.len() / 2;
    let (left, right) = data.split_at(middle);

    let Ok(mut target) = serde_json::from_slice::<serde_json::Value>(left) else { return };
    let Ok(patch) = serde_json::from_slice::<serde_json::Value>(right) else { return };

    let untyped = Patch::<serde_json::Value>::from_value(patch.clone());
    let _ = untyped.last_updated();
    let _ = untyped.fields();
    merge(&mut target, &patch);

    // And the typed path, which validates the merged object rather than trusting it.
    if let Ok(location) = serde_json::from_slice::<Location>(left) {
        let typed = Patch::<Location>::from_value(patch);
        let _ = typed.apply(&location);
    }
});
