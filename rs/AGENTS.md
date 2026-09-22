# Agents Guide: rs/

The Rust port of the canonical TypeScript in [`../ts`](../ts). Read
[`../AGENTS.md`](../AGENTS.md) first: it holds the cross-runtime rules,
and this file only covers what is specific to this crate.

## Layout

| Path | |
|---|---|
| `src/lib.rs` | the embedded grammar text, the option overrides document, `ZonOptions`, the two lifecycle hooks, `zon`, `plugin`, `make`, `make_with`, `parse`, `parse_with`, and the unit tests that hold the overrides to `ts/src/zon.ts` |
| `src/lex.rs` | the six lex matchers (`zonDot`, `zonMultiString`, `zonChar`, `zonNumber`, `zonDocComment`, `zonString`) and the Zig string scanner they share with the `.@"..."` form |
| `src/number.rs` | the Zig number-literal scanner and the small `BigUint` the exactness rule needs |
| `tests/parity_test.rs` | every `../test/spec/*.tsv` fixture through `tabnas_support::Runner::new_with_row`, a fresh parser per row from its `opts` column |
| `tests/zigzon_test.rs` | the two zig reference corpora, fetched first when absent; fails, never skips, when a corpus is missing |
| `tests/debug_model_test.rs` | the grammar composed with `tabnas-debug`, and the structured model it reads: the Rust half of `ts/test/debug-model.test.ts` |
| `tests/perf_test.rs` | `parse()` reuses its instance; reuse beats rebuild-per-parse |
| `tests/zon_test.rs` | in-language behaviour: the `go/zon_test.go` cases, the values with no JSON spelling, error codes and messages, the API, the embedded grammar, threads |
| `tests/version_test.rs` | Cargo.toml == `VERSION` == ts/package.json |
| `tests/common/mod.rs` | shared helpers: repo root, spec dir, value and failure conversion, the `opts` reader |
| `README.md` | the crate front page, prose-gated; its `rust` fences are doctests of this crate (see below) |

Crate `tabnas-zon`, library `tabnas_zon`. The engine (`tabnas`), the
relaxed-JSON grammar (`tabnas-jsonic`, which itself takes `tabnas-json`
by path), the fixture runner (`tabnas-support`, dev only) and the
introspection plugin (`tabnas-debug`, dev only) are **path dependencies
on sibling checkouts** (`../../parser/rs`, `../../jsonic/rs`,
`../../json/rs`, `../../support/rs`, `../../debug/rs`). None is
published.

```bash
cargo build --all-targets
cargo test --all-targets && cargo test --doc
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt
```

`make test-rs` from the repository root is the fast loop; `ci/rust/run.sh`
is the full gate and adds `fmt --check`, the lockfile check and the MSRV
pin.

## The grammar is embedded text, parsed at run time

`ts/embed-grammar.js` copies `zon-grammar.jsonic` VERBATIM between the
`BEGIN/END EMBEDDED` markers of all three runtimes: a template literal in
TypeScript, a raw string in Go, and `pub const GRAMMAR_TEXT` (an `r##"`
raw string) here. Never hand-edit between the markers; edit the `.jsonic`
file and run `node ts/embed-grammar.js`.

Like the Go port, this crate parses the text at run time with
`tabnas_jsonic::parse`, once per process (`grammar_document`), and
attaches the option overrides as the document's `options` key so the
plugin applies rules and options atomically, as `grammarDef.options`
does in TypeScript. jsonic's numbers are doubles, so `b: 2` arrives as
`2.0`; `integral_numbers` puts whole numbers back into integer form
before `GrammarSpec::from_value` reads the `b` field.
`the_embedded_grammar_is_the_authored_one` in `zon_test.rs` fails when
the constant and the file on disk differ.

## The lifecycle hooks and their names

- **`@pair-bc/prepend`**: the duplicate-field guard. It must run before
  jsonic's own `@pair-bc/append`, which performs the assignment (last one
  wins) and hides the collision by `@pair-ac`. It is registered with
  `state_action_with_next_ref` so it can hand back the key's token marked
  `zon_dup_field`, as the TypeScript hook returns it. (Go cannot return a
  token from a state action and uses `ctx.ParseErr` instead.)
- **`@val-ac/prepend`**: the `enumTag` rewrap. jsonic takes the `val`
  before-close phase with `/replace`, which suppresses any `/prepend` on
  it, so the rewrap runs after close, as in TypeScript. TypeScript names
  it bare `@val-ac`; here it carries `/prepend` because the engine wires a
  bare `@val-ac` only when no `@val-ac/append` is registered, and jsonic
  registers one. Order within the phase does not matter: jsonic's
  after-close leaves an enum token's node alone.
