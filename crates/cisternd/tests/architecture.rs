//! The clauses in `docs/architecture.md`, checked against the source.
//!
//! Only what a reference makes visible is checked here.
//! Whether a piece of code is an adapter is a judgement.
//! This sees only what that judgement left in the references.
//! A re-export would slip past it.
//! It is here to catch drift, not to stop someone who means to get around it.

// A test panics to signal failure.
#![allow(clippy::unwrap_used)]

use std::{
    fs,
    path::{Path, PathBuf},
};

/// A directory, and what no file under it may name.
///
/// Adding a clause is a line here and a sentence in `docs/architecture.md`.
const INWARD: &[(&str, &[&str])] = &[
    // The domain is the innermost thing there is.
    ("crates/cisternd/src/core/domain", &["core::port"]),
    // Nothing in the core names what is outside it.
    (
        "crates/cisternd/src/core",
        &["crate::adapter", "crate::platform", "cistern_contract"],
    ),
    // An inbound adapter does not know which stores the core keeps.
    ("crates/cisternd/src/adapter/inbound", &["port::outbound"]),
    // An outbound adapter does not know what the core offers.
    ("crates/cisternd/src/adapter/outbound", &["port::inbound"]),
    // One adapter does not reach another.
    // Only the composition root joins them.
    ("crates/cisternd/src/adapter", &["crate::adapter"]),
    // What touches no port does not reach one.
    ("crates/cisternd/src/platform", &["core::port"]),
];

#[test]
fn references_point_inward() {
    let mut broken = Vec::new();

    for (dir, forbidden) in INWARD {
        for file in rust_files(&workspace().join(dir)) {
            let text = fs::read_to_string(&file).unwrap();
            for name in *forbidden {
                if text.contains(name) {
                    broken.push(format!("{} names {name}", under_workspace(&file)));
                }
            }
        }
    }

    assert!(
        broken.is_empty(),
        "docs/architecture.md says references point inward:\n{}",
        broken.join("\n")
    );
}

/// A name that has to be reached through an alias is the wrong name.
#[test]
fn nothing_is_reached_through_an_alias() {
    let mut aliased = Vec::new();

    for file in rust_files(&workspace().join("crates")) {
        let text = fs::read_to_string(&file).unwrap();
        for line in text.lines() {
            let line = line.trim();
            if !line.starts_with("use ") && !line.starts_with("pub use ") {
                continue;
            }
            if renames_a_type(line) {
                aliased.push(format!("{}: {line}", under_workspace(&file)));
            }
        }
    }

    assert!(
        aliased.is_empty(),
        "docs/architecture.md says an aliased name is the wrong name:\n{}",
        aliased.join("\n")
    );
}

/// What a vendor calls a field is the vendor's, and it belongs in a definition rather than
/// in code. A vendor renaming one is then a change to a file.
///
/// The names are the ones Claude Code answers with. Another vendor spells them
/// differently, which is the point: none of them may be written in Rust.
const VENDOR_FIELDS: &[&str] = &[
    "input_tokens",
    "output_tokens",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
    "total_cost_usd",
];

#[test]
fn no_vendor_field_name_is_written_in_rust() {
    let mut named = Vec::new();

    for file in rust_files(&workspace().join("crates/cisternd/src")) {
        let text = fs::read_to_string(&file).unwrap();
        let text = written_code(&text);
        for field in VENDOR_FIELDS {
            if text.contains(field) {
                named.push(format!("{} names {field}", under_workspace(&file)));
            }
        }
    }

    assert!(
        named.is_empty(),
        "docs/architecture.md says a vendor's words live in a definition, not in Rust:\n{}",
        named.join("\n")
    );
}

/// The socket library reaches one crate, which is why the framing is written once rather than at each end.
#[test]
fn the_socket_library_stays_in_the_contract() {
    for crate_name in ["cistern", "cisternd"] {
        let manifest = workspace()
            .join("crates")
            .join(crate_name)
            .join("Cargo.toml");
        let text = fs::read_to_string(&manifest).unwrap();
        assert!(
            !text.contains("interprocess"),
            "{crate_name} depends on interprocess; the exchange belongs to cistern-contract"
        );
    }
}

/// A file up to where its tests begin.
///
/// A test may write out a vendor's answer as the vendor sends it, and one that could not
/// would be testing something other than what arrives. The clause is about the code that
/// runs, so the tests are left out of it.
fn written_code(text: &str) -> &str {
    match text.find("#[cfg(test)]") {
        Some(at) => &text[..at],
        None => text,
    }
}

/// ` as ` followed by a type name.
/// A lowercase one renames a module or a function, which is not what the clause is about.
fn renames_a_type(line: &str) -> bool {
    line.split(" as ").skip(1).any(|rest| {
        rest.trim_start()
            .chars()
            .next()
            .is_some_and(char::is_uppercase)
    })
}

/// The workspace root, from the crate this test belongs to.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap()
}

fn under_workspace(file: &Path) -> String {
    file.strip_prefix(workspace())
        .unwrap_or(file)
        .display()
        .to_string()
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut left = vec![dir.to_path_buf()];

    while let Some(at) = left.pop() {
        for entry in fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                left.push(path);
            } else if path.extension().is_some_and(|end| end == "rs") {
                found.push(path);
            }
        }
    }
    found
}
