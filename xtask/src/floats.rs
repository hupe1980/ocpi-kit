//! `no-floats`: the check behind "money is never a float".
//!
//! Every OCPI `number` — every price, tax amount, VAT percentage, energy volume and quantity —
//! is a [`Number`](../../src/types/number.rs), an exact decimal. A binary float cannot represent
//! `0.10`, and a column of them does not add up to what a human would write on an invoice.
//!
//! This scans the code that models the wire and the code that computes money for `f32` and `f64`
//! in type position. Two places legitimately mention `f64` — the JSON number boundary, where
//! `serde_json` hands over an `f64` and the crate immediately recovers the exact decimal — and
//! they are listed here by name, so adding a third is a deliberate act rather than an oversight.

use std::path::Path;

use crate::Failure;

/// The directories where a float would be a defect.
const SCANNED: &[&str] =
    &["src/types", "src/v2_1_1", "src/v2_2_1", "src/v2_3_0", "src/tariffs", "src/transport", "src/convert"];

/// Files allowed to mention a float, and why.
const JUSTIFIED: &[(&str, &str)] = &[(
    "src/types/number.rs",
    "the JSON number boundary: serde_json hands over an f64, which is immediately converted back \
     to the exact decimal it came from",
)];

/// Reports every float in the wire models and the pricing engine.
///
/// # Errors
///
/// Fails when the source tree cannot be read.
pub fn check(root: &Path) -> Result<bool, Failure> {
    let mut findings = Vec::new();
    let mut scanned = 0usize;

    for directory in SCANNED {
        for path in rust_files(&root.join(directory))? {
            let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            scanned += 1;
            if JUSTIFIED.iter().any(|(file, _)| *file == relative) {
                continue;
            }
            let text = std::fs::read_to_string(&path)?;
            for (number, line) in text.lines().enumerate() {
                if mentions_float(line) {
                    findings.push(format!("{relative}:{}: {}", number + 1, line.trim()));
                }
            }
        }
    }

    if findings.is_empty() {
        println!("{scanned} file(s) scanned; no floats in the wire models or the pricing engine");
        for (file, reason) in JUSTIFIED {
            println!("  (allowed: {file} — {reason})");
        }
        return Ok(true);
    }

    eprintln!("floats found where OCPI numbers must be exact decimals:");
    for finding in &findings {
        eprintln!("  {finding}");
    }
    eprintln!(
        "\nUse ocpi_kit::types::Number. If a float is genuinely unavoidable, add the file to \
         JUSTIFIED in xtask/src/floats.rs with the reason."
    );
    Ok(false)
}

/// Whether a line uses `f32` or `f64` as a type rather than as part of a longer word.
fn mentions_float(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or(line);
    ["f32", "f64"].iter().any(|needle| {
        code.match_indices(needle).any(|(at, _)| {
            let before = code[..at].chars().next_back();
            let after = code[at + needle.len()..].chars().next();
            let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
            boundary(before) && boundary(after)
        })
    })
}

fn rust_files(dir: &Path) -> Result<Vec<std::path::PathBuf>, Failure> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            out.extend(rust_files(&path)?);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}
