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
    /// Directory under the spec checkout ([`crate::spec_dir`]).
    release: &'static str,
    /// File within that release.
    file: &'static str,
    /// Regexes that must all match for the erratum to still be real.
    present: &'static [&'static str],
    /// Regexes that must all *fail* to match for the erratum to still be real.
    absent: &'static [&'static str],
    /// What it means for this crate — printed when the erratum stops reproducing.
    consequence: &'static str,
    /// Where the erratum stands. `concepts/ERRATA.md` mirrors this; CI cannot read that file,
    /// so the state the checks hold the specification to lives here.
    state: State,
}

/// The lifecycle of an erratum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Present at the pin and, as far as anyone has checked, everywhere upstream.
    Open,
    /// Corrected on an upstream branch the pin has not moved to. Still present at the pin, so the
    /// crate's workaround still stands; `--upstream` holds the branch to having fixed it.
    FixedUpstream { branch: &'static str, commit: &'static str },
    /// The pin has moved past the fix. The defect must stay absent at the pin; the entry is kept
    /// for the reader who meets a peer built against the old text.
    Adopted,
}

impl State {
    fn label(self) -> String {
        match self {
            Self::Open => "open".to_owned(),
            Self::FixedUpstream { branch, commit } => format!("fixed upstream on {branch} @ {commit}"),
            Self::Adopted => "adopted".to_owned(),
        }
    }
}

/// Why `text` does not show the erratum; empty when the erratum is there.
fn evaluate(erratum: &Erratum, text: &str) -> Result<Vec<String>, Failure> {
    let mut reasons = Vec::new();
    for pattern in erratum.present {
        if !Regex::new(pattern)?.is_match(text) {
            reasons.push(format!("expected to find {pattern:?}"));
        }
    }
    for pattern in erratum.absent {
        if Regex::new(pattern)?.is_match(text) {
            reasons.push(format!("expected {pattern:?} to be absent, and it is there"));
        }
    }
    Ok(reasons)
}

