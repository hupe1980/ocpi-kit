//! `errata`: re-derive every recorded specification defect from the specification itself.
//!
//! [`concepts/ERRATA.md`] and the published [spec errata page] list the places where OCPI
//! contradicts itself, and the crate is *shaped* around several of them: a type that exists only
//! because 2.1.1 renamed a field, a URL builder that exists only because one module identifier is
//! addressed through two endpoint variables, a tariff reading that exists only because a sentence
//! and its own worked example disagree.
//!
//! Those claims are public, and other people quote them. An erratum upstream **fixes** therefore
//! turns into a false claim in this repository — and a documented workaround for a problem that no
//! longer exists is worse than no workaround at all, because it is invisible.
//!
//! So each entry below is expressed as a predicate over the vendored specification: text that must
//! still be there, or must still be absent, for the erratum to be real. A fixed erratum makes this
//! check **fail**, with the entry that needs re-reading named. That is the same shape as
//! `tests/fixtures.rs`, which asserts that the specification's own broken examples still fail to
//! decode.
//!
//! ```text
//! cargo run -p xtask -- errata            # report
//! cargo run -p xtask -- errata --check    # …and fail when one stops reproducing (CI)
//! ```
//!
//! [`concepts/ERRATA.md`]: https://github.com/hupe1980/ocpi-kit
//! [spec errata page]: https://hupe1980.github.io/ocpi-kit/docs/reference/errata/

use std::path::Path;

use regex::Regex;

use crate::Failure;

/// One recorded specification defect, and how to tell whether it is still there.
struct Erratum {
    /// Stable identifier, cited from the documentation.
    id: &'static str,
    /// What is wrong, in one line.
    title: &'static str,
    /// Directory under `specs/src/`.
    release: &'static str,
    /// File within that release.
    file: &'static str,
    /// Regexes that must all match for the erratum to still be real.
    present: &'static [&'static str],
    /// Regexes that must all *fail* to match for the erratum to still be real.
    absent: &'static [&'static str],
    /// What it means for this crate — printed when the erratum stops reproducing.
    consequence: &'static str,
}

