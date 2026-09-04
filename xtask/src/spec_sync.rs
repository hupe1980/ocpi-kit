//! `spec-sync`: the vendored specification, pinned.
//!
//! The three censuses ([`coverage`](crate::coverage), [`enums`](crate::enums),
//! [`shapes`](crate::shapes)) are this crate's strongest claim to conformance, and they are only
//! worth as much as the documents they read. Those documents are **not** vendored — OCPI is
//! published under CC BY-ND 4.0 — so what sits in `specs/src/` is whatever the person running the
//! check happened to clone, whenever they happened to clone it.
//!
//! That is a hole, and upstream is exactly the kind of place it matters: the OCPI repository
//! edits released branches in place, and in July 2026 it restructured 2.3.0 entirely — moving the
//! Payments module and `Tariff.preauthorize_amount` out of core onto a release branch, and moving
//! Invoice Reconciliation in. A crate written against the older layout compiles, passes its tests,
//! and is wrong about which release defines what.
//!
//! So this task pins the sources three ways:
//!
//! * **`spec-sources.toml`** names each release, its branch and its **commit**. `--fetch`
//!   materialises exactly that commit, which is what makes a CI run reproducible and what lets CI
//!   run the censuses *for real* rather than skipping them.
//! * **`spec-sources.lock`** records a SHA-256 for every file a census reads. `--check` compares
//!   the working checkout against it and names each file that was added, removed or changed —
//!   which catches a hand-edited or half-updated checkout that a commit id alone would not.
//! * **`--latest`** fetches the branch head and reports how it differs from the pin, for a
//!   scheduled job whose whole purpose is to notice a change somebody else made upstream.
//!
//! ```text
//! cargo run -p xtask -- spec-sync              # report drift against the lock
//! cargo run -p xtask -- spec-sync --check      # …and fail on it (CI)
//! cargo run -p xtask -- spec-sync --fetch      # clone/checkout every pinned commit
//! cargo run -p xtask -- spec-sync --update     # re-record the lock from the checkout
//! cargo run -p xtask -- spec-sync --latest     # what has upstream changed since the pin?
//! ```

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::Failure;

/// One pinned specification release.
#[derive(Debug, Clone)]
pub struct Release {
    /// Directory under `specs/src/`.
    pub name: String,
    /// What it is, for a person reading the report.
    pub label: String,
    /// Git remote to clone from.
    pub repo: String,
    /// The branch the pin lives on.
    pub branch: String,
    /// The commit the crate is written against.
    pub commit: String,
}

/// The manifest file, relative to the repository root.
const MANIFEST: &str = "spec-sources.toml";
/// The per-file digest lock, relative to the repository root.
const LOCK: &str = "spec-sources.lock";

/// What `spec-sync` was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Report drift against the lock.
    Report,
    /// Report drift and fail on it.
    Check,
    /// Clone or check out every pinned commit into `specs/src/`.
    Fetch,
    /// Rewrite the lock from the working checkout.
    Update,
    /// Compare each pin with its branch head upstream.
    Latest,
}

impl Mode {
    /// Reads the mode from the command line.
    #[must_use]
    pub fn from_args<I: Iterator<Item = String>>(args: I) -> Self {
        let mut mode = Self::Report;
        for arg in args {
            mode = match arg.as_str() {
                "--check" => Self::Check,
                "--fetch" => Self::Fetch,
                "--update" => Self::Update,
                "--latest" => Self::Latest,
                _ => continue,
            };
        }
        mode
    }
}

/// Runs `spec-sync`.
///
/// # Errors
///
/// Fails when the manifest cannot be read, when `git` is not available for `--fetch`/`--latest`,
/// or when the lock cannot be written.
pub fn run(root: &Path, mode: Mode) -> Result<bool, Failure> {
    let releases = read_manifest(&root.join(MANIFEST))?;
    match mode {
        Mode::Fetch => fetch(root, &releases),
        Mode::Update => update(root, &releases),
        Mode::Latest => latest(root, &releases),
        Mode::Report | Mode::Check => check(root, &releases, mode == Mode::Check),
    }
}

