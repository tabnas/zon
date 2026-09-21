// In-language behaviour the shared fixtures cannot express: the API
// surface, the values with no JSON spelling (big integers, infinities,
// NaN, negative zero), error messages, plugin layering and re-use, and
// the shared default parser under threads. Mirrors go/zon_test.go and
// ts/test/zon.test.ts case for case, plus what is specific to this port.

mod common;

use std::fs;

use indexmap::IndexMap;
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

/// Parse `src` with a raw option bag, as a caller of `use_plugin` hands
/// one over. The bag is an ENGINE value, so it can hold what JSON cannot:
/// an infinity, a NaN, a negative zero.
fn parse_bag_value(bag: Value, src: &str) -> String {
    let mut parser = tabnas_jsonic::make();
    parser
        .use_plugin(plugin(), Some(bag))
        .expect("the plugin installs");
    json(&parser.parse(src).expect("parses"))
}

/// A one-field option bag holding an engine number.
fn number_bag(name: &str, value: f64) -> Value {
    let mut fields = IndexMap::new();
    fields.insert(name.to_string(), Value::Number(value));
    Value::object(fields)
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

#[test]
fn a_failed_install_is_not_remembered_as_done() {
    // Installing on a bare engine fails, since ZON reshapes jsonic's
    // rules; the failure must not mark the instance as initialised, or
    // the retry after installing jsonic would silently do nothing.
    let mut parser = tabnas::Tabnas::new();
    assert!(tabnas_zon::zon(&mut parser, &ZonOptions::default()).is_err());
    tabnas_jsonic::jsonic(&mut parser).expect("jsonic installs");
    tabnas_zon::zon(&mut parser, &ZonOptions::default())
        .expect("zon installs once the base is there");
    assert_eq!(
        parser.parse(".{ 1, 2 }").expect("parses").to_string(),
        "[1,2]"
    );
}

#[test]
fn a_long_integer_literal_parses_in_linear_time() {
    // Untrusted input can carry a literal of any length; the decimal
    // path keeps the digits and the hex path packs bits, so neither is
    // quadratic. The bounds are loose for a loaded debug build.
    let started = std::time::Instant::now();
    let decimal = format!("1{}", "0".repeat(300_000));
    assert_eq!(big(&decimal), decimal);
    assert!(
        started.elapsed().as_secs() < 5,
        "decimal took {:?}",
        started.elapsed()
    );

    let started = std::time::Instant::now();
    let hex = format!("0x{}", "f".repeat(20_000));
    let digits = big(&hex);
    assert_eq!(digits.len(), 24_083);
    assert!(digits.ends_with('5'));
    assert!(
        started.elapsed().as_secs() < 20,
        "hex took {:?}",
        started.elapsed()
    );
}

#[test]
fn the_option_conversion_pair_is_lossless() {
    // `to_value` and `from_value` are the public conversion between the
    // typed options and the engine's bag, so every valid typed value has
    // to survive the round trip. An EMPTY tag is a PRESENT option, not an
    // absent one: the canonical `options.enumTag || null` is a SEMANTIC
    // test at the point of use, which `tag` performs when the rewrap is
    // wired, so the conversion must not perform it again and lose the
    // string a caller recorded.
    for options in [
        ZonOptions::default(),
        with_options(true, None),
        with_options(false, Some("")),
        with_options(true, Some("")),
        with_options(false, Some("$enum")),
        with_options(true, Some("$enum")),
        // Whitespace and a quote are ordinary characters in a tag.
        with_options(false, Some(" ")),
        with_options(false, Some("a\"b")),
    ] {
        assert_eq!(
            ZonOptions::from_value(&options.to_value()),
            options,
            "{options:?}"
        );
    }

    // The semantics are unchanged at the point of use: an empty tag is
    // still unset, as `options.enumTag || null` makes it, and a tag of
    // one space is NOT empty and does name a key. Both measured against
    // ts/src/zon.ts.
    assert_eq!(
        json(&parse_with(".{ .k = .red }", &with_options(false, Some(""))).unwrap()),
        r#"{"k":"red"}"#
    );
    assert_eq!(
        json(&parse_with(".{ .k = .red }", &with_options(false, Some(" "))).unwrap()),
        r#"{"k":{" ":"red"}}"#
    );
    // `char_as_number` is a plain bool, so `false` and absent are the
    // same option, as `!!options.charAsNumber` makes them.
    assert_eq!(
        parse_bag_value(Value::object(IndexMap::new()), "'A'"),
        r#""A""#
    );
}

#[test]
fn a_numeric_tag_names_the_key_javascript_names() {
    // The canonical plugin uses the option as a COMPUTED PROPERTY KEY,
    // which spells a number with `Number::toString` (ECMA-262
    // 6.1.6.1.20). That is neither Rust's shortest float form nor
    // serde_json's: it writes `10000000000000000` out in full, switches
    // to exponent form only at 1e21 and at 1e-7, and never leaves a
    // trailing `.0`. Every expectation is `String(tag)` in node.
    let tagged = |tag: f64| parse_bag_value(number_bag("enumTag", tag), ".{ .k = .red }");
    for (tag, want) in [
        (123.0, "123"),
        (1.5, "1.5"),
        (-1.5, "-1.5"),
        (100.0, "100"),
        (0.1, "0.1"),
        (1e15, "1000000000000000"),
        (9e15, "9000000000000000"),
        (9007199254740992.0, "9007199254740992"),
        (1e16, "10000000000000000"),
        (1234567890123456800.0, "1234567890123456800"),
        (1e20, "100000000000000000000"),
        (1e21, "1e+21"),
        (0.000001, "0.000001"),
        (1e-7, "1e-7"),
        (1e-10, "1e-10"),
        (5e-324, "5e-324"),
        (f64::MAX, "1.7976931348623157e+308"),
    ] {
        assert_eq!(
            tagged(tag),
            format!(r#"{{"k":{{"{want}":"red"}}}}"#),
            "{tag:?}"
        );
    }
    // A falsy number never reaches the computed key: `|| null` discards
    // it first, in both runtimes.
    for tag in [0.0, -0.0, f64::NAN] {
        assert_eq!(tagged(tag), r#"{"k":"red"}"#, "{tag:?}");
    }
}

#[test]
fn a_non_finite_option_is_read_before_the_json_projection() {
    // `Value::to_json` renders a non-finite number as `null`, and `null`
    // is falsy where `Infinity` is not, so a truthiness or type test
    // taken AFTER that projection sees something the canonical runtime
    // never saw. The option bag is therefore read as the engine value it
    // is. Measured against ts/src/zon.ts: `Infinity` is a truthy
    // `charAsNumber` and names the key `Infinity` as a tag, `-Infinity`
    // names `-Infinity`, and `NaN` is falsy for both.
    assert_eq!(
        parse_bag_value(number_bag("charAsNumber", f64::INFINITY), "'A'"),
        "65"
    );
    assert_eq!(
        parse_bag_value(number_bag("charAsNumber", f64::NEG_INFINITY), "'A'"),
        "65"
    );
    assert_eq!(
        parse_bag_value(number_bag("charAsNumber", f64::NAN), "'A'"),
        r#""A""#
    );
    assert_eq!(
        parse_bag_value(number_bag("enumTag", f64::INFINITY), ".{ .k = .red }"),
        r#"{"k":{"Infinity":"red"}}"#
    );
    assert_eq!(
        parse_bag_value(number_bag("enumTag", f64::NEG_INFINITY), ".{ .k = .red }"),
        r#"{"k":{"-Infinity":"red"}}"#
    );
    assert_eq!(
        parse_bag_value(number_bag("enumTag", f64::NAN), ".{ .k = .red }"),
        r#"{"k":"red"}"#
    );
}

// --- the divergences DIVERGENCE.md records ---------------------------------
//
// Each test below pins a MEASURED difference from the canonical
// TypeScript, so repairing one fails here as loudly as regressing it. A
// repair means deleting the entry in DIVERGENCE.md and the test in the
// same change.

#[test]
fn nesting_is_bounded_by_the_depth_budget() {
    // Inherited from tabnas-jsonic, which refuses the 128th open
    // container with the engine's `cancel` code: the value a parse
    // returns is walked with the call stack to display, convert or drop,
    // so an unbounded one ends the process instead of returning an
    // error. TypeScript and Go have no such limit and parse both of
    // these.
    let open =
        |depth: usize, opener: &str| format!("{}1{}", opener.repeat(depth), " }".repeat(depth));
    for opener in [".{ .a = ", ".{ "] {
        assert!(
            parse(&open(127, opener)).is_ok(),
            "127 levels of {opener:?} parse"
        );
        let refused = parse(&open(128, opener)).expect_err("128 levels are refused");
        assert_eq!(refused.code, "cancel", "{opener:?}");
    }
}

#[test]
fn a_multi_line_string_leaves_the_column_honest() {
    // The canonical runtime advances the column of a `\\` string run by
    // the token's whole length, newlines included, so every later column
    // on that line is reported too far right: TypeScript and Go both say
    // 2:20 here, on a line eight characters long. The engine's lexer
    // exposes only `advance_chars`, which resets the column at each
    // newline, so this port reports the true column instead.
    let error = parse(".{ .a = \\\\x\n, .b = }").expect_err("the input is a syntax error");
    assert_eq!(error.code, "unexpected");
    assert_eq!((error.row, error.col), (2, 8));
}

#[test]
fn an_absurd_decimal_exponent_saturates() {
    // An exponent of 21 digits or more overflows what the canonical
    // runtime's `parseInt` keeps exactly, and it then spells the value
    // back into the literal in exponent form, so `parseFloat` reads only
    // the prefix: TypeScript gives 10 and 0.1 for these two. Go rejects
    // both. This port saturates the exponent, so the value is the
    // infinity or the zero the magnitude calls for.
    assert_eq!(number("1e999999999999999999999"), f64::INFINITY);
    assert_eq!(number("1e-999999999999999999999"), 0.0);
    // A 20-digit exponent is inside the saturation and agrees with
    // TypeScript exactly.
    assert_eq!(number("1e99999999999999999999"), f64::INFINITY);
    // So does an ordinary out-of-range exponent, which all three
    // runtimes answer with an infinity.
    assert_eq!(number("1e400"), f64::INFINITY);
    // The hexadecimal `p` form saturates the same way and agrees with
    // TypeScript at both ends, where Go rejects it.
    assert_eq!(number("0x1p-99999999999999999999"), 0.0);
    assert_eq!(number("0x1p99999999999999999999"), f64::INFINITY);
}

#[test]
fn an_option_bag_field_is_read_on_its_own() {
    // The canonical plugin reads `!!options.charAsNumber` and
    // `options.enumTag || null`, field by field, so an ill-typed field
    // cannot discard a well-typed one and a truthy value is accepted for
    // either. Measured against ts/src/zon.ts.
    let bag = |json: &str| {
        Value::from_json(&serde_json::from_str::<serde_json::Value>(json).expect("valid JSON"))
    };
    let parse_bag = |bag_json: &str, src: &str| {
        let mut parser = tabnas_jsonic::make();
        parser
            .use_plugin(plugin(), Some(bag(bag_json)))
            .expect("the plugin installs");
        json(&parser.parse(src).expect("parses"))
    };

    // The field that matters here is valid; the other one is not.
    assert_eq!(
        parse_bag(r#"{"charAsNumber":true,"enumTag":false}"#, "'A'"),
        "65"
    );
    // JavaScript truthiness, not a strict boolean. The Go port asserts
    // the option to `bool` instead, so it reads these two as unset and
    // gives `"A"`, which DIVERGENCE.md measures.
    assert_eq!(parse_bag(r#"{"charAsNumber":1}"#, "'A'"), "65");
    assert_eq!(parse_bag(r#"{"charAsNumber":"yes"}"#, "'A'"), "65");
    assert_eq!(parse_bag(r#"{"charAsNumber":null}"#, "'A'"), r#""A""#);
    assert_eq!(parse_bag(r#"{"charAsNumber":0}"#, "'A'"), r#""A""#);
    // A plain boolean and a plain string tag: the first row of each
    // table in DIVERGENCE.md, where all three runtimes agree.
    assert_eq!(parse_bag(r#"{"charAsNumber":true}"#, "'A'"), "65");
    assert_eq!(
        parse_bag(r#"{"enumTag":"$e"}"#, ".{ .k = .red }"),
        r#"{"k":{"$e":"red"}}"#
    );
    // An empty or falsy tag means unset; a non-string one is the key it
    // stringifies to, as a computed property key is.
    assert_eq!(
        parse_bag(r#"{"enumTag":""}"#, ".{ .k = .red }"),
        r#"{"k":"red"}"#
    );
    assert_eq!(
        parse_bag(r#"{"enumTag":123}"#, ".{ .k = .red }"),
        r#"{"k":{"123":"red"}}"#
    );
    assert_eq!(
        parse_bag(r#"{"enumTag":1.5}"#, ".{ .k = .red }"),
        r#"{"k":{"1.5":"red"}}"#
    );
    assert_eq!(
        parse_bag(r#"{"enumTag":true}"#, ".{ .k = .red }"),
        r#"{"k":{"true":"red"}}"#
    );
    // An array or an object is outside the option's type in every
    // runtime, and is where this port stops matching: TypeScript names
    // the key `1,2` and `[object Object]` (DIVERGENCE.md).
    assert_eq!(
        parse_bag(r#"{"enumTag":[1,2]}"#, ".{ .k = .red }"),
        r#"{"k":{"[1,2]":"red"}}"#
    );
    assert_eq!(
        parse_bag(r#"{"enumTag":{"a":1}}"#, ".{ .k = .red }"),
        r#"{"k":{"{\"a\":1}":"red"}}"#
    );
    // An EMPTY array or object is truthy in JavaScript too, so the tag
    // is SET in both runtimes and only the key differs: TypeScript names
    // `""` and `[object Object]` (DIVERGENCE.md).
    assert_eq!(
        parse_bag(r#"{"enumTag":[]}"#, ".{ .k = .red }"),
        r#"{"k":{"[]":"red"}}"#
    );
    assert_eq!(
        parse_bag(r#"{"enumTag":{}}"#, ".{ .k = .red }"),
        r#"{"k":{"{}":"red"}}"#
    );
    // A non-finite element of such a container has no JSON spelling and
    // becomes `null` in the one this port writes, where TypeScript joins
    // the element as `Infinity`.
    assert_eq!(
        parse_bag_value(
            {
                let mut fields = IndexMap::new();
                fields.insert(
                    "enumTag".to_string(),
                    Value::array(vec![Value::Number(f64::INFINITY)]),
                );
                Value::object(fields)
            },
            ".{ .k = .red }"
        ),
        r#"{"k":{"[null]":"red"}}"#
    );

    // An unknown field is still ignored, and the typed round trip holds.
    assert_eq!(parse_bag(r#"{"charAsNumber":true,"bogus":9}"#, "'A'"), "65");
    let options = with_options(true, Some("$enum"));
    assert_eq!(ZonOptions::from_value(&options.to_value()), options);
}

#[test]
fn a_lone_surrogate_folds_to_the_replacement_character() {
    // A JavaScript string is a sequence of UTF-16 code units and can
    // hold an unpaired surrogate; a Rust `String` holds Unicode scalar
    // values and cannot. The canonical runtime keeps U+D800 in all three
    // spellings below, and this port substitutes U+FFFD, as the engine
    // does throughout and as the Go port does. Under `char_as_number`
    // the value is a number rather than a string, so the code point
    // itself survives, which all three runtimes agree on.
    let replacement = Value::String("\u{FFFD}".into());
    assert_eq!(parse(r"'\u{D800}'").unwrap(), replacement);
    assert_eq!(parse(r#""\u{D800}""#).unwrap(), replacement);
    assert_eq!(parse(r#".@"\u{D800}""#).unwrap(), replacement);
    assert_eq!(
        json(&parse_with(r"'\u{D800}'", &with_options(true, None)).unwrap()),
        "55296"
    );
}
