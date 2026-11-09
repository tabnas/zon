// A parse from a reader (`Tabnas::parse_reader`, admin ADR-30). `make()`
// declares `lex.window`, so a reader is read a window at a time: the
// matchers this crate installs (`zonDot`, `zonMultiString`, `zonChar`,
// `zonNumber`, `zonDocComment` and `zonString`) read the source only
// through the lexer's window, and jsonic's base is the engine's own
// matchers. Every row of the shared fixtures and every probe of
// `test/strictness/inputs.txt` parses the same from a reader, handed over
// a few bytes at a time and under windows whose margin and block are a few
// characters (`parse_reader_with_sizes`, which makes every matcher meet the
// window's edge), as from its text; and so do tokens of every kind that the
// window's edge cuts at every place, tokens longer than the engine's own
// window, and long documents. `zigzon_test.rs` grades every document of the
// zig corpora from a reader as well as from its text, and
// `ts/test/window.test.ts` and `go/window_test.go` hold the other runtimes
// to the same.

mod common;

use std::collections::HashMap;
use std::fs;
use std::io::Read;

use tabnas::{Tabnas, TabnasError, Value, WindowSizes};
use tabnas_support::{load_spec_dir, SpecOptions};
use tabnas_zon::{make, make_with, ZonOptions};

use common::{repo_root, row_options, spec_dir};

/// The pieces a reader hands its bytes over in.
const PIECES: [usize; 3] = [1, 3, 64];

/// Windows whose margin and block are a few characters.
const WINDOWS: [(usize, usize); 3] = [(1, 1), (3, 2), (64, 64)];

/// A reader that hands its bytes over at most `size` at a time.
struct Pieces<'a> {
    bytes: &'a [u8],
    at: usize,
    size: usize,
}

impl Read for Pieces<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.size.min(buf.len()).min(self.bytes.len() - self.at);
        buf[..n].copy_from_slice(&self.bytes[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

fn pieces(bytes: &[u8], size: usize) -> Pieces<'_> {
    Pieces { bytes, at: 0, size }
}

/// What a parse shows: its value, shown with its keys in their order and
/// its numbers as they are (NaN, an infinity and -0 among them), or its
/// error's code, position, row, column and token text. An error's excerpt
/// is left out: a parse from a reader shows a long line only as far back as
/// the window reaches (tabnas/parser `DIVERGENCE.md`), and that is the one
/// difference allowed.
fn outcome(parsed: Result<Value, TabnasError>) -> String {
    match parsed {
        Ok(value) => format!("ok {value:?}"),
        Err(e) => format!(
            "err {} at {} {}:{} {:?}",
            e.code, e.pos, e.row, e.col, e.token.src
        ),
    }
}

/// The windows a document is read under: the engine's own sizes, and each
/// small one.
fn windows(chunk: usize) -> Vec<Option<WindowSizes>> {
    let mut out = vec![None];
    out.extend(WINDOWS.map(|(margin, block)| {
        Some(WindowSizes {
            margin,
            block,
            chunk,
        })
    }));
    out
}

fn read(parser: &Tabnas, bytes: &[u8], size: usize, sizes: Option<WindowSizes>) -> String {
    let reader = pieces(bytes, size);
    outcome(match sizes {
        None => parser.parse_reader(reader),
        Some(sizes) => parser.parse_reader_with_sizes(reader, sizes),
    })
}

/// Parse `text` whole and from a reader in pieces of each size, under
/// each window, and add where they differ to `failures`. Answers the
/// number of reader parses compared.
fn same_from_reader(
    parser: &Tabnas,
    at: &str,
    text: &str,
    sizes: &[usize],
    failures: &mut Vec<String>,
) -> usize {
    let whole = outcome(parser.parse(text));
    let mut compared = 0;
    for &size in sizes {
        for window in windows(size) {
            let got = read(parser, text.as_bytes(), size, window);
            if got != whole {
                failures.push(format!(
                    "{at}: {:?}, pieces of {size}, {window:?}:\n  parse:        {}\n  parse_reader: {}",
                    cut(text, 80),
                    cut(&whole, 200),
                    cut(&got, 200)
                ));
            }
            compared += 1;
        }
    }
    compared
}

fn cut(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

fn assert_none(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} differences, the first:\n{}",
        failures.len(),
        failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The options the zig corpora are graded with (`zigzon_test.rs`).
fn zig() -> ZonOptions {
    ZonOptions {
        enum_tag: Some("$enum".to_string()),
        char_as_number: true,
    }
}

/// One parser per set of options, built once: building the grammar costs
/// more than a parse.
#[derive(Default)]
struct Parsers(HashMap<String, Tabnas>);

impl Parsers {
    fn of(&mut self, options: &ZonOptions) -> &Tabnas {
        let key = format!("{:?}/{}", options.enum_tag, options.char_as_number);
        self.0.entry(key).or_insert_with(|| make_with(options))
    }
}

/// Every row of the shared fixtures: where it is, its input and its
/// options.
fn fixture_rows() -> Vec<(String, String, ZonOptions)> {
    let dir = spec_dir();
    let specs = load_spec_dir(&dir, &SpecOptions::default())
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
    let mut rows = Vec::new();
    for spec in &specs {
        for row in &spec.rows {
            let options = row_options(row.named("opts"))
                .unwrap_or_else(|failure| panic!("{}: {failure:?}", row.location()));
            rows.push((row.location(), row.unesc_named("input"), options));
        }
    }
    assert!(rows.len() >= 190, "only {} fixture rows", rows.len());
    rows
}

/// The probes of `test/strictness/inputs.txt`, the sources of the
/// strictness corpus, decoded as `scripts/fetch-zigzon.sh` decodes them:
/// `\n`, `\r`, `\t` and `\\`, with blank lines and comment lines skipped.
fn probes() -> Vec<String> {
    let path = repo_root()
        .join("test")
        .join("strictness")
        .join("inputs.txt");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut probe = String::new();
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                probe.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => probe.push('\n'),
                Some('r') => probe.push('\r'),
                Some('t') => probe.push('\t'),
                Some('\\') => probe.push('\\'),
                Some(other) => {
                    probe.push('\\');
                    probe.push(other);
                }
                None => probe.push('\\'),
            }
        }
        out.push(probe);
    }
    assert!(out.len() >= 160, "only {} probes", out.len());
    out
}