/// Compares the checkout with the lock.
fn check(root: &Path, releases: &[Release], fail_on_drift: bool) -> Result<bool, Failure> {
    let lock = read_lock(&root.join(LOCK))?;
    if lock.is_empty() {
        return Err(format!(
            "{LOCK} is empty or missing; run `cargo run -p xtask -- spec-sync --update` against a \
             checkout you trust"
        )
        .into());
    }

    let mut clean = true;
    let mut checked = 0usize;
    for release in releases {
        let dir = root.join("specs/src").join(&release.name);
        if !dir.is_dir() {
            println!("{}: not vendored (run --fetch)", release.name);
            clean = false;
            continue;
        }
        let now = digest_tree(&dir)?;
        let then: BTreeMap<&String, &String> = lock
            .iter()
            .filter(|(path, _)| path.starts_with(&format!("{}/", release.name)))
            .map(|(path, digest)| (path, digest))
            .collect();

        let mut drift = Vec::new();
        for (path, digest) in &now {
            let key = format!("{}/{path}", release.name);
            match then.get(&key) {
                None => drift.push(format!("  + {path} (not in the lock)")),
                Some(recorded) if *recorded != digest => drift.push(format!("  ~ {path} (changed)")),
                Some(_) => {}
            }
        }
        for key in then.keys() {
            let path = key.trim_start_matches(&format!("{}/", release.name));
            if !now.contains_key(path) {
                drift.push(format!("  - {path} (gone)"));
            }
        }
        checked += now.len();

        if drift.is_empty() {
            println!(
                "{} ({}) @ {} — {} file(s), all as pinned",
                release.name,
                release.label,
                short(&release.commit),
                now.len()
            );
        } else {
            clean = false;
            drift.sort();
            println!(
                "{} @ {} — {} file(s) differ from the pin:",
                release.name,
                short(&release.commit),
                drift.len()
            );
            for line in &drift {
                println!("{line}");
            }
        }
    }

    if clean {
        println!("\n{checked} specification file(s) match {LOCK} exactly");
    } else {
        println!(
            "\nthe vendored specification is not what this crate was written against. Either \
             `--fetch` the pinned commits, or review the differences and `--update` the lock \
             together with whatever they change in the crate"
        );
    }
    Ok(clean || !fail_on_drift)
}

/// Clones or checks out each pinned commit.
fn fetch(root: &Path, releases: &[Release]) -> Result<bool, Failure> {
    let specs = root.join("specs/src");
    std::fs::create_dir_all(&specs)?;
    for release in releases {
        let dir = specs.join(&release.name);
        println!("{} <- {} @ {}", release.name, release.branch, short(&release.commit));
        if dir.join(".git").is_dir() {
            git(&dir, &["fetch", "--quiet", "origin", &release.branch])?;
        } else {
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            git(
                &specs,
                &["clone", "--quiet", "--no-checkout", "--filter=blob:none", &release.repo, &release.name],
            )?;
        }
        git(&dir, &["checkout", "--quiet", "--force", &release.commit])?;
    }
    println!("\n{} release(s) at their pinned commits", releases.len());
    Ok(true)
}

/// Rewrites the lock from the working checkout.
fn update(root: &Path, releases: &[Release]) -> Result<bool, Failure> {
    let mut lock: BTreeMap<String, String> = BTreeMap::new();
    for release in releases {
        let dir = root.join("specs/src").join(&release.name);
        if !dir.is_dir() {
            return Err(format!("{} is not vendored; run --fetch first", release.name).into());
        }
        for (path, digest) in digest_tree(&dir)? {
            lock.insert(format!("{}/{path}", release.name), digest);
        }
    }

    let mut out = String::new();
    out.push_str(
        "# SHA-256 of every specification file the censuses read, per release.\n\
         #\n\
         # Written by `cargo run -p xtask -- spec-sync --update`. A change here is a change in\n\
         # what this crate is written against: review it together with whatever it changes in the\n\
         # crate, never on its own.\n",
    );
    for (path, digest) in &lock {
        out.push_str(&format!("{digest}  {path}\n"));
    }
    std::fs::write(root.join(LOCK), out)?;
    println!("{} file(s) recorded in {LOCK}", lock.len());
    Ok(true)
}

