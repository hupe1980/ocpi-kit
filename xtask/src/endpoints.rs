//! `endpoints`: the URLs this crate builds, against the URL structures the specification writes.
//!
//! The three censuses compare *objects*. Nothing compared **URLs** — and a URL is where an OCPI
//! integration actually breaks: a missing owner segment, a query parameter with the wrong name, a
//! sub-path invented rather than discovered. Two of the recorded errata are URL problems.
//!
//! Each case below carries the endpoint structure **verbatim** as the specification writes it, and
//! the check does two things with it:
//!
//! 1. asserts the pattern is still in that chapter, character for character — so a specification
//!    that rewords a URL fails here rather than silently diverging;
//! 2. expands it with canned values and compares the result with what the crate's own builder
//!    produces for the same values — so a builder that drifts fails here too.
//!
//! Neither half is a restatement of the other: the first is anchored in the specification, the
//! second in the code, and this check is the only place they meet.
//!
//! ```text
//! cargo run -p xtask -- endpoints            # report
//! cargo run -p xtask -- endpoints --check    # …and fail on a difference (CI)
//! ```

use std::path::Path;

use ocpi_kit::transport::{PageQuery, ReceiverEndpoint, SenderEndpoint};
use ocpi_kit::types::{PartyRef, Url};

use crate::Failure;

/// The base every case is built from. Deliberately carries a path and no trailing slash, which is
/// what a discovered endpoint looks like.
const BASE: &str = "https://example.com/ocpi/2.3.0/x";

/// One documented URL structure, and the builder that is supposed to produce it.
struct Case {
    /// Directory under `specs/src/`.
    release: &'static str,
    /// The chapter the structure appears in.
    file: &'static str,
    /// The structure, exactly as the specification writes it (with `&amp;` as it appears).
    pattern: &'static str,
    /// The values to expand it with. A bracketed group survives only when every `{placeholder}`
    /// inside it has a value here.
    params: &'static [(&'static str, &'static str)],
    /// What the specification's endpoint variable resolves to, relative to the discovered
    /// endpoint this crate starts from.
    ///
    /// Empty for every module whose one endpoint *is* the discovered one. Payments is the
    /// exception, and the reason this field exists: the chapter declares one module identifier and
    /// then addresses two interfaces through `{payments_terminals_endpoint_url}` and
    /// `{payments_financial_advice_confirmation_endpoint_url}`, which version discovery cannot
    /// express. Writing the sub-path here states this crate's reading — the discovered `payments`
    /// endpoint is the base those two hang off — where a reviewer can disagree with it.
    endpoint_suffix: &'static str,
    /// What this crate builds for those values.
    built: fn(&Url) -> Url,
}

fn base() -> Url {
    Url::new(BASE).expect("a valid base URL")
}

fn party() -> PartyRef {
    PartyRef::new("NL", "TNM").expect("a valid party")
}