/// Every erratum this crate is shaped around.
const ERRATA: &[Erratum] = &[
    Erratum {
        id: "E01",
        title: "the Hub Client Info Sender endpoint is written as {locations_endpoint_url}",
        release: "ocpi-2.3.0",
        file: "mod_hub_client_info.asciidoc",
        present: &[r"locations_endpoint_url"],
        absent: &[],
        consequence: "the URL builders read it as {hubclientinfo_endpoint_url}; if the copy-paste \
                      is fixed, delete the note",
    },
    Erratum {
        id: "E02",
        title: "the Hub Client Info receiver example uses version 2.0 and the path `clientinfo`",
        release: "ocpi-2.3.0",
        file: "mod_hub_client_info.asciidoc",
        present: &[r"/2\.0/clientinfo"],
        absent: &[],
        consequence: "the builders use the discovered endpoint URL and treat the example as \
                      non-normative",
    },
    Erratum {
        id: "E03",
        title: "the Bookings module identifier is `Booking`: singular, and the only mixed-case one",
        release: "ocpi-2.3.0-bookings",
        file: "mod_bookings.asciidoc",
        present: &[r"Module Identifier: `Booking`"],
        absent: &[],
        consequence: "`ModuleId::Booking` matches `bookings` case-insensitively as an interop \
                      accommodation; if the identifier is regularised, drop the accommodation",
    },
    Erratum {
        id: "E04",
        title: "the payments branch's own ModuleID table does not list `payments`",
        release: "ocpi-2.3.0-payments",
        file: "version_information_endpoint.asciidoc",
        present: &[r"ModuleID _OpenEnum_"],
        absent: &[r"\|<<mod_payments"],
        consequence: "`ModuleId::Payments` exists and documents the omission",
    },
    Erratum {
        id: "E05",
        title: "the bookings branch's own ModuleID table does not list `Booking`",
        release: "ocpi-2.3.0-bookings",
        file: "version_information_endpoint.asciidoc",
        present: &[r"ModuleID _OpenEnum_"],
        absent: &[r"\|<<mod_bookings"],
        consequence: "`ModuleId::Booking` exists and documents the omission",
    },
    Erratum {
        id: "E06",
        title: "core's ModuleID table does not list `invoicereconciliation`, though core defines \
                the module",
        release: "ocpi-2.3.0",
        file: "version_information_endpoint.asciidoc",
        present: &[r"ModuleID _OpenEnum_"],
        absent: &[r"\|<<mod_invoice_reconciliation"],
        consequence: "`ModuleId::InvoiceReconciliation` exists and documents the omission",
    },
    Erratum {
        id: "E07",
        title: "Payments declares one module identifier and addresses its two interfaces through \
                two endpoint URL variables",
        release: "ocpi-2.3.0-payments",
        file: "mod_payments.asciidoc",
        present: &[
            r"Module Identifier: `payments`",
            r"payments_terminals_endpoint_url",
            r"payments_financial_advice_confirmation_endpoint_url",
        ],
        absent: &[],
        consequence: "`SenderEndpoint::payments_terminals` derives both from the one discovered \
                      endpoint, and tolerates a peer that advertised a sub-path directly",
    },
    Erratum {
        id: "E08",
        title: "the Payments terminal activation body is a Terminal whose `terminal_id` is optional",
        release: "ocpi-2.3.0-payments",
        file: "mod_payments.asciidoc",
        present: &[r"terminal_id is optional"],
        absent: &[],
        consequence: "the body is typed `Patch<Terminal>` — an object with fields left out — and \
                      is deliberately not treated as a merge patch",
    },
    Erratum {
        id: "E09",
        title: "ChargingProfileResult and ClearProfileResult are the same object shape",
        release: "ocpi-2.3.0",
        file: "mod_charging_profiles.asciidoc",
        present: &[r"==== _ChargingProfileResult_ Object", r"==== _ClearProfileResult_ Object"],
        absent: &[],
        consequence: "the Sender's freedom over the `response_url` is what distinguishes them: \
                      `OcpiRouter` mounts one path per result kind",
    },
    Erratum {
        id: "E10",
        title: "the Tariffs chapter states that there are no rounding rules at all",
        release: "ocpi-2.3.0",
        file: "mod_tariffs.asciidoc",
        present: &[r"no parameters related to price rounding"],
        absent: &[],
        consequence: "rounding is a `PricingPolicy` setting with a recorded default, not an \
                      assumption in the code",
    },
    Erratum {
        id: "E11",
        title: "the specification's own free-of-charge example writes a `step_size` of 0, which \
                the text never defines",
        release: "ocpi-2.3.0",
        file: "examples/tariff_5_free_of_charge.json",
        present: &["\"step_size\": 0"],
        absent: &[],
        consequence: "0 is read as no quantisation at all, while 1 is applied; `ocpi lint` reports \
                      a zero step on a dimension that has a unit",
    },
    Erratum {
        id: "E12",
        title: "a Charging Period carries totals rather than a curve, and the obligation to split \
                one at a price change is put on the CPO",
        release: "ocpi-2.3.0",
        file: "mod_cdrs.asciidoc",
        present: &[r"SHALL at least start \(and add\) a <<mod_cdrs_chargingperiod_class,ChargingPeriod>>"],
        absent: &[],
        consequence: "a period that outlasts its Price Component is priced at the rate that \
                      applied when it began, with a `PeriodSpansPriceChange` note",
    },
    Erratum {
        id: "E13",
        title: "the TIME/PARKING_TIME `step_size` sentence and its own worked example justify the \
                same answer differently",
        release: "ocpi-2.3.0",
        file: "mod_cdrs.asciidoc",
        present: &[
            r"only taken into account for the total parking duration",
            r"not rounded up, as it is followed by another time based period",
        ],
        absent: &[],
        consequence: "the sentence governs: PARKING_TIME absorbs the rounding whenever the session \
                      has any, and `ocpi lint` reports a TIME `step_size` that can never apply",
    },
    Erratum {
        id: "E14",
        title: "`SignedData.url` is a string(512) whose cross-reference points at the CiString \
                anchor",
        release: "ocpi-2.3.0",
        file: "mod_cdrs.asciidoc",
        present: &[r"\|url\s*\|\s*<<types\.asciidoc#types_cistring_type,string>>\(512\)"],
        absent: &[],
        consequence: "modelled as the string(512) the text says, case-sensitive, following the \
                      text over the cross-reference — a Url would reject a conformant 300-\
                      character link",
    },
    Erratum {
        id: "E15",
        title: "the APP_USER whitelist rule is a recommendation the specification's own example \
                breaks",
        release: "ocpi-2.3.0",
        file: "mod_tokens.asciidoc",
        present: &[r"RECOMMENDED to push Tokens with type"],
        absent: &[],
        consequence: "`Validate` does not report it; `Token::follows_whitelist_recommendation` \
                      answers the question for a caller who wants to ask it",
    },
    Erratum {
        id: "E16",
        title: "OCPI 2.1.1 calls the EnvironmentalImpact category `source`, renamed in 2.2 with \
                nothing marking it",
        release: "ocpi-2.1.1",
        file: "mod_locations.md",
        present: &[r"\|\s*source\s*\|\s*\[EnvironmentalImpactCategory\]"],
        absent: &[],
        consequence: "2.1.1 gets its own EnvironmentalImpact type; reusing the later one would \
                      drop a 2.1.1 peer's value into `extensions`",
    },
    Erratum {
        id: "E17",
        title: "OCPI 2.1.1 makes `twentyfourseven` a choice, which 2.2 made required without \
                saying so",
        release: "ocpi-2.1.1",
        file: "mod_locations.md",
        present: &[r"Choice: one of two"],
        absent: &[],
        consequence: "2.1.1 gets its own `Hours` with `twentyfourseven: Option<bool>`",
    },
    Erratum {
        id: "E18",
        title: "`Tariff.preauthorize_amount` belongs to the payments release branch, not to core",
        release: "ocpi-2.3.0-payments",
        file: "mod_tariffs.asciidoc",
        present: &[r"preauthorize_amount"],
        absent: &[],
        consequence: "the field is behind the `payments` feature and declared branch-only in the \
                      census; it was part of core until upstream moved the module out in July 2026",
    },
    Erratum {
        id: "E19",
        title: "the specification's own € 3/hour + € 5/hour parking example carries a TIME \
                `step_size` its own rule makes inert",
        release: "ocpi-2.3.0",
        file: "examples/tariff_13_simple_3hour_5parking.json",
        present: &["\"type\": \"TIME\"", "\"type\": \"PARKING_TIME\"", "\"step_size\": 60"],
        absent: &[],
        consequence: "`ocpi lint` reports it as `unused_time_step_size`; it is the shape on which \
                      two readings of the step_size sentence disagree about a real session",
    },
];

