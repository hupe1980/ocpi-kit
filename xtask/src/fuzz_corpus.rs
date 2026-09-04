//! `fuzz-corpus`: seed the fuzz targets from the specification's own examples.
//!
//! A coverage-guided fuzzer spends its first hours discovering that OCPI documents are JSON, that
//! they have a `country_code`, that `last_updated` is an RFC 3339 timestamp. It does not have to:
//! the specification ships 279 examples, and `fixtures/` already holds every one of them. Seeded
//! with those, the fuzzer starts from documents that decode and spends its budget on the parts
//! that are hard — a `step_size` that overflows a block count, a patch that removes a required
//! field, a `Price` that crosses a version boundary.
//!
//! Two targets read *two* documents from one input, splitting it down the middle, so their seeds
//! are pairs: a CDR followed by a Tariff, a Location followed by a patch. Building those pairs is
//! the reason this is a task rather than a `cp`.
//!
//! ```text
//! cargo run -p xtask -- fuzz-corpus
//! cd fuzz && cargo +nightly fuzz run pricing -- -max_total_time=300
//! ```

use std::path::Path;

use crate::Failure;

/// Writes a seed corpus for every fuzz target.
///
/// # Errors
///
/// Fails when `fixtures/` cannot be read or the corpus cannot be written.
pub fn seed(root: &Path) -> Result<bool, Failure> {
    let fixtures = root.join("fixtures");
    if !fixtures.is_dir() {
        return Err(format!("{} does not exist", fixtures.display()).into());
    }
    let corpus = root.join("fuzz/corpus");

    let all = collect(&fixtures, |_| true)?;
    let cdrs = collect(&fixtures, |name| name.starts_with("cdr") || name.contains("_cdr"))?;
    let tariffs = collect(&fixtures, |name| name.starts_with("tariff"))?;
    let patches = collect(&fixtures, |name| name.contains("patch"))?;
    let locations = collect(&fixtures, |name| name.starts_with("location_example"))?;

    let mut written = 0;
    written += write_each(&corpus.join("wire"), &all)?;
    written += write_each(&corpus.join("envelope"), &envelopes(&all))?;
    written += write_each(&corpus.join("bridge"), &all)?;
    written += write_each(&corpus.join("patch"), &pairs(&locations, &patches))?;
    written += write_each(&corpus.join("pricing"), &pairs(&cdrs, &tariffs))?;
    written += write_each(&corpus.join("headers"), &headers())?;

    println!("{written} seed(s) written under {}", corpus.display());
    println!(
        "run one with:  cd fuzz && cargo +nightly fuzz run <target> -- -max_total_time=300\n\
         targets: wire, envelope, bridge, patch, pricing, headers"
    );
    Ok(true)
}

/// Every fixture whose file name satisfies `wanted`, as bytes.
fn collect(fixtures: &Path, wanted: impl Fn(&str) -> bool) -> Result<Vec<(String, Vec<u8>)>, Failure> {
    let mut out = Vec::new();
    for release in std::fs::read_dir(fixtures)? {
        let release = release?.path();
        if !release.is_dir() {
            continue;
        }
        let label = release.file_name().and_then(|n| n.to_str()).unwrap_or("x").replace('.', "_");
        for entry in std::fs::read_dir(&release)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
            if !wanted(&name) {
                continue;
            }
            out.push((format!("{label}_{name}"), std::fs::read(&path)?));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The pairwise seeds for the targets that read two documents from one input.
///
/// The target splits its input in half, so each seed is two documents padded to the same length —
/// otherwise the split lands inside the first document and the second is never seen.
fn pairs(first: &[(String, Vec<u8>)], second: &[(String, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for (a_name, a) in first.iter().take(12) {
        for (b_name, b) in second.iter().take(12) {
            let width = a.len().max(b.len());
            let mut bytes = Vec::with_capacity(width * 2);
            bytes.extend_from_slice(a);
            bytes.resize(width, b' ');
            bytes.extend_from_slice(b);
            bytes.resize(width * 2, b' ');
            out.push((format!("{a_name}__{b_name}"), bytes));
        }
    }
    out
}

/// Each fixture wrapped in the response envelope a client actually reads.
fn envelopes(documents: &[(String, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for (name, body) in documents.iter().take(60) {
        let Ok(text) = std::str::from_utf8(body) else { continue };
        out.push((
            format!("envelope_{name}"),
            format!(
                "{{\"data\":{text},\"status_code\":1000,\"status_message\":\"Success\",\
                 \"timestamp\":\"2024-03-01T10:00:00Z\"}}"
            )
            .into_bytes(),
        ));
    }
    out.push((
        "error_2001".to_owned(),
        br#"{"status_code":2001,"status_message":"Invalid parameters","timestamp":"2024-03-01T10:00:00Z"}"#
            .to_vec(),
    ));
    out
}

/// The header values a peer sends, as the fuzzer's starting vocabulary.
fn headers() -> Vec<(String, Vec<u8>)> {
    [
        ("token_encoded", "Token ZXhhbXBsZS10b2tlbg=="),
        ("token_unencoded", "Token example-token"),
        ("token_empty", "Token "),
        ("link_next", "<https://example.com/ocpi/cpo/2.3.0/locations?offset=50&limit=50>; rel=\"next\""),
        ("link_two", "<https://a.example/x>; rel=\"prev\", <https://b.example/y>; rel=\"next\""),
        ("page_query", "offset=50&limit=100&date_from=2024-03-01T10:00:00Z&date_to=2024-03-02T00:00:00Z"),
        ("count", "12345"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.as_bytes().to_vec()))
    .collect()
}

fn write_each(dir: &Path, seeds: &[(String, Vec<u8>)]) -> Result<usize, Failure> {
    std::fs::create_dir_all(dir)?;
    for (name, bytes) in seeds {
        std::fs::write(dir.join(name), bytes)?;
    }
    Ok(seeds.len())
}