- The rewrap ASSIGNS the rule's node (`rule.node = Rc::new(RefCell::new(..))`).
  A pushed or replaced rule shares its parent's cell, so writing through
  `borrow_mut()` would overwrite the parent's node too.

## The lex matchers

All six are `imperative_lex_match_ref` registrations named from the
document's `options.lex.match` (`"make": "@zonDot"` and so on), with the
canonical orders: 1e5 to 1.5e5, below the engine's first built-in band,
so `zonDot` owns the `.` prefix ahead of the fixed-token matcher,
`zonDocComment` sees `//!` and `///` before the comment matcher eats them,
and `zonString` owns `"` (the engine's string lexer is off besides,
`string.lex: false`, so nothing else produces an `#ST` from a quote).

`scan_zig_string` is the one Zig string scanner, shared by `zonString`
and the `.@"..."` identifier form. Its escape set is Zig's and no wider,
a `\u{...}` must name a Unicode SCALAR value (the surrogate block is
refused, as the pinned oracle refuses it), a `\xNN` run is bytes decoded
as UTF-8 once the run ends (`String::from_utf8_lossy`, one U+FFFD per
maximal subpart, which is also what `TextDecoder` and the Go
`lossyUTF8` give), and a fault carries the ENGINE's error code
(`unterminated_string`, `unprintable`, `invalid_unicode`,
`invalid_ascii`, `unexpected`) so `test/spec/strings.tsv` can pin the
code in all three runtimes. A CHARACTER literal is an integer in Zig and
does accept a surrogate, so `char_matcher` deliberately keeps the wider
`<= 0x10FFFF` bound and does not share the scanner.

They work on `lexer.remaining()` by byte index and only ever slice at
ASCII positions or whole decoded characters, so a byte index is always a
character boundary where it is used. The cursor moves through
`lexer.advance_chars`, which keeps `row`/`col` honest across the newlines
a `. \n foo` or a `\\` run may span, the bookkeeping the TypeScript
`advance` helpers do by hand. A bad token spans the whole offending
literal (`bad_span`), clamped the way the Go `zonBad` clamps, so the
message can quote it.

`zonChar` bakes `charAsNumber` into its closure, as the TypeScript
`buildZonCharMatcher(charAsNumber)` does, so the option is read once at
install time.

## The option overrides

`options_document()` is the `grammarDef.options` of the canonical
plugin, attached to the grammar document so rules and options apply
atomically. AGENTS.md rule 4 requires the overrides to exist in all
three runtimes and stay in step, and nothing measured that until
`the_option_override_surface_is_the_canonical_one` (a unit test in
`src/lib.rs`): it reads the literal out of `ts/src/zon.ts` and compares
the top-level keys, the error catalogue, the fixed-token remap and the
six matcher names WITH their orders, so an override added or renamed
there without a counterpart here goes red rather than drifting.

Two differences are deliberate, and
`the_two_override_differences_are_the_documented_ones` asserts both, so
neither survives as prose alone:

- `value.def` restates `true` and `false` and leaves `null` to the
  engine's own definition. A serialized `"val": null` reads as "no
  value" rather than as the null value, so restating it would switch the
  keyword off.
- There is no `config` block. The canonical plugin also calls
  `tn.options` with a `config.modify` hook that hangs human token
  descriptions off `cfg.tokenDesc`, which `@tabnas/railroad` reads for a
  diagram legend. This engine's config has no such field, and the Rust
  railroad crate takes the descriptions from its own
  `ExtractOptions::token_desc`, so there is nothing for the plugin to
  attach. `README.md` records it as a difference; no parse result
  depends on it.

## Numbers

`number::scan` is a line-for-line port of `scanZonNumber`. The one
Rust-specific piece is `BigUint`: the engine's `Value` has no big-integer
variant, so the digits are held long enough to decide whether a double
is exact (at most 53 significant bits, within the exponent range) and,
when it is not, to render the decimal string the `{ "$big": digits }`
object carries (`crate::BIG_KEY`). That object is the spelling the zig
reference corpora already use, which is what lets `zigzon_test.rs`
compare values directly. A hex float mantissa is rounded half to even,
exactly `Number(BigInt(...))`.

`-0` (the integer) is rejected, `-0.0` is a negative zero `f64`, `inf` /
`-inf` / `nan` are the non-finite `f64`s, and `-nan` is rejected, all as
in TypeScript.

