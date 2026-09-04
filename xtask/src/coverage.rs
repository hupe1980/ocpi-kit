//! `spec-coverage`: check the crate's fields against the specification's own property tables.
//!
//! The specification defines every object as a table of `Property | Type | Card. | Description`.
//! This parses those tables out of the vendored AsciiDoc and Markdown, parses the field lists out
//! of this crate's Rust source (honouring `#[serde(rename)]` and `#[serde(flatten)]`), and prints
//! the difference.
//!
//! A field the specification has and the crate does not is a **gap**: objects would silently lose
//! it. A field the crate has and the specification does not is a **stray**: either a typo or a
//! deliberate deviation that should be documented.
//!
//! This is the tool that makes "we transcribed the spec" checkable rather than asserted.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;

use crate::{Failure, Fields, join};

/// Which Rust module holds each specification release's objects.
const SOURCES: &[(&str, &str, &str)] = &[
    // (spec directory, crate module directory, human label)
    ("ocpi-2.3.0", "v2_3_0", "2.3.0"),
    ("ocpi-2.2.1", "v2_2_1", "2.2.1"),
    ("ocpi-2.1.1", "v2_1_1", "2.1.1"),
    // The extension branches carry the whole 2.3.0 core plus their own module, and this crate
    // models them in the same `v2_3_0` module behind a feature. Comparing them separately covers
    // the Bookings and Payments objects *and* re-checks that each branch's copy of the core
    // tables still agrees with the core release.
    ("ocpi-2.3.0-bookings", "v2_3_0", "2.3.0 bookings branch"),
    ("ocpi-2.3.0-payments", "v2_3_0", "2.3.0 payments branch"),
];

/// Object names the crate deliberately does not model, with the reason.
const INTENTIONALLY_ABSENT: &[(&str, &str)] = &[
    ("2.1.1", "CredentialsRole"), // 2.1.1 has no roles list; the object is flat
];

/// Fields that exist in one release but not another, and are therefore expected to be "strays"
/// everywhere else.
///
/// The extension branches add fields to *core* objects, and this crate models each release's
/// objects in one module behind a feature flag, so every such field is by construction absent
/// from some other release's property table. Listing them here is what keeps a genuine typo
/// distinguishable from a deliberate cross-release addition.
///
/// `(the release the field belongs to, object, field)`.
const BRANCH_ONLY_FIELDS: &[(&str, &str, &str)] = &[
    // Added by the `bookings` branch to objects the core release also defines.
    ("2.3.0 bookings branch", "CDR", "booking_id"),
    ("2.3.0 bookings branch", "TariffRestrictions", "booking"),
    // Added by the `payments` branch to a core object: the amount a Payment Terminal Provider
    // preauthorizes is a Payments concept, and lives with the module rather than in core.
    ("2.3.0 payments branch", "Tariff", "preauthorize_amount"),
];

/// Compares the crate with the specification and prints a report.
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
        let spec_fields = parse_spec(&specs.join(spec_dir))?;
        let crate_fields = parse_rust(&root.join("src").join(module))?;
        // The per-version modules re-export everything that is wire-identical to 2.3.0, so an
        // object the older module does not redefine is covered by the canonical one.
        let canonical =
            if *module == "v2_3_0" { Fields::new() } else { parse_rust(&root.join("src/v2_3_0"))? };

        println!("\n=== OCPI {label} ===");
        clean &= compare(label, &spec_fields, &crate_fields, &canonical);
    }

    if check && !clean {
        eprintln!("\nspec-coverage found gaps; see above");
    }
    Ok(!check || clean)
}

