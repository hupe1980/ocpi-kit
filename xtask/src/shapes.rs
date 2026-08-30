//! `field-shapes`: check each field's **cardinality and length** against the specification.
//!
//! `spec-coverage` compares field names and `enum-coverage` compares enum values. A property table
//! has four columns, and between them those two checks read one: nothing looks at `Card.` or at
//! the length in `CiString(36)`.
//!
//! Both are load-bearing. A field the spec marks `?` and the crate makes required rejects a
//! conformant peer's object outright. One marked `1` and modelled `Option` lets this crate emit an
//! object that is missing a mandatory field. A length that is too small reports a conformant value
//! as `TooLong` — and with `ClientConfig::validate_outgoing` on by default, refuses to send it.
//!
//! That last one is not hypothetical: `SignedData.url` is a `string(512)`, not the `string(255)`
//! `URL` type every other URL-shaped field uses, and modelling it as a `Url` made this crate
//! reject a conformant link.

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;

use crate::Failure;

/// Which Rust module holds each release's objects. Mirrors `coverage::SOURCES`.
const SOURCES: &[(&str, &str, &str)] = &[
    ("ocpi-2.3.0", "v2_3_0", "2.3.0"),
    ("ocpi-2.2.1", "v2_2_1", "2.2.1"),
    ("ocpi-2.1.1", "v2_1_1", "2.1.1"),
    ("ocpi-2.3.0-bookings", "v2_3_0", "2.3.0 bookings branch"),
    ("ocpi-2.3.0-payments", "v2_3_0", "2.3.0 payments branch"),
];

/// The crate's semantic types, and the spec type each one is exactly.
///
/// Without these every `party_id: PartyId` reads as a missing length. They are checked rather than
/// trusted: `resolve` fails if one of these aliases stops saying what it says here.
const ALIASES: &[(&str, &str, Option<u32>)] = &[
    ("PartyId", "CiString", Some(3)),
    ("CountryCode", "CiString", Some(2)),
    ("EvseId", "CiString", Some(48)),
    ("ContractId", "CiString", Some(36)),
    ("Currency", "string", Some(3)),
    // Structural types that enforce more than a length: `LocalTime` is `HH:MM` and `LocalDate` is
    // `YYYY-MM-DD`, which is what `string(5)` and `string(10)` are spelling out.
    ("LocalTime", "string", Some(5)),
    ("LocalDate", "string", Some(10)),
    ("Url", "URL", None),
    ("DateTime", "DateTime", None),
    ("Number", "number", None),
];

/// Fields whose Rust type deliberately differs from a table, with the reason.
///
/// Every entry here is a release disagreeing with another release, not the crate disagreeing with
/// the specification: this crate models each release's objects in one module, so it follows the
/// newest table and the older one then reads as a mismatch.
const DELIBERATE: &[(&str, &str, &str, &str)] = &[
    // The `bookings` branch forked before core 2.3.0 widened these, and was never re-synced.
    ("2.3.0 bookings branch", "Location", "address", "core 2.3.0 widened it from 45 to 255"),
    ("2.3.0 bookings branch", "Location", "state", "core 2.3.0 widened it from 20 to 45"),
    (
        "2.3.0 bookings branch",
        "SignedValue",
        "plain_data",
        "512 is the value core 2.3.0 corrected to 5000; see the errata reference",
    ),
    // A documented specification defect, not a branch difference.
    (
        "2.1.1",
        "Hours",
        "twentyfourseven",
        "2.1.1 marks it `1` and then says \"Choice: one of two\", which makes it optional",
    ),
];

/// Compares each field's shape with the specification's and prints a report.
///
/// # Errors
///
/// Fails when the vendored specification or the crate source cannot be read, or when one of the
/// [`ALIASES`] no longer resolves to what it claims.
pub fn report(root: &Path, check: bool) -> Result<bool, Failure> {
    let specs = root.join("specs/src");
    if !specs.is_dir() {
        return Err(format!("{} does not exist; vendor the spec first", specs.display()).into());
    }
    verify_aliases(root)?;

    let mut clean = true;
    for (spec_dir, module, label) in SOURCES {
        let spec = parse_spec(&specs.join(spec_dir))?;
        let mut fields = parse_rust(&root.join("src").join(module))?;
        if *module != "v2_3_0" {
            for (name, decl) in parse_rust(&root.join("src/v2_3_0"))? {
                fields.entry(name).or_insert(decl);
            }
        }
        println!("\n=== OCPI {label} field shapes ===");
        clean &= compare(label, &spec, &fields);
    }

    if clean {
        println!("\nEvery field's cardinality and length matches the specification's tables.");
    } else if check {
        eprintln!(
            "\nA cardinality mismatch makes this crate reject a conformant object or emit an \
             incomplete one; a length that is too small reports a conformant value as TooLong and \
             refuses to send it."
        );
        return Ok(false);
    }
    Ok(true)
}