## Options

`ZonOptions` is a serde struct (`camelCase`), so `to_value` / `from_value`
round-trip through the engine's option bag with the canonical field names
`charAsNumber` and `enumTag`. `plugin()` carries the defaults, and the
engine merges a caller's bag over them, the `UseDefaults` of the Go port.
An empty `enumTag` means unset at the POINT OF USE, as in Go: `tag()`
performs that test, and the conversion pair does not.

`from_value` reads each field ON ITS OWN, and by JavaScript truthiness,
because that is what `!!options.charAsNumber` and `options.enumTag ||
null` mean in `ts/src/zon.ts`. Deserializing the bag as a whole let one
ill-typed field discard a well-typed one: `{"charAsNumber": true,
"enumTag": false}` failed at `enumTag` and fell back to the DEFAULTS, so
`'A'` parsed as `"A"` where both other runtimes give `65`. Keep it field
by field; `an_option_bag_field_is_read_on_its_own` pins it.

Two rules hold that conversion together, and both were broken once:

- **The conversion layer is LOSSLESS; the semantic filter lives at the
  point of use.** `ZonOptions { enum_tag: Some(String::new()) }` is a
  valid typed value, so `from_value(&options.to_value())` has to return
  it. Filtering the empty string during conversion ALSO threw away what
  a caller had recorded, and `tag()` was already performing the same
  test one layer up. `the_option_conversion_pair_is_lossless` pins it.
- **Read the bag as the engine `Value`, never through `to_json()`.**
  That projection renders a non-finite number as `null`, and `null` is
  falsy where `Infinity` is not, so any truthiness or type test taken
  after it sees something the canonical runtime never saw: a
  `charAsNumber` of `Infinity` read as false, and an `enumTag` of
  `Infinity` read as unset where JavaScript names the key `Infinity`.
  `a_non_finite_option_is_read_before_the_json_projection` pins it.

A non-string tag becomes the key a computed property key would name, so
a number spells itself with `number::js_number_to_string`, which is
ECMA-262 6.1.6.1.20 and not Rust's shortest float form: it writes
`10000000000000000` out in full and switches to exponent form only at
`1e21` and `1e-7`. The function is ported from `js_number_to_string` in
the csv Rust port; do not write another.
`a_numeric_tag_names_the_key_javascript_names` pins it.

The plugin guards re-invocation with the `zon-init` decoration, set only after the install succeeded so a failed call can be retried (the Go
port's guard), because a derived instance re-applies plugins.

## What a fixture cannot hold

Big integers, infinities, NaN and the `-0` / `0` distinction have no JSON
spelling and live in `zon_test.rs`, mirrored case for case with
`go/zon_test.go` and `ts/test/zon.test.ts`. The parity runner flattens
through `to_json`, which is the `jsonFlatten` of the Go runner.

So do the divergences: `../DIVERGENCE.md` holds every input on which
this port and the canonical TypeScript are known to differ, measured
three ways, and the tests under `the divergences DIVERGENCE.md records`
in `zon_test.rs` pin them so a REPAIR IN THIS PORT fails as loudly as a
regression. Those tests assert the RUST side only: the TypeScript and Go
columns of each table are measurements, so a repair in either of those
runtimes leaves an entry stale without failing anything here, and
re-measuring is the reviewer's job. Repairing one means deleting its
entry and its test in the same change. Finding a new one means measuring
it three ways and adding both; never widen a parity claim past what a
test measures.

## The corpora

`zigzon_test.rs` runs `scripts/fetch-zigzon.sh` (through `bash`) when
either `cases.json` is missing and the host is one the script has a
pinned zig toolchain for, then grades both corpora with the pinned census
(184/44 and 68/95). A missing corpus FAILS the test; the only skip is the
platform one, and it names the platform. Do not widen it.

## The docs are gated

`README.md` is in the published set: no em dashes in prose, no first
person singular, no links to any `AGENTS.md`, no project history. This
file is internal and may be blunt.

## The README is doctested

`src/lib.rs` includes `README.md` as rustdoc under `#[cfg(doctest)]`, so
every `rust` fence in it runs on `cargo test --doc` (they show up as
`readme_examples (line N)`). rustdoc runs each fence as written, so a
fence must be a complete program: wrap it in
`fn main() -> Result<(), Box<dyn std::error::Error>> { ... Ok(()) }`
rather than using `?` at the top level, and never use hidden `# ` lines,
which render as garbage on GitHub. The `toml`, `bash` and `zon` fences
are not run.
