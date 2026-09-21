// Conformance against the ZIG REFERENCE IMPLEMENTATION.
//
// Two corpora, both GENERATED (never committed) by `scripts/fetch-zigzon.sh`
// from the pinned ziglang/zig 0.16.0 release:
//
//   test/zigzon/cases.json      every .zon file and every ZON snippet in the
//                               zig tree, plus its verdict/value
//   test/strictness/cases.json  locally authored leniency probes, judged by
//                               the same reference implementation
//
// Every verdict, and every valid document's expected VALUE, comes from
// `std.zig.Ast` + `std.zig.ZonGen` at that pinned version: this repo never
// decides what ZON means. Both halves are exercised: a valid document must
// parse AND produce the reference value (not merely "it did not fail"), and
// an invalid document must be rejected.
//
// The corpora are not bundled (generating them downloads a pinned zig
// toolchain), so this suite runs `scripts/fetch-zigzon.sh` itself before it
// grades, exactly as `TestMain` in `go/zigzon_test.go` and the `pretest`
// hook in `ts/package.json` do: `cargo test` builds them wherever it runs,
// CI included. A corpus that is still absent afterwards is a FAILURE, never
// a skip: a suite that quietly does not run reports a green tick that means
// nothing. The one exception is a host the pinned zig oracle toolchain is
// not wired for, where it reports a single explicit, platform-named skip.
//
// `ts/test/zigzon.test.ts` and `go/zigzon_test.go` run the identical
// corpora, so the three runtimes cannot drift.
//
// Do not shrink a corpus, add a skip list, or loosen the comparison to make
// the number look better: it is a measuring instrument.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Once;

use serde::Deserialize;
use tabnas::Value;
use tabnas_zon::{make_with, ZonOptions};

use common::{integral_numbers, repo_root};

fn zigzon_corpus() -> PathBuf {
    repo_root().join("test").join("zigzon").join("cases.json")
}

fn strictness_corpus() -> PathBuf {
    repo_root()
        .join("test")
        .join("strictness")
        .join("cases.json")
}

fn fetch_script() -> PathBuf {
    repo_root().join("scripts").join("fetch-zigzon.sh")
}

/// Whether `scripts/fetch-zigzon.sh` has a pinned zig oracle toolchain
/// for this platform. On anything else the corpus cannot be built at all,
/// and that, not a missing file, is what the single permitted skip
/// reports.
fn oracle_host() -> bool {
    matches!(std::env::consts::OS, "linux" | "macos")
        && matches!(std::env::consts::ARCH, "x86_64" | "aarch64")
}

/// Generate the conformance corpora before any corpus test reads them, so
/// the suites are never at the mercy of someone having remembered to run
/// a script. The Rust half of `TestMain` in `go/zigzon_test.go`. The fetch
/// is idempotent and cached, so this costs a few milliseconds once the
/// pinned downloads are in place.
fn ensure_corpora() {
    static FETCH: Once = Once::new();
    FETCH.call_once(|| {
        let present = zigzon_corpus().is_file() && strictness_corpus().is_file();
        if present || !oracle_host() {
            return;
        }
        // Do not panic here: let the suites below report the missing
        // corpus as a test failure, with instructions, rather than
        // poisoning the guard for every other test.
        match Command::new("bash").arg(fetch_script()).status() {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!("fetch-zigzon.sh failed: {status}"),
            Err(error) => eprintln!("fetch-zigzon.sh could not run: {error}"),
        }
    });
}

#[derive(Deserialize)]
struct ZigCase {
    #[serde(default)]
    name: String,
    #[serde(default)]
    origin: String,
    source: String,
    valid: bool,
    #[serde(default)]
    value: serde_json::Value,
    #[serde(default)]
    error: String,
}

#[derive(Deserialize)]
struct ZigCorpus {
    cases: Vec<ZigCase>,
}

/// The plugin options that put its output in the SAME shape as the
/// oracle's canonical encoding, so values compare directly: `enumTag`
/// distinguishes the enum literal `.foo` from the string "foo" (the
/// default flat encoding cannot), and `charAsNumber` matches ZonGen's
/// char_literal. This is representation alignment, not leniency: no
/// input is excused by it.
fn zig_options() -> ZonOptions {
    ZonOptions {
        enum_tag: Some("$enum".to_string()),
        char_as_number: true,
    }
}

