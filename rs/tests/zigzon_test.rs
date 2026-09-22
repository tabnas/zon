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
use std::path::{Path, PathBuf};
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

/// The pinned census of each corpus, valid then invalid. It is one
/// definition rather than a literal per call site because
/// `the_corpus_census_in_the_docs_is_the_one_the_runners_pin` holds the
/// TypeScript and Go runners, and every census figure in every markdown
/// page in this repository, to exactly these numbers.
const ZIGZON_CENSUS: (usize, usize) = (184, 44);
const STRICTNESS_CENSUS: (usize, usize) = (68, 95);

/// Every ZON document harvested from the zig tree. Census re-measured
/// 2026-08-09 against zig 0.16.0; the TypeScript and Go runners pin the
/// same numbers.
#[test]
fn zigzon_reference_corpus() {
    run_zig_corpus(zigzon_corpus(), ZIGZON_CENSUS.0, ZIGZON_CENSUS.1);
}

/// The leniency probes: inputs designed to catch relaxed-JSON behaviour
/// leaking through the jsonic layer, with the verdicts supplied by the
/// same reference implementation.
#[test]
fn zig_strictness_probes() {
    run_zig_corpus(
        strictness_corpus(),
        STRICTNESS_CENSUS.0,
        STRICTNESS_CENSUS.1,
    );
}

// ---------------------------------------------------------------------
// The census as a documentation claim.
//
// The census is pinned in three runners and repeated in prose on several
// pages. Prose does not run, so a repeat goes stale silently: the line in
// `test/AGENTS.md` was moved once and then left behind when the corpus
// grew again, and nothing went red. Everything below derives the figure
// instead of restating it, so a census that moves has exactly one place
// to be changed and every copy of it is checked against that place.

/// Each corpus by the directory name that identifies it in a runner call
/// and in a documentation table, with its pinned census.
fn pinned_census() -> [(&'static str, (usize, usize)); 2] {
    [("zigzon", ZIGZON_CENSUS), ("strictness", STRICTNESS_CENSUS)]
}

/// Every `N / M` in `text`, as (line number, N, M). Spaces either side of
/// the slash are allowed, so both the `184/44` of a sentence and the
/// `184 / 184` of a table cell are found.
fn slashed_pairs(text: &str) -> Vec<(usize, usize, usize)> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            if !chars[at].is_ascii_digit() {
                at += 1;
                continue;
            }
            let left_start = at;
            while at < chars.len() && chars[at].is_ascii_digit() {
                at += 1;
            }
            let mut scan = at;
            while scan < chars.len() && chars[scan] == ' ' {
                scan += 1;
            }
            if scan >= chars.len() || chars[scan] != '/' {
                continue;
            }
            scan += 1;
            while scan < chars.len() && chars[scan] == ' ' {
                scan += 1;
            }
            let right_start = scan;
            while scan < chars.len() && chars[scan].is_ascii_digit() {
                scan += 1;
            }
            if scan == right_start {
                continue;
            }
            let left: String = chars[left_start..at].iter().collect();
            let right: String = chars[right_start..scan].iter().collect();
            found.push((
                index + 1,
                left.parse().expect("a digit run parses"),
                right.parse().expect("a digit run parses"),
            ));
            at = scan;
        }
    }
    found
}

/// The first run of digits after `marker`, skipping any spaces between.
fn number_after(text: &str, marker: &str) -> Option<usize> {
    let rest = text.split(marker).nth(1)?;
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Every markdown page in the repository, skipping the directories that
/// hold build output or a downloaded toolchain rather than this
/// repository's own prose.
fn markdown_pages(dir: &Path, found: &mut Vec<PathBuf>) {
    const SKIP: &[&str] = &[
        ".git",
        "node_modules",
        "target",
        "dist",
        "vendor",
        "coverage",
    ];
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.expect("a readable directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        if path.is_dir() {
            if !SKIP.contains(&name.as_str()) {
                markdown_pages(&path, found);
            }
        } else if name.ends_with(".md") {
            found.push(path);
        }
    }
}

/// One row of the corpus table a reader-facing page carries.
struct CorpusRow {
    corpus: String,
    documents: usize,
    accepted: (usize, usize),
    rejected: (usize, usize),
}

/// The corpus table on a reader-facing page, one `CorpusRow` per corpus.
/// A page without the table has none.
fn corpus_table(page: &str) -> Vec<CorpusRow> {
    let header = "| Corpus | Documents | Accepted correctly | Rejected correctly |";
    let Some(body) = page.split(header).nth(1) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for line in body.lines().skip(2) {
        if !line.starts_with('|') {
            break;
        }
        let cells: Vec<&str> = line.trim().trim_matches('|').split('|').collect();
        assert_eq!(cells.len(), 4, "a corpus table row has four cells: {line}");
        let corpus = if cells[0].contains("strictness") {
            "strictness"
        } else {
            "zigzon"
        };
        let documents: usize = cells[1]
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("the documents cell is a number: {line}"));
        let cell_pair = |cell: &str| -> (usize, usize) {
            let pairs = slashed_pairs(cell);
            assert_eq!(pairs.len(), 1, "a census cell is one `N / M`: {line}");
            (pairs[0].1, pairs[0].2)
        };
        rows.push(CorpusRow {
            corpus: corpus.to_string(),
            documents,
            accepted: cell_pair(cells[2]),
            rejected: cell_pair(cells[3]),
        });
    }
    rows
}