/// Every URL structure this crate implements.
const CASES: &[Case] = &[
    // --- Locations ---------------------------------------------------------------------------
    Case {
        release: "ocpi-2.3.0",
        file: "mod_locations.asciidoc",
        pattern: "{locations_endpoint_url}?[date_from={date_from}]&amp;[date_to={date_to}]&amp;[offset={offset}]&amp;[limit={limit}]",
        params: &[("offset", "50"), ("limit", "10")],
        endpoint_suffix: "",
        built: |base| {
            SenderEndpoint::new(base.clone()).list(&PageQuery::new().with_offset(50).with_limit(10))
        },
    },
    Case {
        release: "ocpi-2.3.0",
        file: "mod_locations.asciidoc",
        pattern: "{locations_endpoint_url}/{location_id}[/{evse_uid}][/{connector_id}]",
        params: &[("location_id", "LOC1"), ("evse_uid", "3256"), ("connector_id", "1")],
        endpoint_suffix: "",
        built: |base| SenderEndpoint::new(base.clone()).location("LOC1", Some("3256"), Some("1")),
    },
    Case {
        release: "ocpi-2.3.0",
        file: "mod_locations.asciidoc",
        pattern: "{locations_endpoint_url}/{location_id}[/{evse_uid}][/{connector_id}]",
        params: &[("location_id", "LOC1")],
        endpoint_suffix: "",
        built: |base| SenderEndpoint::new(base.clone()).location("LOC1", None, None),
    },
    Case {
        release: "ocpi-2.3.0",
        file: "mod_locations.asciidoc",
        pattern: "{locations_endpoint_url}/{country_code}/{party_id}/{location_id}[/{evse_uid}][/{connector_id}]",
        params: &[("country_code", "NL"), ("party_id", "TNM"), ("location_id", "LOC1"), ("evse_uid", "3256")],
        endpoint_suffix: "",
        built: |base| ReceiverEndpoint::new(base.clone()).location(&party(), "LOC1", Some("3256"), None),
    },
    // --- Sessions ----------------------------------------------------------------------------
    Case {
        release: "ocpi-2.3.0",
        file: "mod_sessions.asciidoc",
        pattern: "{sessions_endpoint_url}/{session_id}/charging_preferences",
        params: &[("session_id", "101")],
        endpoint_suffix: "",
        built: |base| SenderEndpoint::new(base.clone()).charging_preferences("101"),
    },
    Case {
        release: "ocpi-2.3.0",
        file: "mod_sessions.asciidoc",
        pattern: "{sessions_endpoint_url}/{country_code}/{party_id}/{session_id}",
        params: &[("country_code", "NL"), ("party_id", "TNM"), ("session_id", "101")],
        endpoint_suffix: "",
        built: |base| ReceiverEndpoint::new(base.clone()).object(&party(), "101"),
    },
    // --- CDRs --------------------------------------------------------------------------------
    Case {
        release: "ocpi-2.3.0",
        file: "mod_cdrs.asciidoc",
        pattern: "{cdr_endpoint_url}?[date_from={date_from}]&amp;[date_to={date_to}]&amp;[offset={offset}]&amp;[limit={limit}]",
        params: &[("offset", "0"), ("limit", "100")],
        endpoint_suffix: "",
        built: |base| {
            SenderEndpoint::new(base.clone()).list(&PageQuery::new().with_offset(0).with_limit(100))
        },
    },
    // --- Tariffs -----------------------------------------------------------------------------
    Case {
        release: "ocpi-2.3.0",
        file: "mod_tariffs.asciidoc",
        pattern: "{tariffs_endpoint_url}/{country_code}/{party_id}/{tariff_id}",
        params: &[("country_code", "NL"), ("party_id", "TNM"), ("tariff_id", "12")],
        endpoint_suffix: "",
        built: |base| ReceiverEndpoint::new(base.clone()).object(&party(), "12"),
    },
    // --- Tokens ------------------------------------------------------------------------------
    Case {
        release: "ocpi-2.3.0",
        file: "mod_tokens.asciidoc",
        pattern: "{token_endpoint_url}/{country_code}/{party_id}/{token_uid}[?type={type}]",
        params: &[("country_code", "NL"), ("party_id", "TNM"), ("token_uid", "012345678"), ("type", "RFID")],
        endpoint_suffix: "",
        built: |base| ReceiverEndpoint::new(base.clone()).token(&party(), "012345678", Some("RFID")),
    },
    Case {
        release: "ocpi-2.3.0",
        file: "mod_tokens.asciidoc",
        // Written with no separator between the endpoint URL and the id — see the report.
        pattern: "{tokens_endpoint_url}{token_uid}/authorize[?type={type}]",
        params: &[("token_uid", "012345678")],
        endpoint_suffix: "",
        built: |base| SenderEndpoint::new(base.clone()).token_authorize("012345678", None),
    },
    // --- Commands ----------------------------------------------------------------------------
    Case {
        release: "ocpi-2.3.0",
        file: "mod_commands.asciidoc",
        pattern: "{commands_endpoint_url}{command}",
        params: &[("command", "START_SESSION")],
        endpoint_suffix: "",
        built: |base| SenderEndpoint::new(base.clone()).command("START_SESSION"),
    },
    // --- Charging Profiles -------------------------------------------------------------------
    Case {
        release: "ocpi-2.3.0",
        file: "mod_charging_profiles.asciidoc",
        pattern: "{chargingprofiles_endpoint_url}{session_id}",
        params: &[("session_id", "101")],
        endpoint_suffix: "",
        built: |base| ReceiverEndpoint::new(base.clone()).charging_profile("101"),
    },
    // --- Payments ------------------------------------------------------------------------------
    //
    // The Payments chapter declares one module identifier and then addresses its two interfaces
    // through two endpoint URL variables, which version discovery cannot express (see the
    // errata). These cases are what pins this crate's reading of that: the discovered `payments`
    // endpoint is the base, and `terminals` and `financial-advice-confirmations` hang off it.
    Case {
        release: "ocpi-2.3.0-payments",
        file: "mod_payments.asciidoc",
        pattern: "{payments_terminals_endpoint_url}/{terminal_id}/deactivate",
        params: &[("terminal_id", "TERM-042")],
        endpoint_suffix: "/terminals",
        built: |base| SenderEndpoint::new(base.join("terminals")).terminal_deactivate("TERM-042"),
    },
    Case {
        release: "ocpi-2.3.0-payments",
        file: "mod_payments.asciidoc",
        pattern: "{payments_terminals_endpoint_url}/activate",
        params: &[],
        endpoint_suffix: "/terminals",
        built: |base| SenderEndpoint::new(base.join("terminals")).terminal_activate(),
    },
    Case {
        release: "ocpi-2.3.0-payments",
        file: "mod_payments.asciidoc",
        pattern: "{payments_terminals_endpoint_url}?[date_from={date_from}]&amp;[date_to={date_to}]&amp;[offset={offset}]&amp;[limit={limit}]",
        params: &[("limit", "25")],
        endpoint_suffix: "/terminals",
        built: |base| {
            SenderEndpoint::new(base.clone()).payments_terminals().list(&PageQuery::new().with_limit(25))
        },
    },
    Case {
        release: "ocpi-2.3.0-payments",
        file: "mod_payments.asciidoc",
        pattern: "{payments_financial_advice_confirmation_endpoint_url}?[date_from={date_from}]&amp;[date_to={date_to}]&amp;[offset={offset}]&amp;[limit={limit}]",
        params: &[("limit", "25")],
        endpoint_suffix: "/financial-advice-confirmations",
        built: |base| {
            SenderEndpoint::new(base.clone())
                .payments_financial_advice_confirmations()
                .list(&PageQuery::new().with_limit(25))
        },
    },
];

