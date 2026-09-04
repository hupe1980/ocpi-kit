//! `validate-coverage`: every field of every wire object reaches the validator.
//!
//! `Validate` is how this crate keeps its central promise — parse permissively, report every
//! deviation with a JSON Pointer — and it is written by hand, one `validate_fields!` call per
//! object. A field added to a struct and *not* added to that call is invisible: the object
//! decodes, validates clean, and a peer's over-long identifier or imprecise number passes through
//! unreported. Nothing else notices, because the field census compares the struct with the
//! specification and the validator is not part of that comparison.
//!
//! This closes that: for every wire struct, the fields it declares are compared with the fields
//! its `Validate` impl passes to the validator. It is the same idea as `dead-config` — a promise
//! the code makes that nothing was checking — one layer up.
//!
//! ```text
//! cargo run -p xtask -- validate-coverage            # report
//! cargo run -p xtask -- validate-coverage --check    # …and fail (CI)
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;

use crate::Failure;

/// What ends a `validate_fields!` call: its arguments span several lines and contain no other
/// `);`, so this is the terminator the parser looks for.
const MACRO_END: &str = ");";

/// The modules whose structs are wire objects.
const MODULES: &[&str] = &["src/v2_3_0", "src/v2_2_1", "src/v2_1_1"];

/// Field types with a no-op `Validate` impl: there is nothing about them to report.
///
/// This is deliberately type-driven rather than a list of field names. A `bool` carries no
/// constraint the specification could state, so passing one to the validator is noise; a
/// `CiString`, a `Number` or any enum carries one, so omitting it is a rule this crate promises
/// and does not keep.
const NO_CONSTRAINT: &[&str] = &["bool", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64"];

/// Whether a declared field type carries anything the validator could report.
fn carries_a_constraint(mut ty: &str) -> bool {
    ty = ty.trim();
    // Peel the containers, which delegate: `Option<T>`, `Vec<T>`, `Box<T>`.
    loop {
        let peeled = ["Option<", "Vec<", "Box<"]
            .iter()
            .find_map(|prefix| ty.strip_prefix(*prefix).and_then(|rest| rest.strip_suffix('>')));
        match peeled {
            Some(inner) => ty = inner.trim(),
            None => break,
        }
    }
    !NO_CONSTRAINT.contains(&ty)
}

/// Compares each wire struct's fields with what its `Validate` impl checks.
///
/// # Errors
///
/// Fails when the crate source cannot be read.
pub fn report(root: &Path, check: bool) -> Result<bool, Failure> {
    let struct_start = Regex::new(r"^\s*pub struct (\w+) \{$")?;
    let field = Regex::new(r"^\s*pub (\w+): *(.+?),?$")?;
    let impl_start = Regex::new(r"^impl Validate for (\w+)")?;
    let listed = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*")?;

    let mut declared: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut validated: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut has_impl: BTreeSet<String> = BTreeSet::new();

    for module in MODULES {
        let dir = root.join(module);
        if !dir.is_dir() {
            continue;
        }
        let mut files: Vec<_> = std::fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "rs"))
            .collect();
        files.sort();

        for file in files {
            let text = std::fs::read_to_string(&file)?;
            // Only the part before the test module: a test fixture struct is not a wire object.
            let text = text.split("#[cfg(test)]").next().unwrap_or_default().to_owned();

            let mut current: Option<String> = None;
            let mut fields: Vec<String> = Vec::new();
            let mut skip_next = false;

            let mut in_validate: Option<String> = None;
            let mut collecting = false;
            let mut buffer = String::new();

            for line in text.lines() {
                // --- struct declarations ---------------------------------------------------
                if let Some(captures) = struct_start.captures(line) {
                    if let Some(name) = current.take() {
                        declared.entry(name).or_default().extend(std::mem::take(&mut fields));
                    }
                    current = Some(format!("{module}::{}", &captures[1]));
                    continue;
                }
                if current.is_some() {
                    if line.trim() == "}" {
                        if let Some(name) = current.take() {
                            declared.entry(name).or_default().extend(std::mem::take(&mut fields));
                        }
                    } else if line.trim_start().starts_with("#[serde(")
                        || line.trim_start().starts_with("#[cfg_attr")
                    {
                        if line.contains("flatten") {
                            skip_next = true;
                        }
                    } else if let Some(captures) = field.captures(line) {
                        if std::mem::take(&mut skip_next) {
                            continue;
                        }
                        if carries_a_constraint(&captures[2]) {
                            fields.push(captures[1].to_owned());
                        }
                    }
                    continue;
                }

                // --- Validate impls ------------------------------------------------------
                if let Some(captures) = impl_start.captures(line) {
                    in_validate = Some(format!("{module}::{}", &captures[1]));
                    has_impl.insert(format!("{module}::{}", &captures[1]));
                    continue;
                }
                let Some(object) = in_validate.clone() else { continue };
                if line.starts_with('}') {
                    in_validate = None;
                    continue;
                }
                if line.contains("validate_fields!(") {
                    collecting = true;
                    buffer.clear();
                    buffer.push_str(line.split_once("validate_fields!(").expect("just matched").1);
                } else if collecting {
                    buffer.push_str(line);
                }
                if collecting && buffer.contains(MACRO_END) {
                    collecting = false;
                    let inside = buffer.split_once(MACRO_END).map_or(&buffer[..], |(a, _)| a);
                    let entry = validated.entry(object.clone()).or_default();
                    // The first two arguments are `self` and the validator; a `field as "wire"`
                    // contributes the Rust name first, and the quoted wire name is skipped
                    // because it is inside a string literal the regex never reaches.
                    for name in listed.find_iter(inside).skip(2) {
                        entry.insert(name.as_str().to_owned());
                    }
                }
                // A field validated by hand rather than through the macro: `v.field("x", &…)`.
                if let Some(rest) = line.split_once("v.field(\"") {
                    if let Some((name, _)) = rest.1.split_once('"') {
                        validated.entry(object.clone()).or_default().insert(name.to_owned());
                    }
                } else if let Some(rest) = line.split_once("v.enter(\"") {
                    if let Some((name, _)) = rest.1.split_once('"') {
                        validated.entry(object.clone()).or_default().insert(name.to_owned());
                    }
                }
            }
            if let Some(name) = current {
                declared.entry(name).or_default().extend(fields);
            }
        }
    }

    let mut clean = true;
    let mut checked = 0usize;
    let mut objects = 0usize;
    for (object, fields) in &declared {
        if !has_impl.contains(object) {
            continue; // Not a wire object: no `Validate` impl at all.
        }
        objects += 1;
        let seen = validated.get(object).cloned().unwrap_or_default();
        let missing: Vec<&String> = fields.iter().filter(|f| !seen.contains(*f)).collect();
        checked += fields.len() - missing.len();
        if !missing.is_empty() {
            clean = false;
            println!(
                "{}: {} declared, {} not reaching the validator: {}",
                object.replace("src/", ""),
                fields.len(),
                missing.len(),
                missing.iter().map(|f| f.as_str()).collect::<Vec<_>>().join(", "),
            );
        }
    }

    if clean {
        println!("{checked} field(s) across {objects} object(s); every one reaches the validator");
    } else {
        println!(
            "\nA field the validator never sees is a rule this crate promises and does not keep: \
             the object decodes, validates clean, and the peer's value is never reported. Add it \
             to the object's `validate_fields!`, or to the exemption table with a reason."
        );
    }
    Ok(clean || !check)
}