/// The census the TypeScript, Go and Rust runners pin is the census the
/// prose claims, everywhere the prose claims one.
///
/// Three things are held to `ZIGZON_CENSUS` and `STRICTNESS_CENSUS`: the
/// literals in the other two runners, the corpus tables in `README.md`
/// and `AGENTS.md`, and every other `N / M` written anywhere in this
/// repository's markdown. The last one is deliberately wide. A census
/// figure is the one number here that a reader takes as measured, so a
/// page may not carry a stale one; write a ratio that is not a census in
/// words rather than with a slash.
#[test]
fn the_corpus_census_in_the_docs_is_the_one_the_runners_pin() {
    let root = repo_root();
    let census = pinned_census();

    // The TypeScript runner: one `runCorpus(...)` call per corpus.
    let ts = fs::read_to_string(root.join("ts").join("test").join("zigzon.test.ts"))
        .expect("ts/test/zigzon.test.ts is readable");
    let mut ts_seen = 0;
    for call in ts.split("\nrunCorpus(").skip(1) {
        let call = call.split("\n)").next().expect("the call ends");
        let (name, (valid, invalid)) = census
            .iter()
            .find(|(name, _)| call.contains(&format!("'{name}'")))
            .expect("a runCorpus call names a known corpus");
        assert_eq!(
            (
                number_after(call, "{ valid:"),
                number_after(call, "invalid:")
            ),
            (Some(*valid), Some(*invalid)),
            "ts/test/zigzon.test.ts pins a different census for {name} than \
             ZIGZON_CENSUS / STRICTNESS_CENSUS in rs/tests/zigzon_test.rs"
        );
        ts_seen += 1;
    }
    assert_eq!(ts_seen, census.len(), "one runCorpus call per corpus");

    // The Go runner: one `runZigCorpus(t, ...)` call per corpus.
    let go = fs::read_to_string(root.join("go").join("zigzon_test.go"))
        .expect("go/zigzon_test.go is readable");
    let mut go_seen = 0;
    for call in go.split("runZigCorpus(t, ").skip(1) {
        let call = call.split(')').next().expect("the call ends");
        let lower = call.to_lowercase();
        let (name, (valid, invalid)) = census
            .iter()
            .find(|(name, _)| lower.contains(*name))
            .expect("a runZigCorpus call names a known corpus");
        let numbers: Vec<usize> = call
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        assert_eq!(
            numbers,
            vec![*valid, *invalid],
            "go/zigzon_test.go pins a different census for {name} than \
             ZIGZON_CENSUS / STRICTNESS_CENSUS in rs/tests/zigzon_test.rs"
        );
        go_seen += 1;
    }
    assert_eq!(go_seen, census.len(), "one runZigCorpus call per corpus");

    // Every markdown page: no stale figure anywhere, and the corpus
    // tables add up.
    let mut pages = Vec::new();
    markdown_pages(&root, &mut pages);
    assert!(pages.len() > 10, "the markdown walk found the pages");
    let mut claiming = Vec::new();
    for page in &pages {
        let shown = page
            .strip_prefix(&root)
            .unwrap_or(page)
            .display()
            .to_string();
        let text = fs::read_to_string(page).unwrap_or_else(|error| panic!("{shown}: {error}"));

        let mut claims = 0;
        for (line, left, right) in slashed_pairs(&text) {
            let is_pair = census
                .iter()
                .any(|(_, pinned)| (left, right) == (pinned.0, pinned.1));
            let is_half = left == right
                && census
                    .iter()
                    .any(|(_, pinned)| left == pinned.0 || left == pinned.1);
            assert!(
                is_pair || is_half,
                "{shown}:{line} writes `{left} / {right}`, which is not the pinned \
                 corpus census ({}/{} in zigzon, {}/{} in strictness). A stale census \
                 is a claim no runtime holds; a ratio that is not a census belongs in \
                 words, not in a slash.",
                ZIGZON_CENSUS.0,
                ZIGZON_CENSUS.1,
                STRICTNESS_CENSUS.0,
                STRICTNESS_CENSUS.1
            );
            claims += 1;
        }
        if 0 < claims {
            claiming.push(shown.clone());
        }

        for row in corpus_table(&text) {
            let corpus = &row.corpus;
            let (_, (valid, invalid)) = census
                .iter()
                .find(|(name, _)| *name == corpus)
                .expect("the table names a known corpus");
            assert_eq!(
                (row.accepted, row.rejected),
                ((*valid, *valid), (*invalid, *invalid)),
                "{shown}: the {corpus} row claims a pass rate the runners do not pin"
            );
            assert_eq!(
                row.documents,
                valid + invalid,
                "{shown}: the {corpus} row's document count is not its census summed"
            );
        }
    }

    // The pages that are supposed to carry the figure still do, so this
    // test cannot pass by the claims having been deleted.
    claiming.sort();
    assert_eq!(
        claiming,
        vec![
            "AGENTS.md".to_string(),
            "README.md".to_string(),
            "rs/AGENTS.md".to_string(),
            "test/AGENTS.md".to_string(),
        ],
        "the census is claimed on a different set of pages than expected; add the \
         page here once its figure is derived, or remove the figure from it"
    );
}
