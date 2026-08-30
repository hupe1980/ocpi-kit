//! `dead-config`: the check behind "every setting does something".
//!
//! A configuration field that does nothing is worse than a missing feature: somebody reads the doc
//! comment, sets the flag, believes the problem is handled, and ships. Nothing fails, and the
//! discovery happens in production.
//!
//! So every public field of the structs below must be **read** somewhere. Being written is not
//! enough, and that distinction is the whole check — a dead flag always has a `Default` that sets
//! it and often a builder method too. A read is an occurrence of `.field` that is not the target of
//! an assignment; `field: true` in a struct literal is not one. Reading it inside the type's own
//! methods counts: `UrlPolicy::allowed_schemes` is consulted only by `UrlPolicy::check`, and that
//! is a field doing its job.
//!
//! A read is matched by *name*, because this tool has no type information, so two of these structs
//! sharing a field name would make it blind. That is a failure in its own right: rename one for the
//! side it describes.

use std::path::Path;

use crate::Failure;

/// The configuration structs whose fields must all be live, and the file each is declared in.
///
/// These are the types a user reaches for to change behaviour. A field on one of them is a
/// promise.
const CONFIGS: &[(&str, &str)] = &[
    ("Quirks", "src/transport/quirks.rs"),
    ("ClientConfig", "src/client/mod.rs"),
    ("RetryPolicy", "src/client/mod.rs"),
    ("ServerConfig", "src/server/router.rs"),
    ("PricingPolicy", "src/tariffs/policy.rs"),
    ("UrlPolicy", "src/types/url.rs"),
];

/// Reports every public configuration field that nothing reads.
///
/// # Errors
///
/// Fails when the source tree cannot be read, or when a struct named in [`CONFIGS`] is not found
/// in the file it claims to be in — which means this check has silently stopped checking.
pub fn check(root: &Path) -> Result<bool, Failure> {
    let sources = rust_sources(&root.join("src"))?;
    let mut findings = Vec::new();
    let mut checked = 0usize;
    let mut declared: Vec<(&str, &str, String)> = Vec::new();

    for (type_name, declared_in) in CONFIGS {
        let declaring_path = root.join(declared_in);
        let text = std::fs::read_to_string(&declaring_path).map_err(|e| format!("{declared_in}: {e}"))?;
        let fields = public_fields(&text, type_name)
            .ok_or_else(|| format!("no `pub struct {type_name}` in {declared_in}"))?;
        if fields.is_empty() {
            return Err(format!("`{type_name}` in {declared_in} has no public fields to check").into());
        }
        for field in fields {
            declared.push((type_name, declared_in, field));
        }
    }

    // Matched by name, so a collision between two of these structs would make the check blind.
    for (i, (type_a, _, field)) in declared.iter().enumerate() {
        if let Some((type_b, _, _)) = declared[i + 1..].iter().find(|(_, _, other)| other == field) {
            return Err(format!(
                "`{type_a}::{field}` and `{type_b}::{field}` share a field name, so a read of \
                 `.{field}` cannot be attributed to either one and a dead field on one of them \
                 would pass unnoticed. Rename one for the side it describes."
            )
            .into());
        }
    }

    for (type_name, declared_in, field) in &declared {
        checked += 1;
        if !sources.iter().any(|(_, body)| is_read(body, field)) {
            findings.push(format!(
                "  {type_name}::{field}\n      declared in {declared_in}, only ever assigned — \
                 either honour it or delete it"
            ));
        }
    }

    if findings.is_empty() {
        println!(
            "{checked} configuration field(s) across {} struct(s); every one is read somewhere",
            CONFIGS.len()
        );
        return Ok(true);
    }
    eprintln!("configuration that does nothing:\n{}", findings.join("\n"));
    eprintln!(
        "\nA setting nobody reads is a promise the crate does not keep. If the behaviour is \
         unconditional, say so in the module documentation and remove the field."
    );
    Ok(false)
}

/// Whether `text` reads `field` anywhere, rather than only assigning it.
///
/// `self.timeout` is a read; `timeout: Duration::from_secs(30)` in a `Default` impl is not, and
/// neither is `self.timeout = value` in a builder.
fn is_read(text: &str, field: &str) -> bool {
    let needle = format!(".{field}");
    text.match_indices(&needle).any(|(at, _)| {
        // The next character must not continue an identifier, or `.max_page` matches
        // `.max_page_limit`.
        let after = text[at + needle.len()..].chars().next();
        if after.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            return false;
        }
        // An assignment target is a write. `==` and `+=` are reads.
        let rest = text[at + needle.len()..].trim_start();
        !(rest.starts_with('=') && !rest.starts_with("=="))
    })
}

/// The names of the public fields of `pub struct <type_name>` in `text`.
///
/// A deliberately small parser: these structs are plain field lists, and pulling in `syn` to read
/// six of them would cost more than it explains.
fn public_fields(text: &str, type_name: &str) -> Option<Vec<String>> {
    let start = text.find(&format!("pub struct {type_name} {{"))?;
    let body = &text[start..];
    let end = body.find("\n}")?;
    let mut fields = Vec::new();
    for line in body[..end].lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub ") else { continue };
        let Some((name, _)) = rest.split_once(':') else { continue };
        if name.chars().all(|c| c.is_ascii_lowercase() || c == '_') && !name.is_empty() {
            fields.push(name.to_owned());
        }
    }
    Some(fields)
}

/// Every `.rs` file under `directory`, as (repo-relative path, contents).
fn rust_sources(directory: &Path) -> Result<Vec<(String, String)>, Failure> {
    let root = directory.parent().unwrap_or(directory).to_path_buf();
    let mut out = Vec::new();
    let mut stack = vec![directory.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let relative = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                out.push((relative, std::fs::read_to_string(&path)?));
            }
        }
    }
    Ok(out)
}
