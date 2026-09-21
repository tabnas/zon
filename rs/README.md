# tabnas-zon (Rust)

The Zig Object Notation (ZON) grammar plugin for the
[`tabnas`](https://github.com/tabnas/parser) parsing engine, crate
`tabnas_zon`.

ZON is the data format of Zig `build.zig.zon` manifests, built on Zig
anonymous struct literals:

```zon
.{
    .name = "example",
    .version = "0.0.1",
    .dependencies = .{ .foo = .{ .url = "https://..." } },
    .paths = .{ "build.zig", "src" },
}
```

This is a jsonic plugin: it layers on the relaxed-JSON grammar of
[`tabnas-jsonic`](https://github.com/tabnas/jsonic) and reshapes it into
ZON. It switches the jsonic extensions off, remaps the fixed tokens (`.{`
opens both a struct and a tuple, `}` closes both, `=` separates a field
from its value), adds five lex matchers for Zig syntax (dot tokens,
multi-line strings, character literals, Zig number literals, and the
rejection of doc comments), and prepends the grammar alternates in
[`../zon-grammar.jsonic`](../zon-grammar.jsonic), which every runtime
embeds.

This is the Rust port of the canonical TypeScript implementation in
[`../ts`](../ts); the TypeScript version is authoritative and this crate
tracks it. The Go port is in [`../go`](../go).

## Use

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let value = tabnas_zon::parse(".{ .name = \"Alice\", .age = 30 }")?;
    assert_eq!(value.to_string(), r#"{"name":"Alice","age":30}"#);

    let tuple = tabnas_zon::parse(".{ 1, 2, 3 }")?;
    assert_eq!(tuple.to_string(), "[1,2,3]");
    Ok(())
}
```

`parse` reuses one shared instance. Build your own with `make`, or with
`make_with` and typed options, and reuse it: building the grammar
dominates a parse.

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parser = tabnas_zon::make();
    let manifest = parser.parse(
        ".{ .paths = .{ \"build.zig\", \"src\" }, .deps = .{ .foo = .{ .url = \"u\" } } }",
    )?;
    assert_eq!(
        manifest.to_string(),
        r#"{"paths":["build.zig","src"],"deps":{"foo":{"url":"u"}}}"#
    );

    // Two options shape values. `enum_tag` wraps an enum literal used as a
    // value, so `.red` and the string "red" stay distinguishable, and
    // `char_as_number` gives a character literal as its code point.
    let tagged = tabnas_zon::make_with(&tabnas_zon::ZonOptions {
        enum_tag: Some("$enum".to_string()),
        char_as_number: true,
    });
    assert_eq!(
        tagged.parse(".{ .kind = .red, .c = 'A' }")?.to_string(),
        r#"{"kind":{"$enum":"red"},"c":65}"#
    );
    Ok(())
}
```

To layer the plugin on an instance you configure yourself, install it on
a jsonic-enabled engine directly, or through `use_plugin` with an option
bag:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut parser = tabnas_jsonic::make();
    tabnas_zon::zon(&mut parser, &tabnas_zon::ZonOptions::default())?;
    assert_eq!(parser.parse(".{ .a = .{ .b = 1 } }")?.to_string(), r#"{"a":{"b":1}}"#);

    let mut derived = tabnas_jsonic::make();
    let options = tabnas_zon::ZonOptions { char_as_number: true, ..Default::default() };
    derived.use_plugin(tabnas_zon::plugin(), Some(options.to_value()))?;
    assert_eq!(derived.parse("'\\n'")?.to_string(), "10");
    Ok(())
}
```

Parse errors are the engine's `TabnasError`, re-exported as `ZonError`,
with `code`, `row`, `col` and a report that shows the offending source.
The five codes this plugin declares are `zon_number`, `zon_ident`,
`zon_char`, `zon_doc_comment` and `zon_dup_field`:

```rust
fn main() {
    let error = tabnas_zon::parse(".{ .a = 1, .a = 2 }").unwrap_err();
    assert_eq!(error.code, "zon_dup_field");
    assert_eq!(tabnas_zon::parse("0X2A").unwrap_err().code, "zon_number");
}
```

## Install

Neither the engine nor the grammars it builds on are published to a
registry, so all of them are consumed as **sibling checkouts**, the
standard tabnas development model. Clone
`https://github.com/tabnas/parser`, `https://github.com/tabnas/json` and
`https://github.com/tabnas/jsonic` next to this repository and point at
them:

```toml
[dependencies]
tabnas-zon = { path = "../zon/rs" }
tabnas-jsonic = { path = "../jsonic/rs" }
tabnas = { path = "../parser/rs" }
```

All three entries are needed. A crate's dependencies are not passed on to
its dependents, so `tabnas-zon` alone does not put `tabnas` or
`tabnas-jsonic` in your extern prelude, and the examples above that name
`tabnas_jsonic::make` would not resolve. Only `ZonError` is re-exported.
The `json` checkout is needed because `tabnas-jsonic` takes it by path.
The test suite additionally needs `https://github.com/tabnas/support`
beside the repository, for the shared fixture runner.

## Differences from the canonical TypeScript

Every verdict and every parse result a ZON document can express is the
TypeScript one: the shared fixtures in [`../test/spec`](../test/spec)
and the two zig reference corpora hold all three runtimes to it. What
differs is the shape of the API, the spelling of values the host
language has no type for, and the handful of inputs measured in
[`../DIVERGENCE.md`](../DIVERGENCE.md):

- **Options are a struct.** `ZonOptions` has `char_as_number` and
  `enum_tag` as typed fields; `to_value` and `from_value` convert to and
  from the option bag `use_plugin` takes, with the same defaults.
  `from_value` reads each field on its own and by JavaScript truthiness,
  as the canonical plugin does, and reads the bag as the engine value it
  is rather than through `to_json`, which renders a non-finite number as
  `null`. The conversion is lossless: an empty `enum_tag` survives the
  round trip, and `tag` treats it as unset at the point of use, exactly
  as `options.enumTag || null` does. An array or an object as
  `enumTag`, outside the option's declared type in every runtime, keeps
  its JSON spelling here rather than the JavaScript one.
- **A big integer is an object.** An integer literal whose exact value no
  IEEE-754 double holds is a `bigint` in TypeScript and a `*big.Int` in
  Go. The engine's `Value` has no such variant, so this crate returns
  `{ "$big": "<decimal digits>" }` (`tabnas_zon::BIG_KEY`) rather than
  rounding, the spelling the zig reference corpora already use for one.
  Every integer a double holds exactly, `2^64` included, is a plain
  number.
- **`inf`, `-inf` and `nan` are `f64` values**, as in both other
  runtimes; `to_json` renders them as `null`, which is what a JSON
  round-trip does to them everywhere.
- **Key order is document order**, and a struct is an `IndexMap`, so a
  parsed manifest prints its fields in the order they were written.
- **The duplicate-field guard hands back an error token**, as the
  TypeScript hook does; the Go port signals the same code through the
  parse context. The result is the same `zon_dup_field` error at the
  same position.
- **Lone surrogates fold to U+FFFD**, and the regular expression dialect
  is the `regex` crate's. Both come from the engine, and both are
  recorded there.
- **A document nested more than 127 containers deep fails** with the
  engine's `cancel` code. The engine walks a value with the call stack
  to display, convert or drop it, so an unbounded one ends the process
  rather than failing; the budget is the one `tabnas-jsonic` already
  applies, and TypeScript and Go set no limit. A `build.zig.zon`
  manifest comes nowhere near it, and the deepest document in either zig
  corpus nests 7 levels.
- **The column after a multi-line string is the true column.** The
  canonical runtime advances the column of a `\\` string run by the
  token's whole length, newlines included, so it names a column too far
  right for a later error on that line; this port counts the rows the
  token spans, and no other position differs between the runtimes.
- **A decimal exponent of 21 digits or more saturates** to an infinity
  or a zero, where the canonical runtime reads only the prefix of the
  literal it rebuilds and the Go port rejects it.

## Build and test

The engine, the JSON core, the relaxed-JSON grammar and the fixture
runner are path dependencies on sibling checkouts, so there is nothing to
fetch by hand:

```bash
cargo test --all-targets && cargo test --doc
```

Or, from the repository root, `make test-rs`. For what CI would say,
including formatting and the lockfile check, run `ci/rust/run.sh`.

The suite runs every shared `../test/spec/*.tsv` fixture, the same files
the TypeScript and Go suites run, building a fresh parser for each row's
`opts` column. It also grades the two zig reference corpora
(`test/zigzon/cases.json` and `test/strictness/cases.json`), generating
them with `scripts/fetch-zigzon.sh` first when they are absent, exactly
as the other two runtimes do: a corpus that is still missing afterwards
fails the suite, it never skips. Beside them are the in-language tests
for what a fixture cannot express: big integers, infinities, NaN and
negative zero, the error messages, plugin layering and re-use, the
embedded grammar against its source, the shared default parser under
threads, and that `parse` reuses its instance.

## License

MIT.
