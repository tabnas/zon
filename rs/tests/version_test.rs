// The Rust crate's version is one of the release sites that must agree.
// A bump that updates ts/package.json and forgets these fails here rather
// than shipping a crate whose version disagrees with the package it is a
// port of. Mirrors ts/test/version.test.ts and go/version_test.go: it
// fails, never skips, when ts/package.json cannot be read.

use std::fs;
use std::path::Path;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rs/ has a parent")
}

#[test]
fn version_looks_like_a_semver() {
    let parts: Vec<&str> = tabnas_zon::VERSION.split('.').collect();
    assert_eq!(
        parts.len(),
        3,
        "VERSION is not x.y.z: {}",
        tabnas_zon::VERSION
    );
    for part in parts {
        assert!(
            !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()),
            "VERSION segment is not numeric: {}",
            tabnas_zon::VERSION
        );
    }
}

#[test]
fn version_matches_cargo_toml() {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("the manifest is readable");
    let declared = manifest
        .lines()
        .find_map(|line| line.strip_prefix("version = \""))
        .and_then(|rest| rest.split('"').next())
        .expect("the manifest declares a version");
    assert_eq!(
        declared,
        tabnas_zon::VERSION,
        "Cargo.toml disagrees with VERSION"
    );
}

#[test]
fn version_matches_package_json() {
    let package = fs::read_to_string(repo_root().join("ts").join("package.json"))
        .expect("ts/package.json is readable");
    let declared: serde_json::Value =
        serde_json::from_str(&package).expect("ts/package.json is JSON");
    assert_eq!(
        declared["version"]
            .as_str()
            .expect("package.json has a version"),
        tabnas_zon::VERSION,
        "ts/package.json disagrees with VERSION"
    );
}
