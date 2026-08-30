//! `enum-coverage`: check the crate's enum *values* against the specification's own tables.
//!
//! `spec-coverage` compares field names and says nothing about what may go in them. A missing enum
//! value is the same defect one level down: on a **closed** enum it makes a conformant peer's
//! object undecodable, and on an **open** one it survives in `Custom("MCS")`, fails nothing, and
//! never matches `ConnectorType::Mcs` in any `match` an integrator writes. No fixture round-trip
//! finds either unless the specification ships an example using that value.
//!
//! This parses the `|Value |Description` tables under every heading the specification marks as an
//! enum, parses the wire strings out of this crate's `ocpi_enum!` / `ocpi_open_enum!` /
//! `ocpi_lenient_enum!` blocks, and prints the difference.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;

use crate::Failure;

/// The enum values of each named type.
type Values = BTreeMap<String, BTreeSet<String>>;

/// Which Rust module holds each specification release's enums. Mirrors `coverage::SOURCES`.
const SOURCES: &[(&str, &str, &str)] = &[
    ("ocpi-2.3.0", "v2_3_0", "2.3.0"),
    ("ocpi-2.2.1", "v2_2_1", "2.2.1"),
    ("ocpi-2.1.1", "v2_1_1", "2.1.1"),
    ("ocpi-2.3.0-bookings", "v2_3_0", "2.3.0 bookings branch"),
    ("ocpi-2.3.0-payments", "v2_3_0", "2.3.0 payments branch"),
];

/// Values that exist in one release's table and not another's, and are therefore expected to look
/// like strays everywhere else.
///
/// This crate models each release's enums in one module behind a feature flag, exactly as it does
/// for fields, so a value added by a branch is by construction absent from another release's
/// table. Listing them is what keeps a genuine typo distinguishable from a deliberate addition.
///
/// `(the release the value belongs to, enum, value)`.
const BRANCH_ONLY_VALUES: &[(&str, &str, &str)] = &[
    // The bookings branch adds two reservation dimensions to the core CDR enum. They are
    // declared unconditionally in the crate — `CdrDimensionType` is closed, so gating them would
    // make a booking-aware CPO's whole CDR undecodable — which makes them strays everywhere else.
    ("2.3.0 bookings branch", "CdrDimensionType", "RESERVATION_EXPIRES"),
    ("2.3.0 bookings branch", "CdrDimensionType", "RESERVATION_OVERTIME"),
];

/// Enums the crate deliberately does not model as an enum, with the reason.
const INTENTIONALLY_ABSENT: &[(&str, &str)] = &[];

/// Compares the crate's enum values with the specification's tables and prints a report.
///
/// # Errors
///
/// Fails when the vendored specification or the crate source cannot be read.
pub fn report(root: &Path, check: bool) -> Result<bool, Failure> {
    let specs = root.join("specs/src");
    if !specs.is_dir() {
        return Err(format!("{} does not exist; vendor the spec first", specs.display()).into());
    }

    let mut clean = true;
    for (spec_dir, module, label) in SOURCES {
        let spec = parse_spec(&specs.join(spec_dir))?;
        let mut crate_values = parse_rust(&root.join("src").join(module))?;
        // The per-version modules re-export every enum that is wire-identical to 2.3.0, so one
        // they do not redefine is covered by the canonical module.
        if *module != "v2_3_0" {
            for (name, values) in parse_rust(&root.join("src/v2_3_0"))? {
                crate_values.entry(name).or_insert(values);
            }
        }
        println!("\n=== OCPI {label} enums ===");
        clean &= compare(label, &spec, &crate_values);
    }

    if clean {
        println!("\nEvery enum the specification defines has exactly the values it lists.");
    } else if check {
        eprintln!(
            "\nA missing value on a closed enum makes a conformant peer's object undecodable; on \
             an open enum it survives as `Custom(_)` and silently never matches. Add it, or \
             record it in this task's tables."
        );
        return Ok(false);
    }
    Ok(true)
}

fn compare(label: &str, spec: &Values, krate: &Values) -> bool {
    let mut clean = true;
    let mut matched = 0usize;

    for (name, expected) in spec {
        if INTENTIONALLY_ABSENT.iter().any(|(l, n)| l == &label && n == name) {
            continue;
        }
        let Some(actual) = krate.get(name) else {
            // An enum the crate does not model at all is `spec-coverage`'s business, not this
            // one: it reports the object, and reporting it twice helps nobody.
            continue;
        };
        let branch_only = |value: &String| BRANCH_ONLY_VALUES.iter().any(|(_, n, v)| n == name && v == value);
        let missing: Vec<&String> = expected.difference(actual).filter(|v| !branch_only(v)).collect();
        let stray: Vec<&String> = actual.difference(expected).filter(|v| !branch_only(v)).collect();

        if missing.is_empty() && stray.is_empty() {
            matched += 1;
            continue;
        }
        clean = false;
        println!("  {name}");
        if !missing.is_empty() {
            println!("      missing: {}", list(&missing));
        }
        if !stray.is_empty() {
            println!("      stray:   {}", list(&stray));
        }
    }
    if clean {
        println!("{matched} enum(s) match the specification's value tables exactly");
    }
    clean
}