/// Tokens of every kind the matchers cut, well formed and not, each placed
/// where the window's edge cuts it at every place: alone, as a field's
/// value, and as an item of a tuple, after 0 to 6 spaces. The same shapes
/// as `ts/test/window.test.ts`.
const SHAPES: &[&str] = &[
    // zonDot: the struct opener, which reads on to decide map or tuple, a
    // field name, an enum literal, and the quoted name.
    ".{ .a = 1 }",
    ".{ 1, 2 }",
    ".{}",
    ".{ .abc }",
    ".{ .abc = .def }",
    ". foo",
    ".\n// c\nfoo",
    ".{ // c\n .a = 1 }",
    ".{ .@\"a b\" = 1 }",
    ".{ .@\"a\\u{41}\" = 1, .b = 2 }",
    ".@\"x\\u{41}y\"",
    ".@\"\"",
    ".@\"a",
    ".@\"\\x00\"",
    ".{\n\t.a = 1,\n}",
    ".{ . a = 1 }",
    ".{ .a\n= 1 }",
    "._a_1",
    // zonString.
    "\"abc\"",
    "\"a\\x41b\"",
    "\"\\xe2\\x82\\xac\"",
    "\"a\\u{1F600}b\"",
    "\"\\u{0000000041}\"",
    "\"\\u{D800}\"",
    "\"\\u{110000}\"",
    "\"\\u{}\"",
    "\"\\u{41\"",
    "\"\\u0041\"",
    "\"a\\qb\"",
    "\"abc",
    "\"ab\ncd\"",
    "\"a\tb\"",
    "\"\\x4\"",
    "\"\\\"",
    "\"\\",
    "\"€😀\"",
    "\"\\n\\r\\t\\\\\\'\\\"\"",
    // zonMultiString.
    "\\\\abc\n\\\\def",
    "\\\\abc\r\n\\\\def",
    "\\\\abc\n\n  \\\\def",
    "\\\\abc",
    "\\\\",
    "\\\\a\n",
    ".{ .t = \\\\a\n , .u = 1 }",
    // zonChar.
    "'a'",
    "'\\n'",
    "'\\x41'",
    "'\\u{1F600}'",
    "'\\u{0041}'",
    "'\\u{110000}'",
    "'😀'",
    "'\\u{41'",
    "'\\u{4G}'",
    "''",
    "'ab'",
    "'\\0'",
    "'\n'",
    "'a",
    "'\\x4'",
    "'é'",
    // zonNumber.
    "123",
    "1_000",
    "0x2A",
    "0x1.8p3",
    "1.5e-3",
    "1.e3",
    "0xF.p1",
    "-inf",
    "inf",
    "nan",
    "-nan",
    "-0",
    "-0.0",
    "0123",
    "1__0",
    "0X2A",
    "0b102",
    "99999999999999999999999",
    "- 5",
    "1.",
    "infinity",
    "0x_2A",
    "0o17",
    "1e",
    "1e+",
    "0x1p-2",
    "12abc",
    "1.2.3",
    "-",
    "0",
    // zonDocComment, and the plain comment it lets through.
    "//! x\n1",
    "/// x\n1",
    "//// x\n1",
    "// x\n1",
    "//",
    "///",
    "//!",
    "1 // x",
    "1 ///",
];