/// Compares every documented URL structure with what this crate builds.
///
/// # Errors
///
/// Fails when the specification is not vendored or a chapter cannot be read.
pub fn report(root: &Path, check: bool) -> Result<bool, Failure> {
    let specs = root.join("specs/src");
    if !specs.is_dir() {
        return Err(format!(
            "{} does not exist; run `cargo run -p xtask -- spec-sync --fetch` first",
            specs.display()
        )
        .into());
    }

    let base = base();
    let mut clean = true;
    let mut checked = 0usize;
    for case in CASES {
        let path = specs.join(case.release).join(case.file);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;

        // Half one: the specification still writes this structure.
        if !text.contains(case.pattern) {
            clean = false;
            println!("[x] {} no longer documents\n      {}", case.file, case.pattern);
            continue;
        }

        // Half two: the builder produces what the structure describes.
        let expected = expand(case.pattern, case.params, &format!("{BASE}{}", case.endpoint_suffix));
        let actual = (case.built)(&base);
        if actual.as_str() == expected {
            checked += 1;
            println!("[+] {}", trim_base(actual.as_str()));
        } else {
            clean = false;
            println!(
                "[x] {}\n      spec:  {}\n      built: {}",
                case.pattern,
                trim_base(&expected),
                trim_base(actual.as_str())
            );
        }
    }

    if clean {
        println!("\n{checked} endpoint URL(s) match the structures the specification documents");
    } else {
        println!(
            "\nA URL this crate builds is not the one the specification writes. Either the \
             builder drifted, or the specification did — `spec-sync --latest` says which."
        );
    }
    Ok(clean || !check)
}