/// Every enum table in a directory of specification source.
fn parse_spec(dir: &Path) -> Result<Values, Failure> {
    let mut out = Values::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "asciidoc" || e == "md"))
        .collect();
    files.sort();

    for file in files {
        let text = std::fs::read_to_string(&file)?;
        let mut current: Option<String> = None;
        let mut in_table = false;
        let mut values: BTreeSet<String> = BTreeSet::new();

        for line in text.lines() {
            if let Some(name) = enum_heading(line) {
                flush(&mut out, current.take(), std::mem::take(&mut values));
                in_table = false;
                current = Some(name);
                continue;
            }
            // A non-enum heading ends the previous enum's scope, so a later object's property
            // table is never read as more values.
            if is_heading(line) && enum_heading(line).is_none() {
                flush(&mut out, current.take(), std::mem::take(&mut values));
                in_table = false;
                continue;
            }
            let trimmed = line.trim();
            if trimmed == "|===" {
                if in_table {
                    flush(&mut out, current.clone(), std::mem::take(&mut values));
                }
                in_table = !in_table;
                continue;
            }
            if current.is_some()
                && let Some(value) = value_of(trimmed)
            {
                values.insert(value);
            }
        }
        flush(&mut out, current, values);
    }
    Ok(out)
}

fn list(values: &[&String]) -> String {
    values.iter().map(|v| v.as_str()).collect::<Vec<_>>().join(", ")
}

fn is_heading(line: &str) -> bool {
    line.starts_with("==") || line.starts_with("##")
}

/// The enum a heading names, if it names one.
///
/// Both markups mark the kind in the heading: `==== ConnectorType _OpenEnum_`,
/// `=== Role _enum_`, `### 14.4.3 *AuthMethod* *enum*`.
fn enum_heading(line: &str) -> Option<String> {
    let body = line.strip_prefix("==").or_else(|| line.strip_prefix("##"))?;
    let body = body.trim_start_matches(['=', '#']).trim_start();
    let cleaned: String = body.chars().filter(|c| *c != '*' && *c != '_' && *c != '`').collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    // Drop a leading section number such as "14.4.3".
    if words.first().is_some_and(|w| w.bytes().all(|b| b.is_ascii_digit() || b == b'.')) {
        words.remove(0);
    }
    let kind = words.pop()?.to_ascii_lowercase();
    if kind != "enum" && kind != "openenum" {
        return None;
    }
    let name = words.join("");
    let first = name.chars().next()?;
    (first.is_ascii_uppercase() && name.len() <= 40 && name.chars().all(char::is_alphanumeric))
        .then_some(name)
}

/// The value in the first cell of an enum table row, if the row carries one.
fn value_of(line: &str) -> Option<String> {
    if !line.starts_with('|') {
        return None;
    }
    let first = line.trim_start_matches('|');
    let first = match first.find("]]") {
        Some(at) => &first[at + 2..],
        None => first,
    };
    let cell = first.split('|').next()?.trim().trim_matches('`').trim();
    // A wire value is an identifier starting with an ASCII letter. That also matches the `Value`
    // header cell, which is why it is excluded by name — every real OCPI value is either all
    // upper case or contains an underscore, and `Value` is neither.
    if cell.is_empty()
        || cell.len() > 48
        || cell == "Value"
        || !cell.chars().next()?.is_ascii_alphabetic()
        // `N/A` is a wire value in `TaxIncluded`, so `/` belongs in the character set.
        || !cell.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'/')
    {
        return None;
    }
    // A description sentence never survives the character test, but a single capitalised English
    // word can. Every OCPI enum value is upper case or snake case.
    let shouty = cell.chars().all(|c| !c.is_ascii_lowercase());
    if !shouty && !cell.contains('_') {
        return None;
    }
    Some(cell.to_owned())
}

fn flush(out: &mut Values, name: Option<String>, values: BTreeSet<String>) {
    let Some(name) = name else { return };
    if values.is_empty() {
        return;
    }
    // An enum described in more than one place keeps the longest table, which is the definition
    // rather than a summary.
    let entry = out.entry(name).or_default();
    if values.len() > entry.len() {
        *entry = values;
    }
}

/// The wire values of every enum this crate declares with one of the three enum macros.
fn parse_rust(dir: &Path) -> Result<Values, Failure> {
    let mut out = Values::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let start = Regex::new(r"^\s*pub enum (\w+) \{$")?;
    let value = Regex::new(r#"^\s*(\w+) = "([^"]+)","#)?;

    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.sort();

    for file in files {
        let text = std::fs::read_to_string(&file)?;
        let mut current: Option<(String, BTreeSet<String>)> = None;
        let mut depth = 0i32;

        for line in text.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if let Some(name) = start.captures(line).map(|c| c[1].to_owned()) {
                if let Some((name, values)) = current.take() {
                    flush(&mut out, Some(name), values);
                }
                current = Some((name, BTreeSet::new()));
                depth = 1;
                continue;
            }
            if let Some((_, values)) = current.as_mut() {
                if let Some(caps) = value.captures(line) {
                    values.insert(caps[2].to_owned());
                }
                depth += i32::try_from(line.matches('{').count()).unwrap_or(0);
                depth -= i32::try_from(line.matches('}').count()).unwrap_or(0);
                if depth <= 0
                    && let Some((name, values)) = current.take()
                {
                    flush(&mut out, Some(name), values);
                }
            }
        }
        if let Some((name, values)) = current {
            flush(&mut out, Some(name), values);
        }
    }
    Ok(out)
}
