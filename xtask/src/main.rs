//! Repository automation for `ocpi-kit`.
//!
//! ```text
//! cargo run -p xtask -- spec-sync        # is the vendored spec the one we were written against?
//! cargo run -p xtask -- sync-fixtures     # copy the spec's examples into fixtures/
//! cargo run -p xtask -- spec-coverage     # compare the crate's fields with the spec's tables
//! cargo run -p xtask -- enum-coverage     # compare the crate's enum values with the spec's
//! cargo run -p xtask -- field-shapes      # cardinality and length, against the same tables
//! cargo run -p xtask -- endpoints        # the URLs we build vs the ones the spec writes
//! cargo run -p xtask -- errata           # every recorded spec defect is still real
//! cargo run -p xtask -- no-floats         # money is never a float
//! cargo run -p xtask -- dead-config       # every setting does something
//! ```
//!
//! Every task but `no-floats` and `dead-config` reads the vendored specification under
//! `specs/`, which is not shipped with the crate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod coverage;
mod dead_config;
mod endpoints;
mod enums;
mod errata;
mod fixtures;
mod floats;
mod fuzz_corpus;
mod shapes;
mod spec_sync;
mod validate_coverage;

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let root = repo_root();
    let result = match task.as_deref() {
        Some("sync-fixtures") => fixtures::sync(&root),
        Some("spec-coverage") => coverage::report(&root, std::env::args().any(|a| a == "--check")),
        Some("enum-coverage") => enums::report(&root, std::env::args().any(|a| a == "--check")),
        Some("field-shapes") => shapes::report(&root, std::env::args().any(|a| a == "--check")),
        Some("fuzz-corpus") => fuzz_corpus::seed(&root),
        Some("endpoints") => endpoints::report(&root, std::env::args().any(|a| a == "--check")),
        Some("validate-coverage") => {
            validate_coverage::report(&root, std::env::args().any(|a| a == "--check"))
        }
        Some("errata") => errata::report(&root, std::env::args().any(|a| a == "--check")),
        Some("no-floats") => floats::check(&root),
        Some("dead-config") => dead_config::check(&root),
        Some("spec-sync") => spec_sync::run(&root, spec_sync::Mode::from_args(std::env::args())),
        Some(other) => Err(format!(
            "unknown task {other:?}; try sync-fixtures, spec-sync, spec-coverage, \
                 enum-coverage, field-shapes, endpoints, errata, fuzz-corpus, no-floats or \
                 dead-config"
        )
        .into()),
        None => {
            eprintln!("{}", include_str!("usage.txt"));
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Anything a task can fail with.
pub type Failure = Box<dyn std::error::Error>;

/// The repository root, which is the parent of this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the repository root")
        .to_path_buf()
}

/// The releases whose examples become fixtures, and the fixture directory each feeds.
///
/// The `payments` branch is censused but not synced: its `examples/` is byte-identical to core
/// 2.3.0's, so copying it would only duplicate `fixtures/2.3.0`. Each census carries its own
/// release table.
pub const RELEASES: &[(&str, &str)] = &[
    ("ocpi-2.3.0", "2.3.0"),
    ("ocpi-2.2.1", "2.2.1"),
    ("ocpi-2.3.0-bookings", "2.3.0-bookings"),
    ("ocpi-2.3.0-payments", "2.3.0-payments"),
    ("ocpi-2.1.1", "2.1.1"),
];

/// Collects the names in `set` into a stable, printable list.
pub fn join(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(", ")
}

/// A table of object name to field names, in document order.
pub type Fields = BTreeMap<String, Vec<String>>;