/// A parsed value in the oracle's canonical JSON shape. The oracle spells
/// the non-JSON numbers "@inf"/"@-inf"/"@nan", and an integer too large
/// for an exact double as `{"$big": "<decimal>"}`, which is what the
/// plugin already returns for one. This only makes a CORRECT parse
/// comparable; it never excuses a wrong one.
fn zig_canon(value: &Value) -> serde_json::Value {
    match value {
        Value::Number(n) if n.is_nan() => serde_json::Value::String("@nan".to_string()),
        Value::Number(n) if *n == f64::INFINITY => serde_json::Value::String("@inf".to_string()),
        Value::Number(n) if *n == f64::NEG_INFINITY => {
            serde_json::Value::String("@-inf".to_string())
        }
        Value::Array(items) => serde_json::Value::Array(items.iter().map(zig_canon).collect()),
        Value::ListRef(list) => {
            serde_json::Value::Array(list.value.iter().map(zig_canon).collect())
        }
        Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), zig_canon(value)))
                .collect(),
        ),
        Value::MapRef(map) => serde_json::Value::Object(
            map.value
                .iter()
                .map(|(key, value)| (key.clone(), zig_canon(value)))
                .collect(),
        ),
        other => other.to_json(),
    }
}

fn label(case: &ZigCase) -> String {
    let one = case.source.split_whitespace().collect::<Vec<_>>().join(" ");
    let short = if 60 < one.chars().count() {
        format!("{}...", one.chars().take(57).collect::<String>())
    } else {
        one
    };
    let origin = if case.origin.is_empty() {
        &case.name
    } else {
        &case.origin
    };
    format!("{origin} | {short}")
}

/// Grade every document in a corpus. `want_valid` / `want_invalid` are the
/// pinned census: a floor of "more than zero" lets a broken generator
/// quietly shrink the corpus and inflate the pass rate, so the exact
/// counts are asserted.
fn run_zig_corpus(path: PathBuf, want_valid: usize, want_invalid: usize) {
    ensure_corpora();

    let Ok(body) = fs::read_to_string(&path) else {
        if !oracle_host() {
            // Not a hidden skip: it names the platform, and it cannot mask
            // a missing corpus anywhere the oracle can actually be built.
            eprintln!(
                "SKIP: no pinned zig oracle toolchain for {}/{}, so the corpus cannot be generated on this host",
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            return;
        }
        panic!(
            "corpus {} is missing.\nIt is generated by `bash scripts/fetch-zigzon.sh`, which this \
             suite runs before grading. This suite FAILS rather than skips when the corpus is \
             absent: a conformance suite that quietly does not run reports a green tick while \
             measuring nothing.",
            path.display()
        );
    };
    let corpus: ZigCorpus =
        serde_json::from_str(&body).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert!(
        !corpus.cases.is_empty(),
        "{}: corpus has no cases",
        path.display()
    );

    let valid = corpus.cases.iter().filter(|case| case.valid).count();
    let invalid = corpus.cases.len() - valid;
    assert!(
        valid == want_valid && invalid == want_invalid,
        "{}: corpus census is {valid} valid / {invalid} invalid, pinned at {want_valid} / \
         {want_invalid}. The generator changed, or the corpus was narrowed. Do not adjust this \
         number to match; find out what changed.",
        path.display()
    );

    let parser = make_with(&zig_options());
    let mut failures = Vec::new();
    for case in &corpus.cases {
        let result = parser.parse(&case.source);
        if !case.valid {
            if let Ok(got) = result {
                failures.push(format!(
                    "{}: accepted as {} a document zig rejects ({}):\n{}",
                    label(case),
                    zig_canon(&got),
                    case.error,
                    case.source
                ));
            }
            continue;
        }
        match result {
            Err(error) => failures.push(format!(
                "{}: rejected a document zig accepts:\n{}\n{error}",
                label(case),
                case.source
            )),
            Ok(got) => {
                let got = integral_numbers(zig_canon(&got));
                let want = integral_numbers(case.value.clone());
                if got != want {
                    failures.push(format!(
                        "{}: value mismatch\n  got  {got}\n  want {want}",
                        label(case)
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} documents in {} failed:\n{}",
        failures.len(),
        corpus.cases.len(),
        path.display(),
        failures.join("\n\n")
    );
}

/// Every ZON document harvested from the zig tree. Census re-measured
/// 2026-08-09 against zig 0.16.0; the TypeScript and Go runners pin the
/// same numbers.
#[test]
fn zigzon_reference_corpus() {
    run_zig_corpus(zigzon_corpus(), 184, 44);
}

/// The leniency probes: inputs designed to catch relaxed-JSON behaviour
/// leaking through the jsonic layer, with the verdicts supplied by the
/// same reference implementation.
#[test]
fn zig_strictness_probes() {
    run_zig_corpus(strictness_corpus(), 48, 74);
}
