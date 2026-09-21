// In-language behaviour the shared fixtures cannot express: the API
// surface, the values with no JSON spelling (big integers, infinities,
// NaN, negative zero), error messages, plugin layering and re-use, and
// the shared default parser under threads. Mirrors go/zon_test.go and
// ts/test/zon.test.ts case for case, plus what is specific to this port.

mod common;

use std::fs;

use tabnas::Value;
use tabnas_support::{load_spec_dir, SpecOptions};
use tabnas_zon::{
    make, make_with, parse, parse_with, plugin, zon, ZonOptions, BIG_KEY, GRAMMAR_TEXT,
};

use common::{json, repo_root, spec_dir};

fn with_options(char_as_number: bool, enum_tag: Option<&str>) -> ZonOptions {
    ZonOptions {
        char_as_number,
        enum_tag: enum_tag.map(str::to_string),
    }
}

fn number(src: &str) -> f64 {
    match parse(src) {
        Ok(Value::Number(n)) => n,
        other => panic!("parse({src:?}) is not a number: {other:?}"),
    }
}

fn big(src: &str) -> String {
    match parse(src) {
        Ok(Value::Object(fields)) => match fields.get(BIG_KEY) {
            Some(Value::String(digits)) => digits.clone(),
            other => panic!("parse({src:?}) has no {BIG_KEY}: {other:?}"),
        },
        other => panic!("parse({src:?}) is not a big integer: {other:?}"),
    }
}

// --- the go/zon_test.go cases ----------------------------------------------

#[test]
fn scalars() {
    assert_eq!(json(&parse("42").unwrap()), "42");
    assert_eq!(json(&parse("3.14").unwrap()), "3.14");
    assert_eq!(parse("true").unwrap(), Value::Bool(true));
    assert_eq!(parse("false").unwrap(), Value::Bool(false));
    assert_eq!(parse("null").unwrap(), Value::Null);
    assert_eq!(parse("\"hello\"").unwrap(), Value::String("hello".into()));
}

#[test]
fn numeric_bases() {
    assert_eq!(number("0x2a"), 42.0);
    assert_eq!(number("0o52"), 42.0);
    assert_eq!(number("0b101010"), 42.0);
    assert_eq!(number("1_000_000"), 1_000_000.0);
}

#[test]
fn enum_literal_as_value() {
    assert_eq!(parse(".foo").unwrap(), Value::String("foo".into()));
    assert_eq!(parse(".bar_baz").unwrap(), Value::String("bar_baz".into()));
    assert_eq!(
        json(&parse(".{ .kind = .red }").unwrap()),
        r#"{"kind":"red"}"#
    );
}