/// Reports how each pin differs from its branch head upstream.
fn latest(root: &Path, releases: &[Release]) -> Result<bool, Failure> {
    let mut moved = false;
    for release in releases {
        let dir = root.join("specs/src").join(&release.name);
        if !dir.join(".git").is_dir() {
            println!("{}: not a git checkout; run --fetch first", release.name);
            continue;
        }
        git(&dir, &["fetch", "--quiet", "origin", &release.branch])?;
        let head = capture(&dir, &["rev-parse", "FETCH_HEAD"])?;
        let head = head.trim();
        if head == release.commit {
            println!("{} @ {} — up to date with {}", release.name, short(head), release.branch);
            continue;
        }
        moved = true;
        let names = capture(&dir, &["diff", "--name-only", &release.commit, head])?;
        let count = names.lines().filter(|l| !l.trim().is_empty()).count();
        println!(
            "{}: {} moved from {} to {} — {} file(s) changed:",
            release.name,
            release.branch,
            short(&release.commit),
            short(head),
            count
        );
        for line in names.lines().take(40) {
            println!("  {line}");
        }
    }
    if moved {
        println!(
            "\nupstream has edited a released branch. Read the diff, decide what it means for the \
             crate, then move the pin in {MANIFEST} and re-run --fetch --update."
        );
    }
    // Not a failure: a moved pin is news, and the caller decides what to do about it.
    Ok(true)
}

/// Every file a census reads, with its digest, keyed by path relative to the release directory.
fn digest_tree(dir: &Path) -> Result<BTreeMap<String, String>, Failure> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
            if name == ".git" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let interesting =
                matches!(path.extension().and_then(|e| e.to_str()), Some("asciidoc" | "md" | "json"));
            if !interesting {
                continue;
            }
            let relative = path.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
            out.insert(relative, sha256_hex(&std::fs::read(&path)?));
        }
    }
    Ok(out)
}

/// Parses the manifest.
///
/// A deliberately small reader for a deliberately small file: five tables of five string fields.
/// Pulling in a TOML parser for that would add a dependency to the one crate in this repository
/// whose dependencies nobody audits.
fn read_manifest(path: &Path) -> Result<Vec<Release>, Failure> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut releases = Vec::new();
    let mut current: BTreeMap<String, String> = BTreeMap::new();
    let mut in_release = false;

    let finish = |fields: &mut BTreeMap<String, String>, out: &mut Vec<Release>| -> Result<(), Failure> {
        if fields.is_empty() {
            return Ok(());
        }
        let take = |key: &str, fields: &BTreeMap<String, String>| -> Result<String, Failure> {
            fields.get(key).cloned().ok_or_else(|| format!("a [[release]] has no `{key}`").into())
        };
        out.push(Release {
            name: take("name", fields)?,
            label: take("label", fields)?,
            repo: take("repo", fields)?,
            branch: take("branch", fields)?,
            commit: take("commit", fields)?,
        });
        fields.clear();
        Ok(())
    };

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[release]]" {
            finish(&mut current, &mut releases)?;
            in_release = true;
            continue;
        }
        if !in_release {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"').to_owned();
        current.insert(key.trim().to_owned(), value);
    }
    finish(&mut current, &mut releases)?;

    if releases.is_empty() {
        return Err(format!("{} lists no [[release]]", path.display()).into());
    }
    Ok(releases)
}

/// Reads the digest lock.
fn read_lock(path: &Path) -> Result<BTreeMap<String, String>, Failure> {
    let Ok(text) = std::fs::read_to_string(path) else { return Ok(BTreeMap::new()) };
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((digest, file)) = line.split_once("  ") {
            out.insert(file.trim().to_owned(), digest.trim().to_owned());
        }
    }
    Ok(out)
}

fn git(dir: &Path, args: &[&str]) -> Result<(), Failure> {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if status.success() {
        return Ok(());
    }
    Err(format!("git {} failed in {}", args.join(" "), dir.display()).into())
}

fn capture(dir: &Path, args: &[&str]) -> Result<String, Failure> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(10)]
}

/// SHA-256, in about forty lines.
///
/// The alternative is a dependency in the one crate in this repository whose dependency tree
/// nobody audits, for a hash whose specification fits on a page.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    let mut message = data.to_vec();
    let bits = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());

    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }

    h.iter().map(|word| format!("{word:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_published_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // Longer than one block, to exercise the message schedule across chunks.
        assert_eq!(
            sha256_hex(&b"a".repeat(1000)),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn the_manifest_reader_reads_a_release() {
        let dir = std::env::temp_dir().join("ocpi-kit-spec-sync-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("manifest.toml");
        std::fs::write(
            &path,
            "# a comment\n\n[[release]]\nname = \"ocpi-2.3.0\"\nlabel = \"core\"\n\
             repo = \"https://example.com/ocpi.git\"\nbranch = \"main\"\ncommit = \"abc123\"\n",
        )
        .expect("write");
        let releases = read_manifest(&path).expect("parses");
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].name, "ocpi-2.3.0");
        assert_eq!(releases[0].commit, "abc123");
    }
}