/// Re-derives every erratum from the vendored specification.
///
/// # Errors
///
/// Fails when the specification is not vendored, when a file an erratum names cannot be read, or
/// when a pattern does not compile.
pub fn report(root: &Path, check: bool) -> Result<bool, Failure> {
    let specs = root.join("specs/src");
    if !specs.is_dir() {
        return Err(format!(
            "{} does not exist; run `cargo run -p xtask -- spec-sync --fetch` first",
            specs.display()
        )
        .into());
    }

    let mut standing = 0usize;
    let mut stale = Vec::new();
    for erratum in ERRATA {
        let path = specs.join(erratum.release).join(erratum.file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            stale.push(format!(
                "{}: {} — {} is not in the vendored {} any more",
                erratum.id, erratum.title, erratum.file, erratum.release
            ));
            continue;
        };

        let mut reasons = Vec::new();
        for pattern in erratum.present {
            let re = Regex::new(pattern)?;
            if !re.is_match(&text) {
                reasons.push(format!("expected to find {pattern:?}"));
            }
        }
        for pattern in erratum.absent {
            let re = Regex::new(pattern)?;
            if re.is_match(&text) {
                reasons.push(format!("expected {pattern:?} to be absent, and it is there"));
            }
        }

        if reasons.is_empty() {
            standing += 1;
            println!("[ok] {} {} — still in {}/{}", erratum.id, erratum.title, erratum.release, erratum.file);
        } else {
            stale.push(format!(
                "{} {}\n     in {}/{}\n     {}\n     what this crate does about it: {}",
                erratum.id,
                erratum.title,
                erratum.release,
                erratum.file,
                reasons.join("; "),
                erratum.consequence,
            ));
        }
    }

    if stale.is_empty() {
        println!("\n{standing} erratum/errata re-derived from the specification; every one is still real");
        return Ok(true);
    }

    println!("\n{} erratum/errata no longer reproduce:\n", stale.len());
    for entry in &stale {
        println!("  {entry}\n");
    }
    println!(
        "An erratum that stops reproducing is good news and a job: re-read the chapter, then \
         either delete the workaround and its documentation, or fix this check if the wording \
         merely moved."
    );
    Ok(!check)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles_and_every_entry_is_complete() {
        let mut ids: Vec<&str> = Vec::new();
        for erratum in ERRATA {
            assert!(!erratum.title.is_empty(), "{} has no title", erratum.id);
            assert!(!erratum.consequence.is_empty(), "{} says nothing about the crate", erratum.id);
            assert!(
                !(erratum.present.is_empty() && erratum.absent.is_empty()),
                "{} asserts nothing, so it can never stop reproducing",
                erratum.id,
            );
            for pattern in erratum.present.iter().chain(erratum.absent) {
                Regex::new(pattern).unwrap_or_else(|e| panic!("{}: {pattern:?}: {e}", erratum.id));
            }
            assert!(!ids.contains(&erratum.id), "{} is used twice", erratum.id);
            ids.push(erratum.id);
        }
    }
}