#[test]
fn structs_and_tuples() {
    assert_eq!(json(&parse(".{}").unwrap()), "[]");
    assert_eq!(json(&parse(".{ .a = 1 }").unwrap()), r#"{"a":1}"#);
    assert_eq!(
        json(&parse(".{ .a = 1, .b = 2 }").unwrap()),
        r#"{"a":1,"b":2}"#
    );
    assert_eq!(json(&parse(".{ .a = 1, }").unwrap()), r#"{"a":1}"#);
    assert_eq!(
        json(&parse(".{ .a = 1, .b = 2, }").unwrap()),
        r#"{"a":1,"b":2}"#
    );
    assert_eq!(json(&parse(".{ 1, 2, 3 }").unwrap()), "[1,2,3]");
    assert_eq!(json(&parse(".{ \"a\", \"b\" }").unwrap()), r#"["a","b"]"#);
    assert_eq!(json(&parse(".{ 1, 2, 3, }").unwrap()), "[1,2,3]");
    assert_eq!(
        json(&parse(".{ .a = .{ .b = 1 } }").unwrap()),
        r#"{"a":{"b":1}}"#
    );
    assert_eq!(
        json(&parse(".{ .{ 1, 2 }, .{ 3, 4 } }").unwrap()),
        "[[1,2],[3,4]]"
    );
    assert_eq!(
        json(&parse(".{ .xs = .{ 1, 2, 3 }, .y = .{ .z = true } }").unwrap()),
        r#"{"xs":[1,2,3],"y":{"z":true}}"#
    );
}

#[test]
fn string_escapes() {
    assert_eq!(parse(r#""a\nb""#).unwrap(), Value::String("a\nb".into()));
    assert_eq!(parse(r#""a\tb""#).unwrap(), Value::String("a\tb".into()));
    assert_eq!(parse(r#""a\\b""#).unwrap(), Value::String("a\\b".into()));
}

#[test]
fn comments() {
    let src = ".{\n\t// a comment\n\t.name = \"x\", // trailing comment\n\t.version = \"1.0\", // version\n}";
    assert_eq!(
        json(&parse(src).unwrap()),
        r#"{"name":"x","version":"1.0"}"#
    );
}

#[test]
fn realistic_manifest() {
    let src = r#".{
        .name = "example",
        .version = "0.0.1",
        .minimum_zig_version = "0.14.0",
        .dependencies = .{
            .foo = .{
                .url = "https://example.com/foo.tar.gz",
                .hash = "1220deadbeef",
            },
        },
        .paths = .{
            "build.zig",
            "src",
            "",
        },
    }"#;
    assert_eq!(
        json(&parse(src).unwrap()),
        r#"{"name":"example","version":"0.0.1","minimum_zig_version":"0.14.0","dependencies":{"foo":{"url":"https://example.com/foo.tar.gz","hash":"1220deadbeef"}},"paths":["build.zig","src",""]}"#
    );
}

#[test]
fn char_literals() {
    let as_number = with_options(true, None);
    assert_eq!(json(&parse_with("'A'", &as_number).unwrap()), "65");
    assert_eq!(json(&parse_with("'\\n'", &as_number).unwrap()), "10");
    assert_eq!(
        json(&parse_with("'\\u{1F600}'", &as_number).unwrap()),
        "128512"
    );
    assert_eq!(parse("'A'").unwrap(), Value::String("A".into()));
    assert_eq!(
        parse("'\\u{1F600}'").unwrap(),
        Value::String("\u{1F600}".into())
    );
}

#[test]
fn multi_line_string() {
    let src = ".{\n\t.text = \\\\hello\n\t\t\\\\world\n\t,\n}";
    assert_eq!(json(&parse(src).unwrap()), r#"{"text":"hello\nworld"}"#);
}

#[test]
fn enum_tag_option() {
    let tagged = with_options(false, Some("$enum"));
    assert_eq!(
        json(&parse_with(".{ .kind = .red }", &tagged).unwrap()),
        r#"{"kind":{"$enum":"red"}}"#
    );
    // At the top level too, and for an escaped identifier.
    assert_eq!(
        json(&parse_with(".red", &tagged).unwrap()),
        r#"{"$enum":"red"}"#
    );
    assert_eq!(
        json(&parse_with(".@\"x x\"", &tagged).unwrap()),
        r#"{"$enum":"x x"}"#
    );
    // A field NAME is never wrapped, and a string never is.
    assert_eq!(
        json(&parse_with(".{ .kind = \"red\" }", &tagged).unwrap()),
        r#"{"kind":"red"}"#
    );
    // An empty tag means unset, as in the Go port.
    assert_eq!(
        json(&parse_with(".red", &with_options(false, Some(""))).unwrap()),
        r#""red""#
    );
}

#[test]
fn syntax_error() {
    // `{` without `.` is not valid ZON.
    assert!(parse("{ a = 1 }").is_err());
}

// --- values with no JSON spelling (see test/AGENTS.md) ----------------------

#[test]
fn big_integers_keep_their_exact_value() {
    // 2^65 - 1 is not representable as an IEEE-754 double, so the plugin
    // keeps the digits rather than silently rounding: as a bigint in
    // TypeScript, a *big.Int in Go, and here the `{ "$big": digits }`
    // object the engine's value model can hold.
    let want = "36893488147419103231";
    for src in [
        "36893488147419103231",
        "368934_881_474191032_31",
        "0x1ffffffffffffffff",
        "0o3777777777777777777777",
        &format!("0b{}", "1".repeat(65)),
    ] {
        assert_eq!(big(src), want, "parse({src:?})");
    }
    assert_eq!(big("-36893488147419103231"), "-36893488147419103231");
    assert_eq!(big("9007199254740993"), "9007199254740993");
    assert_eq!(
        json(&parse("36893488147419103231").unwrap()),
        r#"{"$big":"36893488147419103231"}"#
    );
}

#[test]
fn integers_that_fit_a_double_stay_numbers() {
    assert_eq!(number("9007199254740992"), 9007199254740992.0);
    assert_eq!(number("18446744073709551616"), 18446744073709551616.0);
    assert_eq!(number("-36893488147419103232"), -36893488147419103232.0);
}

#[test]
fn inf_and_nan_literals() {
    assert_eq!(number("inf"), f64::INFINITY);
    assert_eq!(number("-inf"), f64::NEG_INFINITY);
    assert_eq!(number("- inf"), f64::NEG_INFINITY);
    assert!(number("nan").is_nan());
    let Value::Array(items) = parse(".{ nan, inf, -inf }").unwrap() else {
        panic!("a list")
    };
    assert!(items
        .iter()
        .all(|item| matches!(item, Value::Number(n) if !n.is_finite())));
    // `-nan` is not a Zig numeric literal.
    assert_eq!(parse("-nan").unwrap_err().code, "zon_number");
}

#[test]
fn negative_zero() {
    for src in ["-0.0", "-0e0"] {
        let got = number(src);
        assert!(
            got == 0.0 && got.is_sign_negative(),
            "parse({src:?}) = {got}"
        );
    }
    // But an integer `-0` is ambiguous in Zig and rejected.
    assert_eq!(parse("-0").unwrap_err().code, "zon_number");
}

#[test]
fn hex_floats() {
    assert_eq!(number("0x1p4"), 16.0);
    assert_eq!(number("0x1.8"), 1.5);
    assert_eq!(number("0x.Fp1"), 1.875);
    assert_eq!(number("0x1234_5678.9ABC_CDEFp-10"), 298261.6177777768);
    // A mantissa beyond 53 bits rounds half to even, as `Number(BigInt)`.
    assert_eq!(number("0x20000000000001p0"), 9007199254740992.0);
    assert_eq!(number("0x20000000000003p0"), 9007199254740996.0);
    // Overflow is infinity, as in Zig.
    assert_eq!(number("0x1p1024"), f64::INFINITY);
    assert_eq!(number("1e999"), f64::INFINITY);
}

// --- errors ----------------------------------------------------------------

#[test]
fn duplicate_struct_field_names_are_rejected() {
    for src in [".{ .a = 1, .a = 2 }", ".{ .@\"a\" = 1, .a = 2 }"] {
        let error = parse(src).unwrap_err();
        assert_eq!(error.code, "zon_dup_field", "{src}");
        assert!(
            error.to_string().contains("duplicate struct field"),
            "{src}: {error}"
        );
    }
    // Sibling structs may of course repeat a name.
    assert_eq!(
        json(&parse(".{ .x = .{ .a = 1 }, .y = .{ .a = 2 } }").unwrap()),
        r#"{"x":{"a":1},"y":{"a":2}}"#
    );
}

#[test]
fn doc_comments_are_rejected_and_ordinary_comments_are_not() {
    for src in ["//! doc\n1", "/// doc\n1"] {
        let error = parse(src).unwrap_err();
        assert_eq!(error.code, "zon_doc_comment", "{src}");
        assert!(error.to_string().contains("doc comments"), "{src}: {error}");
    }
    for src in ["//// four\n1", "// two\n1"] {
        assert_eq!(number(src), 1.0, "{src}");
    }
}

#[test]
fn every_declared_error_code_is_raised_with_its_message() {
    for (src, code, message) in [
        ("0X2A", "zon_number", "invalid ZON number literal: 0X2A"),
        (".@\"\"", "zon_ident", "invalid ZON identifier: .@\""),
        (
            "'\\u{110000}'",
            "zon_char",
            "invalid ZON character literal: '\\u{110000}'",
        ),
        (
            "///x",
            "zon_doc_comment",
            "doc comments are not allowed in ZON: ///",
        ),
        (
            ".{ .a = 1, .a = 2 }",
            "zon_dup_field",
            "duplicate struct field name: .a",
        ),
    ] {
        let error = parse(src).unwrap_err();
        assert_eq!(error.code, code, "{src}");
        assert!(error.to_string().contains(message), "{src}: {error}");
    }
}

#[test]
fn errors_carry_the_zon_code_and_a_position() {
    let error = parse(".{\n  .a = 1,\n  .b = 0X2A,\n}").unwrap_err();
    assert_eq!(error.code, "zon_number");
    assert_eq!((error.row, error.col), (3, 8));
    // A dot-prefixed token that crosses a line keeps positions honest.
    let error = parse(".\n  foo = 1").unwrap_err();
    assert_eq!(error.code, "unexpected");
    assert_eq!(error.row, 2);
}

// --- the API -----------------------------------------------------------------

#[test]
fn plugin_installs_on_a_jsonic_instance() {
    let mut parser = tabnas_jsonic::make();
    parser
        .use_plugin(plugin(), Some(with_options(true, Some("$enum")).to_value()))
        .expect("installs");
    assert_eq!(
        json(&parser.parse(".{ .c = 'A', .e = .x }").unwrap()),
        r#"{"c":65,"e":{"$enum":"x"}}"#
    );
    // The resolved options are on the instance under the plugin's name.
    let options = parser.plugin_options("zon").expect("recorded");
    assert_eq!(
        ZonOptions::from_value(options),
        with_options(true, Some("$enum"))
    );
}

#[test]
fn plugin_defaults_are_the_canonical_ones() {
    assert_eq!(ZonOptions::default(), with_options(false, None));
    assert_eq!(
        plugin().defaults.to_json().to_string(),
        r#"{"charAsNumber":false,"enumTag":null}"#
    );
    // A bag with unknown or missing fields reads as the defaults.
    let bag = Value::from_json(&serde_json::json!({ "other": 1 }));
    assert_eq!(ZonOptions::from_value(&bag), ZonOptions::default());
}

#[test]
fn zon_installs_directly_and_is_idempotent() {
    let mut parser = tabnas_jsonic::make();
    zon(&mut parser, &ZonOptions::default()).expect("installs");
    zon(&mut parser, &ZonOptions::default()).expect("a re-run is a no-op");
    assert_eq!(
        json(&parser.parse(".{ 1, .{ .a = 2 } }").unwrap()),
        r#"[1,{"a":2}]"#
    );
    // The alternates were not doubled: the alternate count is what one
    // install leaves, so a duplicate field is still caught and a stray
    // input is still one error.
    assert_eq!(
        parser.parse(".{ .a = 1, .a = 2 }").unwrap_err().code,
        "zon_dup_field"
    );
}

#[test]
fn jsonic_relaxations_are_switched_off() {
    // Implicit maps and lists, path dives, bare braces and brackets,
    // colons, single-quoted strings, hash and block comments: none of
    // the relaxed-JSON extensions survive.
    for src in [
        "a:1",
        "1,2",
        "a:b:1",
        "{ a = 1 }",
        "[1,2]",
        ".{ .a: 1 }",
        "'ab'",
        "# comment\n1",
        "/* c */ 1",
        "+1",
        ".5",
        "0123",
        "1__0",
        "a",
    ] {
        assert!(parse(src).is_err(), "accepted {src:?}: {:?}", parse(src));
    }
}

#[test]
fn the_grammar_can_be_excluded_by_its_group() {
    // Every alternate the plugin adds carries the `zon` group.
    let mut parser = make();
    parser
        .set_options(|options| options.rule.exclude = "jsonic,imp,zon".to_string())
        .expect("options apply");
    assert!(parser.parse(".{ 1, 2 }").is_err());
    assert!(make().parse(".{ 1, 2 }").is_ok());
}

#[test]
fn make_with_reads_the_typed_options() {
    let parser = make_with(&with_options(true, Some("kind")));
    assert_eq!(
        json(&parser.parse(".{ 'x', .y }").unwrap()),
        r#"[120,{"kind":"y"}]"#
    );
}

#[test]
fn the_embedded_grammar_is_the_authored_one() {
    // ts/embed-grammar.js copies zon-grammar.jsonic verbatim between the
    // BEGIN/END markers of every runtime, as a raw string here. The
    // literal opens with the newline the embedder writes after `r##"`.
    let authored = fs::read_to_string(repo_root().join("zon-grammar.jsonic"))
        .expect("zon-grammar.jsonic is readable");
    assert_eq!(GRAMMAR_TEXT, format!("\n{authored}"));
    // And the same text sits in the other two runtimes.
    for path in ["ts/src/zon.ts", "go/zon.go"] {
        let source = fs::read_to_string(repo_root().join(path)).expect("readable");
        assert!(
            source.contains(&authored),
            "{path} embeds a different grammar"
        );
    }
}

#[test]
fn the_fixture_census_is_what_the_other_runtimes_run() {
    // The parity runner discovers the files by listing; this pins that
    // the listing is the whole shared corpus and that every file has
    // the standard shape the Go and TypeScript runners expect.
    let specs = load_spec_dir(spec_dir(), &SpecOptions::default()).expect("fixtures load");
    let mut files: Vec<&str> = specs.iter().map(|spec| spec.file.as_str()).collect();
    files.sort_unstable();
    assert_eq!(
        files,
        [
            "chars.tsv",
            "comments.tsv",
            "enums.tsv",
            "errors.tsv",
            "nesting.tsv",
            "numbers.tsv",
            "realworld.tsv",
            "scalars.tsv",
            "strict.tsv",
            "strings.tsv",
            "struct.tsv",
            "tuple.tsv",
        ]
    );
    for spec in &specs {
        assert!(!spec.rows.is_empty(), "{} has no rows", spec.file);
        assert_eq!(
            spec.header.as_slice(),
            ["input", "expected", "opts"],
            "{}",
            spec.file
        );
    }
}

#[test]
fn parse_is_safe_across_threads() {
    // The default parser is built once and shared, as `sync.Once` shares
    // it in the Go port. Failing parses are interleaved with succeeding
    // ones on purpose: a lexer or rule-stack leak across calls would
    // surface as a wrong value or a spurious error here.
    let threads: Vec<_> = (0..8)
        .map(|n| {
            std::thread::spawn(move || {
                let src = format!(".{{ .n = {n}, .xs = .{{ 1, 2, 3 }}, .s = \"a b\" }}");
                for _ in 0..50 {
                    let value = parse(&src).expect("parses");
                    let Value::Object(fields) = &value else {
                        panic!("an object, got {value:?}")
                    };
                    assert_eq!(fields["n"], Value::Number(f64::from(n)));
                    assert_eq!(json(&fields["xs"]), "[1,2,3]");
                    assert!(parse("}").is_err());
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("no thread panicked");
    }
}