fn compare(label: &str, spec: &Fields, crate_fields: &Fields, canonical: &Fields) -> bool {
    let mut clean = true;
    let mut covered = 0usize;

    for (object, properties) in spec {
        if INTENTIONALLY_ABSENT.iter().any(|(v, o)| v == &label && o == object) {
            continue;
        }
        let Some(ours) = crate_fields.get(object).or_else(|| canonical.get(object)) else {
            // Not every table in the spec is an object this crate models — the modules it does
            // not implement, and the tables that describe request parameters rather than objects.
            continue;
        };
        covered += 1;

        let expected: BTreeSet<String> = properties.iter().cloned().collect();
        let actual: BTreeSet<String> = ours.iter().cloned().collect();

        let missing: BTreeSet<String> = expected.difference(&actual).cloned().collect();
        let extra: BTreeSet<String> = actual
            .difference(&expected)
            .filter(|f| {
                !BRANCH_ONLY_FIELDS
                    .iter()
                    .any(|(owner, o, field)| o == object && field == f && owner != &label)
            })
            .cloned()
            .collect();

        if missing.is_empty() && extra.is_empty() {
            continue;
        }
        clean = false;
        println!("{object}:");
        if !missing.is_empty() {
            println!("  MISSING in the crate: {}", join(&missing));
        }
        if !extra.is_empty() {
            println!("  not in the spec table: {}", join(&extra));
        }
    }

    if clean {
        println!("{covered} object(s) match the specification's property tables exactly");
    }
    if std::env::args().any(|a| a == "--list") {
        let mut checked: Vec<&String> =
            spec.keys().filter(|o| crate_fields.contains_key(*o) || canonical.contains_key(*o)).collect();
        checked.sort();
        println!("  checked: {}", checked.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
        let mut unchecked: Vec<&String> =
            spec.keys().filter(|o| !crate_fields.contains_key(*o) && !canonical.contains_key(*o)).collect();
        unchecked.sort();
        println!("  not modelled: {}", unchecked.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
    }
    clean
}

/// Reads every property table out of one specification release.
fn parse_spec(dir: &Path) -> Result<Fields, Failure> {
    let mut out = Fields::new();

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
        let mut fields: Vec<String> = Vec::new();

        for line in text.lines() {
            if let Some(name) = heading_object(line).or_else(|| markdown_heading_object(line)) {
                flush(&mut out, current.take(), std::mem::take(&mut fields));
                in_table = false;
                current = Some(name);
                continue;
            }
            let trimmed = line.trim();
            if trimmed == "|===" {
                if in_table {
                    flush(&mut out, current.clone(), std::mem::take(&mut fields));
                }
                in_table = !in_table;
                continue;
            }
            if let Some(property) = property_of(trimmed) {
                fields.push(property);
            }
        }
        flush(&mut out, current, fields);
    }
    Ok(out)
}

/// The object a heading names, if it names one.
///
/// The two markups spell the same thing several ways — `==== _Location_ Object`,
/// `### 3.1 *Location* Object`, `==== Hours _class_`, `=== Role _enum_` — so this strips the
/// decoration rather than trying to match every variant.
fn heading_object(line: &str) -> Option<String> {
    let level_stripped = line.strip_prefix("==")?.trim_start_matches('=').trim_start();
    heading_body(level_stripped)
}

/// The same, for a Markdown heading.
fn markdown_heading_object(line: &str) -> Option<String> {
    let level_stripped = line.strip_prefix("##")?.trim_start_matches('#').trim_start();
    heading_body(level_stripped)
}

fn heading_body(text: &str) -> Option<String> {
    // Drop a leading section number such as "3.1 " or "4.16 ".
    let text = text
        .split_once(' ')
        .filter(|(first, _)| first.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
        .map_or(text, |(_, rest)| rest);
    // Strip emphasis and the trailing kind word.
    let cleaned: String = text.chars().filter(|c| *c != '*' && *c != '_' && *c != '`').collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    while matches!(
        words.last().map(|w| w.to_ascii_lowercase()),
        Some(ref w) if w == "object" || w == "class" || w == "enum" || w == "openenum" || w == "type"
    ) {
        words.pop();
    }
    let name = words.join("");
    let first = name.chars().next()?;
    if !first.is_ascii_uppercase() || name.len() > 40 || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(name)
}

/// Extracts the property name from one row of a property table, in either markup.
fn property_of(line: &str) -> Option<String> {
    if !line.starts_with('|') {
        return None;
    }
    // AsciiDoc anchors appear before the name: `|[[anchor]] parking_places |…`.
    let first = line.trim_start_matches('|');
    let first = match first.find("]]") {
        Some(at) => &first[at + 2..],
        None => first,
    };
    let cell = first.split('|').next()?.trim();
    // OCPI 2.1.1 marks the members of a "Choice: one of two" with a leading ">".
    let name = cell.trim_start_matches('>').trim().trim_matches('`').trim();
    // A property name is a lower-case snake-case identifier. Anything else is a header row, an
    // enum value, or a description that happens to start a line.
    if name.is_empty()
        || name.len() > 40
        || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    Some(name.to_owned())
}

fn flush(out: &mut Fields, name: Option<String>, fields: Vec<String>) {
    let Some(name) = name else { return };
    if fields.is_empty() {
        return;
    }
    // Some objects are described in more than one place; keep the longest table, which is the
    // object definition rather than a summary.
    let entry = out.entry(name).or_default();
    if fields.len() > entry.len() {
        *entry = fields;
    }
}

/// Reads the field list of every `pub struct` in a directory of Rust source.
fn parse_rust(dir: &Path) -> Result<Fields, Failure> {
    let mut out = Fields::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let struct_start = Regex::new(r"^\s*pub struct (\w+) \{$")?;
    let field = Regex::new(r"^\s*pub (\w+):")?;
    let rename = Regex::new(r#"serde\(rename = "([^"]+)""#)?;

    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.sort();

    for file in files {
        let text = std::fs::read_to_string(&file)?;
        let mut current: Option<String> = None;
        let mut fields: Vec<String> = Vec::new();
        let mut pending_rename: Option<String> = None;
        let mut skip_next = false;

        for line in text.lines() {
            if let Some(captures) = struct_start.captures(line) {
                if let Some(name) = current.take() {
                    out.insert(name, std::mem::take(&mut fields));
                }
                current = Some(captures[1].to_owned());
                continue;
            }
            if current.is_none() {
                continue;
            }
            if line.trim() == "}" {
                if let Some(name) = current.take() {
                    out.insert(name, std::mem::take(&mut fields));
                }
                continue;
            }
            if line.trim_start().starts_with("#[serde(") || line.trim_start().starts_with("#[cfg_attr") {
                if let Some(captures) = rename.captures(line) {
                    pending_rename = Some(captures[1].to_owned());
                }
                // `extensions` is this crate's mechanism, not a spec field.
                if line.contains("flatten") {
                    skip_next = true;
                }
                continue;
            }
            if let Some(captures) = field.captures(line) {
                let taken_rename = pending_rename.take();
                if std::mem::take(&mut skip_next) {
                    continue;
                }
                fields.push(taken_rename.unwrap_or_else(|| captures[1].to_owned()));
            }
        }
        if let Some(name) = current {
            out.insert(name, fields);
        }
    }

    // Aliases: the crate names a few objects differently from the specification's heading.
    let aliases: BTreeMap<&str, &str> = BTreeMap::from([
        ("Evse", "EVSE"),
        ("EvseParking", "EVSEParking"),
        ("Cdr", "CDR"),
        ("CdrToken", "CdrToken"),
        ("PublishTokenType", "PublishTokenType"),
    ]);
    for (ours, theirs) in aliases {
        if let Some(fields) = out.get(ours).cloned() {
            out.entry(theirs.to_owned()).or_insert(fields);
        }
    }
    Ok(out)
}
