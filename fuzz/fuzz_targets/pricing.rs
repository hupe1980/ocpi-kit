//! Price an arbitrary CDR against an arbitrary Tariff.
//!
//! The engine is the crate's densest arithmetic, it runs on documents a *counterparty* wrote, and
//! `rust_decimal` panics on overflow. `PricingError::OutOfRange` is supposed to make that
//! unreachable; this is what tests that claim against inputs nobody wrote by hand.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ocpi_kit::tariffs::{PricedSession, PricingEngine, PricingPolicy, TimeZone, lint, verify_cdr};
use ocpi_kit::v2_3_0::cdrs::Cdr;
use ocpi_kit::v2_3_0::tariffs::Tariff;

fuzz_target!(|data: &[u8]| {
    let middle = data.len() / 2;
    let (left, right) = data.split_at(middle);

    let Ok(cdr) = serde_json::from_slice::<Cdr>(left) else { return };
    let Ok(tariff) = serde_json::from_slice::<Tariff>(right) else { return };

    let _ = lint(&tariff);
    let session = PricedSession::from_cdr(&cdr, TimeZone::utc());
    for policy in [PricingPolicy::default(), PricingPolicy::default().without_step_size()] {
        if let Ok(breakdown) = PricingEngine::with_policy(policy).price(&session, &[tariff.clone()]) {
            // The invariant the breakdown is published on: the tax lines account for exactly the
            // difference between the two totals.
            let taxes: ocpi_kit::types::Number = breakdown.taxes.iter().map(|t| t.amount).sum();
            assert_eq!(
                taxes,
                breakdown.total_incl_vat - breakdown.total_excl_vat,
                "tax lines do not account for the difference between the totals",
            );
            let _ = verify_cdr(&cdr, &breakdown);
            let _ = serde_json::to_string(&breakdown);
        }
    }
});