fn placed(shape: &str) -> Vec<String> {
    let mut out = Vec::new();
    for k in 0..=6 {
        let pad = " ".repeat(k);
        out.push(format!("{pad}{shape}"));
        out.push(format!(".{{ .k ={pad}{shape} }}"));
        out.push(format!(".{{{pad}{shape}, 0 }}"));
    }
    out
}

/// A long document: a struct of `n` fields of every kind, then tokens of
/// every kind `long` characters long, then `ending` in place of its close.
fn long_document(n: usize, long: usize, ending: &str) -> String {
    let mut text = String::from(".{\n");
    for i in 0..n {
        text.push_str(&format!(
            "  .f{i} = .{{ .s = \"s\\x41\\u{{1F600}}{i}\", .c = '\\u{{41}}', .n = {}, .x = 0x1.8p{}, \
             .e = .tag{i}, .q = .@\"q {i}\", .m = \\\\line {i}\n    \\\\more\n  , \
             .t = .{{ 1, -2.5, inf }} }}, // note {i}\n",
            i * 1_000_003,
            i % 7
        ));
    }
    text.push_str(&format!(
        "  .long_string = \"{}\\u{{{}41}}\",\n",
        "é".repeat(long),
        "0".repeat(long)
    ));
    text.push_str(&format!(
        "  .long_multi = \\\\{}\n{}\\\\end\n  ,\n",
        "m".repeat(long),
        " ".repeat(long)
    ));
    text.push_str(&format!("  .long_number = {}.5e-3,\n", "1_0".repeat(long)));
    text.push_str(&format!(
        "  .long_name_{} = .{}b,\n",
        "a".repeat(long),
        " ".repeat(long)
    ));
    text.push_str(&format!(
        "  .long_char = '\\u{{{}1F600}}',\n",
        "0".repeat(long)
    ));
    text.push_str(&format!(
        "  .long_open = .{}\n{{ .{}k = 1 }},\n",
        "\n// c".repeat(long),
        " ".repeat(long)
    ));
    text.push_str(ending);
    text
}

const ENDINGS: [&str; 8] = [
    "}\n",
    "  .bad = \"unterminated",
    "  .bad = 0X12,\n}",
    "  .bad = '\\u{110000}',\n}",
    "  .bad = .@\"\",\n}",
    "  .f0 = 1,\n}",
    "  //! doc\n}",
    "  .last = \\\\open",
];

#[test]
fn make_reads_a_reader_a_window_at_a_time() {
    assert!(make().options.lex.window);
    assert!(make_with(&zig()).options.lex.window);
    // The plugin leaves the window to the parser that installs it, since a
    // host may install it beneath matchers of its own.
    let mut parser = tabnas_jsonic::make();
    parser
        .use_plugin(tabnas_zon::plugin(), None)
        .expect("the plugin installs");
    assert!(!parser.options.lex.window);
}

#[test]
fn make_parses_as_the_composition() {
    let mut failures = Vec::new();
    let mut made = Parsers::default();
    for (at, input, options) in fixture_rows() {
        let mut composed = tabnas_jsonic::make();
        composed
            .use_plugin(tabnas_zon::plugin(), Some(options.to_value()))
            .expect("the plugin installs");
        let want = outcome(composed.parse(&input));
        let got = outcome(made.of(&options).parse(&input));
        if got != want {
            failures.push(format!("{at}:\n  make_with: {got}\n  composed:  {want}"));
        }
    }
    assert_none(&failures);
}

#[test]
fn fixtures_parse_the_same_from_a_reader() {
    let mut failures = Vec::new();
    let mut compared = 0;
    let mut made = Parsers::default();
    for (at, input, options) in fixture_rows() {
        compared += same_from_reader(made.of(&options), &at, &input, &PIECES, &mut failures);
    }
    eprintln!("reader fixtures: {compared} reader parses (100%)");
    assert!(compared >= 2300, "only {compared} reader parses");
    assert_none(&failures);
}