/// What a property table says about one field.
#[derive(Debug)]
struct SpecField {
    base: String,
    length: Option<u32>,
    /// `1`, `?`, `*` or `+`.
    card: char,
}

type SpecObjects = BTreeMap<String, BTreeMap<String, SpecField>>;
type RustObjects = BTreeMap<String, BTreeMap<String, String>>;

fn compare(label: &str, spec: &SpecObjects, rust: &RustObjects) -> bool {
    let mut clean = true;
    let mut checked = 0usize;

    for (object, fields) in spec {
        let Some(declared) = rust.get(object) else { continue };
        for (name, want) in fields {
            let Some(ty) = declared.get(name) else { continue };
            if DELIBERATE.iter().any(|(l, o, f, _)| l == &label && o == object && f == name) {
                continue;
            }
            checked += 1;
            for problem in problems(want, ty) {
                clean = false;
                println!("  {object}::{name}\n      {problem} (Rust: `{ty}`)");
            }
        }
    }
    if clean {
        println!("{checked} field(s) match the specification's cardinality and length");
    }
    clean
}

/// Every way one field's Rust type disagrees with its table row.
fn problems(want: &SpecField, ty: &str) -> Vec<String> {
    let mut out = Vec::new();
    let optional = ty.starts_with("Option<");
    let list = ty.starts_with("Vec<");
    match want.card {
        '?' if !optional => {
            out.push("the table marks it optional (`?`); the field is not an `Option`".to_owned())
        }
        '1' if optional => out.push("the table marks it required (`1`); the field is an `Option`".to_owned()),
        '1' | '?' if list => {
            out.push(format!("the table marks it `{}`, not a list; the field is a `Vec`", want.card))
        }
        '*' | '+' if !list => {
            out.push(format!("the table marks it a list (`{}`); the field is not a `Vec`", want.card))
        }
        _ => {}
    }
    if let Some(want_len) = want.length {
        let inner = ty.trim_start_matches("Option<").trim_start_matches("Vec<").trim_end_matches('>');
        let (base, got_len) = resolve(inner);
        match got_len {
            Some(got) if got != want_len => {
                out.push(format!("the table says {}({want_len}); the field holds {base}({got})", want.base))
            }
            None if matches!(want.base.as_str(), "CiString" | "string") => out.push(format!(
                "the table says {}({want_len}); the field's type carries no length",
                want.base
            )),
            _ => {}
        }
    }
    out
}

/// A Rust type reduced to `(base, length)`, resolving this crate's aliases.
fn resolve(ty: &str) -> (String, Option<u32>) {
    if let Some((name, len)) = ty.split_once('<')
        && let Ok(len) = len.trim_end_matches('>').parse()
    {
        return (name.to_owned(), Some(len));
    }
    for (alias, base, len) in ALIASES {
        if ty == *alias {
            return ((*base).to_owned(), *len);
        }
    }
    (ty.to_owned(), None)
}

/// Fails if an alias in [`ALIASES`] no longer means what the table says it does.
///
/// The alias list is the one place this check trusts something other than the source, so it is the
/// one place worth proving. `PartyId` silently becoming a `CiString<4>` would otherwise make every
/// `party_id` in the crate pass a check it no longer satisfies.
fn verify_aliases(root: &Path) -> Result<(), Failure> {
    let src = std::fs::read_to_string(root.join("src/types/ids.rs"))?;
    let decl = Regex::new(r"pub type (\w+) = (\w+)<(\d+)>;")?;
    let found: BTreeMap<&str, (String, u32)> = decl
        .captures_iter(&src)
        .map(|c| {
            let len = c[3].parse().unwrap_or_default();
            (c.get(1).expect("group 1").as_str(), (c[2].to_owned(), len))
        })
        .collect();
    for (alias, base, len) in ALIASES {
        let Some((actual_base, actual_len)) = found.get(alias) else { continue };
        let want_base = if *base == "string" { "OcpiString" } else { *base };
        if actual_base != want_base || Some(*actual_len) != *len {
            return Err(format!(
                "`{alias}` is declared as `{actual_base}<{actual_len}>`, but this check's alias \
                 table says it is {base}({len:?}). Update the table."
            )
            .into());
        }
    }
    Ok(())
}