/// Expands one documented structure into the URL it describes.
///
/// `[…]` groups survive only when every placeholder inside them has a value, which is how one
/// pattern covers `/{location_id}`, `/{location_id}/{evse_uid}` and all three segments.
fn expand(pattern: &str, params: &[(&str, &str)], base: &str) -> String {
    let mut out = String::new();
    let mut rest = pattern.replace("&amp;", "&");

    // The endpoint variable is the base, and the specification is inconsistent about whether a
    // separator follows it — `{commands_endpoint_url}{command}` against
    // `{locations_endpoint_url}/{location_id}`. Both mean the same endpoint plus one segment.
    if let Some(end) = rest.find('}') {
        rest = rest[end + 1..].to_owned();
    }
    out.push_str(base);
    if !rest.is_empty() && !rest.starts_with('/') && !rest.starts_with('?') {
        out.push('/');
    }

    let mut in_query = false;
    let mut wrote_param = false;
    let mut chars = rest.chars().peekable();
    let mut buffer = String::new();
    while let Some(c) = chars.next() {
        match c {
            '[' => {
                // Collect the group, then keep it only if every placeholder in it is known.
                let mut group = String::new();
                let mut depth = 1;
                for g in chars.by_ref() {
                    match g {
                        '[' => depth += 1,
                        ']' if depth == 1 => break,
                        ']' => depth -= 1,
                        _ => {}
                    }
                    group.push(g);
                }
                let Some(expanded) = substitute(&group, params) else { continue };
                // A group that carries its own `?` starts the query where the pattern did not.
                let expanded = expanded.strip_prefix('?').map_or(expanded.clone(), |rest| {
                    in_query = true;
                    rest.to_owned()
                });
                if in_query {
                    if wrote_param {
                        buffer.push('&');
                    } else if !buffer.ends_with('?') {
                        buffer.push('?');
                    }
                    wrote_param = true;
                }
                buffer.push_str(&expanded);
            }
            // A separator between two optional groups. Which separator is actually needed
            // depends on which groups survived, so it is written when a group is kept.
            '&' => {}
            '?' => {
                in_query = true;
                buffer.push('?');
            }
            _ => buffer.push(c),
        }
    }
    out.push_str(&substitute(&buffer, params).unwrap_or_default());
    out
}

/// Substitutes `{name}` placeholders, or `None` when one has no value.
fn substitute(text: &str, params: &[(&str, &str)]) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let end = rest[start..].find('}')? + start;
        out.push_str(&rest[..start]);
        let name = &rest[start + 1..end];
        let value = params.iter().find(|(key, _)| *key == name)?.1;
        out.push_str(value);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn trim_base(url: &str) -> &str {
    url.strip_prefix(BASE).unwrap_or(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_survives_only_when_all_its_placeholders_are_known() {
        let pattern = "{locations_endpoint_url}/{location_id}[/{evse_uid}][/{connector_id}]";
        assert_eq!(expand(pattern, &[("location_id", "LOC1")], "https://e/x"), "https://e/x/LOC1");
        assert_eq!(
            expand(pattern, &[("location_id", "LOC1"), ("evse_uid", "3256")], "https://e/x"),
            "https://e/x/LOC1/3256"
        );
    }

    #[test]
    fn optional_query_parameters_are_joined_with_the_right_separator() {
        let pattern = "{cdr_endpoint_url}?[date_from={date_from}]&amp;[offset={offset}]&amp;[limit={limit}]";
        assert_eq!(
            expand(pattern, &[("offset", "50"), ("limit", "10")], "https://e/x"),
            "https://e/x?offset=50&limit=10"
        );
    }

    #[test]
    fn a_structure_written_without_a_separator_still_means_one_segment() {
        // `{commands_endpoint_url}{command}` — the specification is inconsistent about this.
        assert_eq!(
            expand("{commands_endpoint_url}{command}", &[("command", "START_SESSION")], "https://e/x"),
            "https://e/x/START_SESSION"
        );
    }
}