/// What one upstream ref says about one erratum, compared with what the crate records.
///
/// `is_head` is true for the release's own branch, false for a watched one. Returns the finding
/// to report when the two disagree.
fn upstream_finding(
    erratum: &Erratum,
    branch: &str,
    commit: &str,
    is_head: bool,
    present: bool,
) -> Option<String> {
    let at = format!("{branch} @ {}", crate::spec_sync::short(commit));
    match erratum.state {
        State::Adopted => None,
        State::Open if !present => Some(format!(
            "{} is fixed on {at} but recorded as open: record it as fixed upstream, and re-read what \
             this crate does about it ({})",
            erratum.id, erratum.consequence
        )),
        State::FixedUpstream { branch: fixed_on, .. } if branch == fixed_on && present => {
            Some(format!("{} is recorded as fixed on {fixed_on}, but {at} has it again", erratum.id))
        }
        State::FixedUpstream { .. } if is_head && !present => Some(format!(
            "{} is fixed on the pinned branch's head {at}: move the pin and adopt it",
            erratum.id
        )),
        _ => None,
    }
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
        state: State::Open,
    },
    Erratum {
        id: "E02",
        title: "the Hub Client Info receiver example uses the path `clientinfo` (and, at the pin, version 2.0)",
        release: "ocpi-2.3.0",
        file: "mod_hub_client_info.asciidoc",
        // The hotfix corrects the version and keeps the path, so only the path is held here.
        present: &[r"/\d\.\d(?:\.\d)?/clientinfo"],
        absent: &[],
        consequence: "the builders use the discovered endpoint URL and treat the example as \
                      non-normative",
        state: State::Open,
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
        state: State::Open,
    },
    Erratum {
        id: "E04",
        title: "the payments branch's own ModuleID table does not list `payments`",
        release: "ocpi-2.3.0-payments",
        file: "version_information_endpoint.asciidoc",
        present: &[r"ModuleID _OpenEnum_"],
        absent: &[r"\|<<mod_payments"],
        consequence: "`ModuleId::Payments` exists and documents the omission",
        state: State::Adopted,
    },
    Erratum {
        id: "E05",
        title: "the bookings branch's own ModuleID table does not list `Booking`",
        release: "ocpi-2.3.0-bookings",
        file: "version_information_endpoint.asciidoc",
        present: &[r"ModuleID _OpenEnum_"],
        absent: &[r"\|<<mod_bookings"],
        consequence: "`ModuleId::Booking` exists and documents the omission",
        state: State::Open,
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
        state: State::FixedUpstream { branch: "2.3.0/hotfix/core-editorial", commit: "ac7b49043d" },
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
        state: State::Adopted,
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
        state: State::Adopted,
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
        state: State::Open,
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
        state: State::Open,
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
        state: State::Open,
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
        state: State::Open,
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
        state: State::Open,
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
        state: State::Open,
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
        state: State::Open,
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
        state: State::Open,
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
        state: State::Open,
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
        state: State::Open,
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
        state: State::Open,
    },
    Erratum {
        id: "E20",
        title: "the Complex Tariff example prices 114 minutes at € 1.25/h as € 2.28 (that is € 1.20/h)",
        release: "ocpi-2.3.0",
        file: "mod_tariffs.asciidoc",
        present: &[
            r"\| Charging time \| 114 minutes \| 1\.25 per hour \| 2\.28",
            r"\| Total \| \| \| 12\.28",
        ],
        absent: &[],
        consequence: "the engine follows the rules, not the arithmetic: the worked-example test asserts \
                      € 12.375 / € 13.975 (€ 12.38 / € 13.98 at currency precision)",
        state: State::FixedUpstream { branch: "2.3.0/hotfix/core-editorial", commit: "ac7b49043d" },
    },
    Erratum {
        id: "E21",
        title: "both `Authorization: Token` examples base64-encode a trailing newline",
        release: "ocpi-2.3.0",
        file: "transport_and_format.asciidoc",
        present: &[r"Authorization: Token \S*Cg==", r"Authorization: Token ZXhhbXBsZS10b2tlbgo="],
        absent: &[],
        consequence: "not accommodated: a token is what it is. The likeliest cause of a first \
                      registration failing against a peer that copied the example",
        state: State::FixedUpstream { branch: "2.3.0/hotfix/core-editorial", commit: "ac7b49043d" },
    },
    Erratum {
        id: "E22",
        title: "the CDR example's coordinates are transposed (latitude 3.73, longitude 51.05 for Ghent)",
        release: "ocpi-2.3.0",
        file: "examples/cdr_example.json",
        present: &[r#""latitude": "3\.729944""#],
        absent: &[],
        consequence: "the fixture decodes (both values are in range) and nothing structural can catch it; \
                      recorded so nobody copies the example",
        state: State::FixedUpstream { branch: "2.3.0/hotfix/core-editorial", commit: "ac7b49043d" },
    },
];