/// Every property table in a directory of specification source.
fn parse_spec(dir: &Path) -> Result<SpecObjects, Failure> {
    let mut out = SpecObjects::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "asciidoc" || e == "md"))
        .collect();
    files.sort();

    for file in files {
        let text = std::fs::read_to_string(&file)?;
        let mut current: Option<String> = None;
        for line in text.lines() {
            if let Some(name) = heading_object(line) {
                current = Some(name);
                continue;
            }
            if line.starts_with("==") || line.starts_with("##") {
                current = None;
                continue;
            }
            let (Some(object), Some((name, field))) = (current.as_ref(), row(line)) else { continue };
            // An object described in more than one place keeps the fullest table.
            out.entry(object.clone()).or_default().entry(name).or_insert(field);
        }
    }
    Ok(out)
}

/// The object a heading names, if it names one. Mirrors `coverage::heading_object`.
fn heading_object(line: &str) -> Option<String> {
    let body = line.strip_prefix("==").or_else(|| line.strip_prefix("##"))?;
    let body = body.trim_start_matches(['=', '#']).trim_start();
    let cleaned: String = body.chars().filter(|c| !matches!(c, '*' | '_' | '`')).collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    if words.first().is_some_and(|w| w.bytes().all(|b| b.is_ascii_digit() || b == b'.')) {
        words.remove(0);
    }
    while words
        .last()
        .map(|w| w.to_ascii_lowercase())
        .is_some_and(|w| matches!(w.as_str(), "object" | "class" | "enum" | "openenum" | "type"))
    {
        words.pop();
    }
    let name = words.join("");
    let first = name.chars().next()?;
    (first.is_ascii_uppercase() && name.len() <= 40 && name.chars().all(char::is_alphanumeric))
        .then_some(name)
}

/// One `|name |Type(N) |card |description` row.
fn row(line: &str) -> Option<(String, SpecField)> {
    if !line.starts_with('|') {
        return None;
    }
    let cells: Vec<&str> = line.split('|').skip(1).collect();
    if cells.len() < 3 {
        return None;
    }
    let name = cells[0].trim().trim_start_matches('>').trim().trim_matches('`').trim();
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
        return None;
    }
    let card = cells[2].trim().chars().next().filter(|c| matches!(c, '1' | '?' | '*' | '+'))?;
    let ty = cells[1].trim();
    // `<<types.asciidoc#types_cistring_type,CiString>>(36)`, or a bare `boolean` / `int`.
    let base = ty
        .rsplit_once(',')
        .and_then(|(_, rest)| rest.split_once(">>"))
        .map_or_else(|| ty.split('(').next().unwrap_or(ty).trim().to_owned(), |(b, _)| b.to_owned());
    let length = ty.rsplit_once('(').and_then(|(_, n)| n.trim_end_matches(')').trim().parse().ok());
    Some((name.to_owned(), SpecField { base, length, card }))
}

/// The declared type of every field of every `pub struct` in a directory of Rust source.
fn parse_rust(dir: &Path) -> Result<RustObjects, Failure> {
    let mut out = RustObjects::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let start = Regex::new(r"^\s*pub struct (\w+) \{$")?;
    let field = Regex::new(r"^\s*pub (\w+): (.+),$")?;
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
        let mut renamed: Option<String> = None;
        for line in text.lines() {
            if let Some(name) = start.captures(line).map(|c| c[1].to_owned()) {
                current = Some(name);
                continue;
            }
            if line == "}" {
                current = None;
                continue;
            }
            let Some(object) = current.as_ref() else { continue };
            if let Some(caps) = rename.captures(line) {
                renamed = Some(caps[1].to_owned());
                continue;
            }
            if let Some(caps) = field.captures(line) {
                let name = renamed.take().unwrap_or_else(|| caps[1].to_owned());
                out.entry(object.clone()).or_default().insert(name, caps[2].trim().to_owned());
            }
        }
    }
    Ok(out)
}