#[test]
fn the_strictness_probes_parse_the_same_from_a_reader() {
    let mut failures = Vec::new();
    let mut compared = 0;
    for options in [ZonOptions::default(), zig()] {
        let parser = make_with(&options);
        for probe in probes() {
            compared += same_from_reader(
                &parser,
                &format!("{options:?}"),
                &probe,
                &PIECES,
                &mut failures,
            );
        }
    }
    eprintln!("reader probes: {compared} reader parses (100%)");
    assert!(compared >= 2500, "only {compared} reader parses");
    assert_none(&failures);
}

#[test]
fn tokens_meet_the_window_edge_at_every_place() {
    let mut failures = Vec::new();
    let mut compared = 0;
    for options in [ZonOptions::default(), zig()] {
        let parser = make_with(&options);
        for shape in SHAPES {
            for text in placed(shape) {
                compared += same_from_reader(
                    &parser,
                    &format!("shape {shape:?}"),
                    &text,
                    &[1, 3],
                    &mut failures,
                );
            }
        }
    }
    eprintln!("reader shapes: {compared} reader parses (100%)");
    assert!(compared >= 20_000, "only {compared} reader parses");
    assert_none(&failures);
}

#[test]
fn tokens_longer_than_the_window_parse_the_same_from_a_reader() {
    // The engine's own window reaches 64 Ki characters past the cursor
    // before each token: a token longer than that meets its edge under the
    // engine's own sizes, and each matcher reads the window on as it cuts it.
    let long = (64 << 10) + 2048;
    let docs = [
        format!(".{{ .k = \"{}\\u{{1F600}}\", .z = 1 }}", "é".repeat(long)),
        format!(".{{ .k = \"\\u{{{}41}}\", .z = 1 }}", "0".repeat(long)),
        format!(".{{ .k = \"{}\\q\", .z = 1 }}", "q".repeat(long)),
        format!(".{{ .k = \"{}", "u".repeat(long)),
        format!(
            ".{{ .k = \\\\{}\n{}\\\\end\n, .z = 1 }}",
            "m".repeat(long),
            " ".repeat(long)
        ),
        format!(".{{ .k = {}.5, .z = 1 }}", "1".repeat(long)),
        format!(".{{ .k = {}x, .z = 1 }}", "1".repeat(long)),
        format!(".{{ .{} = 1, .z = 1 }}", "a".repeat(long)),
        format!(".{{ .k = .{}b, .z = 1 }}", " ".repeat(long)),
        format!(
            ".{{ .k = .{}\n{{ .{}k = 1 }} }}",
            "\n// c".repeat(long / 5),
            " ".repeat(long)
        ),
        format!(".{{ .k = .@\"{}\", .z = 1 }}", "w".repeat(long)),
        format!(".{{ .k = '\\u{{{}1F600}}', .z = 1 }}", "0".repeat(long)),
        format!(".{{ .k = '\\u{{{}110000}}', .z = 1 }}", "0".repeat(long)),
    ];
    let parser = make();
    let mut failures = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        same_from_reader(
            &parser,
            &format!("long token {i}"),
            doc,
            &[4096, (64 << 10) + 3],
            &mut failures,
        );
        eprintln!(
            "reader long tokens: {} of {} ({}%)",
            i + 1,
            docs.len(),
            100 * (i + 1) / docs.len()
        );
    }
    assert_none(&failures);
}

#[test]
fn a_long_document_parses_the_same_from_a_reader() {
    // About 50 KB, with a token of every kind longer than the small
    // windows, so that each is read on as it is cut, and the window drops
    // what lies behind the tokens the parser holds many times over.
    let parser = make();
    let mut failures = Vec::new();
    for (i, ending) in ENDINGS.iter().enumerate() {
        let text = long_document(60, 2000, ending);
        let whole = outcome(parser.parse(&text));
        if i == 0 {
            assert!(whole.starts_with("ok "), "{}", cut(&whole, 300));
        } else {
            assert!(
                whole.starts_with("err "),
                "{ending:?}: {}",
                cut(&whole, 300)
            );
        }
        same_from_reader(
            &parser,
            &format!("long, ending {ending:?}"),
            &text,
            &[64],
            &mut failures,
        );
        eprintln!(
            "reader long document: {} of {} endings ({}%)",
            i + 1,
            ENDINGS.len(),
            100 * (i + 1) / ENDINGS.len()
        );
    }
    assert_none(&failures);
}
