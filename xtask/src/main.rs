//! Repository automation for `ocpi-kit`.
//!
//! ```text
//! cargo run -p xtask -- sync-fixtures     # copy the spec's examples into fixtures/
//! cargo run -p xtask -- spec-coverage     # compare the crate's fields with the spec's tables
//! ```
//!
//! Both read the vendored specification under `specs/`, which is not shipped with the crate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod coverage;
mod fixtures;
mod floats;

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let root = repo_root();
    let result = match task.as_deref() {
        Some("sync-fixtures") => fixtures::sync(&root),
        Some("spec-coverage") => coverage::report(&root, std::env::args().any(|a| a == "--check")),
        Some("no-floats") => floats::check(&root),
        Some(other) => {
            Err(format!("unknown task {other:?}; try sync-fixtures, spec-coverage or no-floats").into())
        }
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

/// The specification releases this repository vendors, and the fixture directory each feeds.
pub const RELEASES: &[(&str, &str)] = &[
    ("ocpi-2.3.0", "2.3.0"),
    ("ocpi-2.2.1", "2.2.1"),
    ("ocpi-2.3.0-bookings", "2.3.0-bookings"),
    ("ocpi-2.1.1", "2.1.1"),
];

/// Collects the names in `set` into a stable, printable list.
pub fn join(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(", ")
}

/// A table of object name to field names, in document order.
pub type Fields = BTreeMap<String, Vec<String>>;
