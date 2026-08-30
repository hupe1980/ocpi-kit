//! `sync-fixtures`: keep `fixtures/` in step with the vendored specification.
//!
//! OCPI 2.2.1 and later ship their examples as separate `.json` files, which are copied verbatim.
//! OCPI 2.1.1 embeds them in fenced blocks inside Markdown, so those are extracted, named after
//! the section they appear in, and written out formatted the way they appear.

use std::collections::BTreeMap;
use std::path::Path;

use crate::{Failure, RELEASES};

/// Copies and extracts every example the specification ships.
///
/// # Errors
///
/// Fails when the vendored specification cannot be read or the fixture directory cannot be
/// written.
pub fn sync(root: &Path) -> Result<bool, Failure> {
    let specs = root.join("specs/src");
    if !specs.is_dir() {
        return Err(format!(
            "{} does not exist; the vendored specification is git-ignored, so clone \
             github.com/ocpi/ocpi into specs/src/ocpi-<version> first",
            specs.display()
        )
        .into());
    }

    let mut total = 0usize;
    for (release, version) in RELEASES {
        let source = specs.join(release);
        if !source.is_dir() {
            eprintln!("skipping {release}: not vendored");
            continue;
        }
        let target = root.join("fixtures").join(version);
        std::fs::create_dir_all(&target)?;

        let copied = if source.join("examples").is_dir() {
            copy_examples(&source.join("examples"), &target)?
        } else {
            extract_from_markdown(&source, &target)?
        };
        println!("{version}: {copied} example(s)");
        total += copied;
    }
    println!("{total} example(s) in total");
    Ok(true)
}

/// Copies every `.json` file from a specification's `examples/` directory.
fn copy_examples(from: &Path, to: &Path) -> Result<usize, Failure> {
    let mut copied = 0;
    for entry in std::fs::read_dir(from)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let name = path.file_name().expect("a file has a name");
        std::fs::copy(&path, to.join(name))?;
        copied += 1;
    }
    Ok(copied)
}

/// Pulls the fenced JSON blocks out of a Markdown specification.
///
/// Each block is named after the nearest preceding heading, lower-cased and slugified, with a
/// numeric suffix when a section has several. That gives stable names across re-syncs as long as
/// the headings do not move.
fn extract_from_markdown(from: &Path, to: &Path) -> Result<usize, Failure> {
    let mut written = 0usize;
    let mut per_section: BTreeMap<String, usize> = BTreeMap::new();

    let mut files: Vec<_> = std::fs::read_dir(from)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();

    for file in files {
        let text = std::fs::read_to_string(&file)?;
        let module =
            file.file_stem().and_then(|s| s.to_str()).unwrap_or("spec").trim_start_matches("mod_").to_owned();

        let mut heading = module.clone();
        let mut lines = text.lines().peekable();
        while let Some(line) = lines.next() {
            if let Some(rest) = line.strip_prefix('#') {
                heading = slug(rest.trim_start_matches('#').trim());
                continue;
            }
            if !line.trim_start().starts_with("```json") {
                continue;
            }
            let mut body = String::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            // Only keep blocks that are actually JSON documents; the specifications also show
            // fragments and HTTP exchanges in `json` fences.
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else { continue };

            let key = format!("{module}_{heading}");
            let index = per_section.entry(key.clone()).or_insert(0);
            let name = if *index == 0 { format!("{key}.json") } else { format!("{key}_{index}.json") };
            *index += 1;

            std::fs::write(to.join(name), format!("{}\n", serde_json::to_string_pretty(&value)?))?;
            written += 1;
        }
    }
    Ok(written)
}

/// Turns a heading into a stable file-name fragment.
fn slug(heading: &str) -> String {
    let cleaned: String = heading
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    // Drop a leading section number such as "3_1_".
    let without_number = trimmed
        .split('_')
        .skip_while(|part| part.is_empty() || part.chars().all(|c| c.is_ascii_digit()))
        .collect::<Vec<_>>()
        .join("_");
    let result = if without_number.is_empty() { trimmed } else { &without_number };
    result.split('_').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("_")
}
