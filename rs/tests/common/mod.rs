// Shared test helpers. Cargo compiles this module into EVERY integration
// test binary, so an item only one binary uses is dead code in the
// others; the allow keeps that from being a warning rather than hiding
// anything real.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use tabnas_support::{find_spec_dir, Failure, Value};
use tabnas_zon::ZonOptions;

/// The repository root: the parent of `rs/`.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rs/ has a parent")
        .to_path_buf()
}

/// The shared `test/spec` directory, found by walking up from the crate
/// rather than by counting `..` hops.
pub fn spec_dir() -> PathBuf {
    find_spec_dir(Some(Path::new(env!("CARGO_MANIFEST_DIR"))))
        .expect("a test/spec directory above rs/")
}

/// An engine value as the fixture data model, through JSON: the
/// `jsonFlatten` of the Go runner. `Undefined` and non-finite numbers
/// become `null`, and the fixtures pin neither (see test/AGENTS.md).
pub fn to_value(value: &tabnas::Value) -> Value {
    Value::from(value.to_json())
}

/// A parse error as the runner's failure: the code the fixture pins, and
/// the rendered report for the failure message.
pub fn to_failure(error: tabnas::TabnasError) -> Failure {
    Failure::new(error.code.clone())
        .at(error.row, error.col)
        .with_message(error.to_string())
}

/// The plugin options a fixture row's `opts` cell names: an empty cell
/// is the defaults, anything else is a JSON object of them.
pub fn row_options(raw: &str) -> Result<ZonOptions, Failure> {
    if raw.trim().is_empty() {
        return Ok(ZonOptions::default());
    }
    serde_json::from_str(raw)
        .map_err(|error| Failure::message(format!("bad opts {raw:?}: {error}")))
}

/// The value as compact JSON, for assertions. The engine carries every
/// number as an `f64`, the way JavaScript does, so an integral one is
/// put back as an integer: `1`, not `1.0`, which is what the fixtures
/// and the other two runtimes print.
pub fn json(value: &tabnas::Value) -> String {
    integral_numbers(value.to_json()).to_string()
}

/// Every whole number in a JSON tree as an integer.
pub fn integral_numbers(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) => match number.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => serde_json::json!(f as i64),
            _ => serde_json::Value::Number(number),
        },
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(integral_numbers).collect())
        }
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, integral_numbers(value)))
                .collect(),
        ),
        other => other,
    }
}