/// Re-derives every erratum from the vendored specification.
///
/// # Errors
///
/// Fails when the specification is not vendored, when a file an erratum names cannot be read, or
/// when a pattern does not compile.
pub fn report(root: &Path, check: bool, upstream: bool) -> Result<bool, Failure> {
    let specs = crate::spec_dir(root);
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

        let reasons = evaluate(erratum, &text)?;

        if erratum.state == State::Adopted {
            if reasons.is_empty() {
                stale.push(format!(
                    "{} {}\n     in {}/{}\n     recorded as adopted, and the defect is back at the pin",
                    erratum.id, erratum.title, erratum.release, erratum.file
                ));
            } else {
                standing += 1;
                println!("[ok] {} {} — adopted, and still fixed at the pin", erratum.id, erratum.title);
            }
            continue;
        }
        if reasons.is_empty() {
            standing += 1;
            println!(
                "[ok] {} {} — still in {}/{} ({})",
                erratum.id,
                erratum.title,
                erratum.release,
                erratum.file,
                erratum.state.label()
            );
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

    let news = if upstream { upstream_news(root)? } else { Vec::new() };

    if stale.is_empty() && news.is_empty() {
        println!(
            "\n{standing} erratum/errata re-derived from the specification; every one is where it is recorded"
        );
        return Ok(true);
    }
    if !news.is_empty() {
        println!("\n{} upstream change(s) the errata table does not record yet:\n", news.len());
        for entry in &news {
            println!("  {entry}");
        }
    }
    if stale.is_empty() {
        return Ok(!check);
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

/// Evaluates every erratum that is not adopted at its release's branch head and at every branch
/// the manifest watches for that release, and returns where that disagrees with the recorded state.
fn upstream_news(root: &Path) -> Result<Vec<String>, Failure> {
    use crate::spec_sync::{capture, git, read_manifest, short};

    let mut news = Vec::new();
    for release in read_manifest(&root.join("spec-sources.toml"))? {
        let dir = crate::spec_dir(root).join(&release.name);
        if !dir.join(".git").is_dir() {
            return Err(format!("{}: not a git checkout; run spec-sync --fetch first", release.name).into());
        }
        let errata: Vec<&Erratum> =
            ERRATA.iter().filter(|e| e.release == release.name && e.state != State::Adopted).collect();
        let refs = std::iter::once((release.branch.as_str(), true))
            .chain(release.watch.iter().map(|w| (w.as_str(), false)));
        for (branch, is_head) in refs {
            if git(&dir, &["fetch", "--quiet", "origin", branch]).is_err() {
                news.push(format!(
                    "{}: could not fetch {branch} (deleted or merged upstream?)",
                    release.name
                ));
                continue;
            }
            let commit = capture(&dir, &["rev-parse", "FETCH_HEAD"])?.trim().to_owned();
            println!("[upstream] {} — {branch} @ {}", release.name, short(&commit));
            for erratum in &errata {
                // A file that does not exist on the branch cannot show the erratum.
                let text =
                    capture(&dir, &["show", &format!("{commit}:{}", erratum.file)]).unwrap_or_default();
                let present = evaluate(erratum, &text)?.is_empty();
                if let Some(finding) = upstream_finding(erratum, branch, &commit, is_head, present) {
                    news.push(finding);
                }
            }
        }
    }
    Ok(news)
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

    fn erratum(state: State) -> Erratum {
        Erratum {
            id: "E99",
            title: "t",
            release: "ocpi-2.3.0",
            file: "f",
            present: &["defect"],
            absent: &[],
            consequence: "c",
            state,
        }
    }

    const HOTFIX: State = State::FixedUpstream { branch: "hotfix", commit: "abc" };

    #[test]
    fn evaluation_finds_the_defect_or_names_why_not() {
        let e = erratum(State::Open);
        assert!(evaluate(&e, "a defect here").unwrap().is_empty());
        assert_eq!(evaluate(&e, "fixed").unwrap().len(), 1);
    }

    #[test]
    fn an_open_erratum_fixed_anywhere_upstream_is_news_with_a_name() {
        let news = upstream_finding(&erratum(State::Open), "hotfix", "abcdef1234567", false, false).unwrap();
        assert!(news.contains("E99") && news.contains("hotfix @ abcdef1234"), "{news}");
        assert_eq!(upstream_finding(&erratum(State::Open), "main", "abc", true, true), None);
    }

    #[test]
    fn a_recorded_upstream_fix_is_held_to_its_branch() {
        assert_eq!(upstream_finding(&erratum(HOTFIX), "hotfix", "abc", false, false), None, "as recorded");
        let back = upstream_finding(&erratum(HOTFIX), "hotfix", "abc", false, true).unwrap();
        assert!(back.contains("has it again"), "{back}");
        let merged = upstream_finding(&erratum(HOTFIX), "main", "abc", true, false).unwrap();
        assert!(merged.contains("move the pin"), "{merged}");
        assert_eq!(upstream_finding(&erratum(HOTFIX), "main", "abc", true, true), None, "not merged yet");
    }

    #[test]
    fn an_adopted_erratum_is_not_evaluated_upstream() {
        assert_eq!(upstream_finding(&erratum(State::Adopted), "main", "abc", true, true), None);
    }
}
