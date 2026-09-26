# Divergences

TypeScript in [`ts/`](ts/) is canonical; the Go port in [`go/`](go/) and
the Rust port in [`rs/`](rs/) track it. This file records where a runtime
produces a **different result for the same input**, and why that
difference is allowed to stand.

Every row below was MEASURED, on 2026-09-22, by running the input in its
first column through all three implementations: `ts/src/zon.ts` compiled
against `@tabnas/parser` 0.10.0 and `@tabnas/jsonic` 0.6.7, the `go/`
package as it stands, and `tabnas-zon` 0.5.6 against the sibling
checkouts. Nothing here is inferred from reading the source.

Each entry names who owns the repair, and is pinned by a named test in
the port the entry belongs to. Those tests assert THIS PORT's side, so a
repair here fails as loudly as a regression; where the repair is owned
by TypeScript or by Go, nothing in this repository fails when it lands,
and the entry has to be re-measured by hand. Repairing one means
deleting its entry here and its test in the same change.

## Where these are pinned

This repository has no executable divergence register in
[`test/spec/`](test/spec/), because none of the divergences below can be
written as a row of one. A fixture row holds an input, one expected
value as JSON, and an options cell; the runners compare after a JSON
round trip (see [`test/AGENTS.md`](test/AGENTS.md)). A big integer, a
reported column, a lone surrogate and an out-of-range exponent have no
cell that says what differs, and the register would also need a `rust`
column in all three runners before a row could carry one. So each entry
is pinned by a test in the port it belongs to, named below.

The Rust tests are in
[`rs/tests/zon_test.rs`](rs/tests/zon_test.rs), under the heading
`the divergences DIVERGENCE.md records`.

## A big integer is an object in Rust

| input | TypeScript | Go | Rust |
|---|---|---|---|
| `36893488147419103231` | `36893488147419103231n` | `*big.Int` of the same value | `{"$big":"36893488147419103231"}` |
| `0x1ffffffffffffffff` | the same bigint | the same `*big.Int` | the same object |
| `9007199254740993` | `9007199254740993n` | the same `*big.Int` | `{"$big":"9007199254740993"}` |
| `18446744073709551616` | `18446744073709551616` | the same number | the same number |

**Deliberate, and Rust-only.** An integer literal whose exact value no
IEEE-754 double holds is a `bigint` in TypeScript and a `*big.Int` in
Go. The engine's `tabnas::Value` has no big-integer variant, so the Rust
port keeps the digits as `{ "$big": "<decimal digits>" }`
(`tabnas_zon::BIG_KEY`) rather than rounding them. That is the spelling
the zig reference corpora already use for such an integer, so both
corpora grade the Rust port on the exact value, not on a rounded one.
Every integer a double holds exactly, `2^64` included, is a plain number
in all three runtimes, which the last row measures.

Owned by the Rust port. The repair is a big-integer variant in the
engine's value type, which is an engine decision, not a zon one. Pinned
by `big_integers_keep_their_exact_value` and
`integers_that_fit_a_double_stay_numbers`.

## A lone surrogate folds to U+FFFD outside TypeScript

| input | TypeScript | Go | Rust |
|---|---|---|---|
| `'\u{D800}'` | `"\uD800"` | `U+FFFD` | `U+FFFD` |
| `'\u{D800}'` with `charAsNumber` | `55296` | `55296` | `55296` |

**Inherited, and not Rust-only.** A JavaScript string is a sequence of
UTF-16 code units and can hold an unpaired surrogate; a Go `string` and
a Rust `String` hold Unicode scalar values and cannot. Both ports
substitute U+FFFD, as the engine does throughout, and as
`@tabnas/parser`'s own `DIVERGENCE.md` records for the engine. The code
point itself survives under `charAsNumber`, where the value is a number
rather than a string, which the second row measures.

The one route to a string here is this plugin's character matcher,
which is handed the code point and asks for a one-character string:
`char::from_u32` has no answer in Rust and `string(rune(0xD800))` has
none in Go, so both give U+FFFD. A CHARACTER literal is an integer in
zig and DOES accept a surrogate (the pinned zig 0.16.0 oracle answers
`'\u{D800}'` with 55296), which is why the character matcher
deliberately keeps the wider bound.

Two more rows USED to sit here with the same three answers,
`"\u{D800}"` and `.@"\u{D800}"`. Neither was a divergence; both were
shared defects. Zig requires a `\u{...}` escape in a string or an
identifier to name a Unicode SCALAR value, and the oracle answers both
with "unicode escape does not correspond to a valid unicode scalar
value". The identifier was decoded by this plugin, which tested only
`cp <= 0x10FFFF`; the string never reached the plugin at all, being
lexed by jsonic's own string matcher with the relaxed-JSON escape set.
The plugin now lexes `"..."` itself (`zonString`, in all three
runtimes, with the engine's string lexer off), and every runtime
rejects both: `zon_ident` for the identifier and the engine's
`invalid_unicode` for the string, so there is nothing left to record.
`test/spec/strict.tsv` and `test/spec/strings.tsv` pin the rejections,
`test/spec/enums.tsv` and `strings.tsv` pin U+D7FF and U+E000 either
side of the block, and `test/strictness/inputs.txt` puts the boundary
in front of the oracle itself.

Owned by the engine ports. Pinned in Rust by
`a_lone_surrogate_folds_to_the_replacement_character`, which covers both
rows above AND asserts the rejection of the two rows that left.
`chars.tsv` carries the second row as a shared fixture, because all
three runtimes agree on it. The first row cannot be one, because a
fixture row holds one expected value for all three runtimes and that is
exactly where the three do not agree.

## Nesting past 127 levels is refused in Rust

| input | TypeScript | Go | Rust |
|---|---|---|---|
| `.{ .a = ` 127 times, then `1`, then `}` 127 times | parses | parses | parses |
| the same, 128 times | parses | parses | `ERROR:cancel` |
| `.{ ` 128 times, then `1`, then `}` 128 times | parses | parses | `ERROR:cancel` |

**Deliberate, and Rust-only.** The engine parses iteratively, but the
value it returns is walked with the call stack to display, to convert to
JSON, and to drop, one frame per level, so a deep enough document ends
the process with a stack overflow that no caller can catch. The budget
is `tabnas-jsonic`'s, which refuses the 128th open container with the
engine's `cancel` code, and `tabnas-jsonic`'s own `DIVERGENCE.md`
records it for the relaxed-JSON grammar this plugin layers on. ZON
inherits it unchanged: a ZON struct is jsonic's `map` rule and a ZON
tuple its `list` rule, so both are counted.

The limit is far above anything a `build.zig.zon` manifest reaches, and
the zig reference corpora are unaffected: the deepest document in
`test/zigzon/cases.json` nests 7 levels and in
`test/strictness/cases.json` 3.

Owned by `tabnas-jsonic`. Pinned by
`nesting_is_bounded_by_the_depth_budget`.

## The column after a multi-line string

| input | TypeScript | Go | Rust |
|---|---|---|---|
| `.{ .a = \\x` newline `, .b = }` | `unexpected` at 2:20 | `unexpected` at 2:20 | `unexpected` at 2:8 |

**Rust-only, and the engine cannot express the other answer.** A `\\`
string run is one token spanning several lines. The canonical runtime
advances the column by the token's whole length, newlines included
(`pnt.cI = startCI + (sI - startI)` in `ts/src/zon.ts`), so every later
column on that line is reported too far right; the offending line above
is eight characters long, and both other runtimes name column 20 on it.
The Rust lexer's public interface is `advance_chars`, which resets the
column at each newline and keeps the row and column honest. There is no
public way to set a column, so matching the canonical answer here would
take a change in the engine crate.

The row, the error code, the message and the quoted source line are the
same in all three. Only the column differs, and only after a multi-line
string.

Owned by the canonical TypeScript: the repair is to count the token's
rows there, after which this port matches with no change. Pinned by
`a_multi_line_string_leaves_the_column_honest`, which asserts the code,
the row and the column AND the message, the offending token and the
quoted source line, so that a formatter or source-excerpt regression
fails the pin rather than leaving this entry stale behind a green one.
The column is the one cell the entry leaves to prose, and it is the
cell the test asserts as 8.

## An exponent past what the runtime's integer parse holds

Every cell below was measured on 2026-09-21: the `zig` column through
the pinned zig 0.16.0 oracle the conformance corpora use, the Go column
twice, once as built for this host and once under `GOARCH=386`, which is
a real 32-bit build and not an inference from `strconv.IntSize`.
`ERROR` is `ERROR:zon_number` throughout.

| input | zig 0.16.0 | TypeScript | Go, 64-bit | Go, 32-bit | Rust |
|---|---|---|---|---|---|
| `1e400` | `Infinity` | `Infinity` | `Infinity` | `Infinity` | `Infinity` |
| `1e2147483647` | `Infinity` | `Infinity` | `Infinity` | `Infinity` | `Infinity` |
| `1e2147483648` | `Infinity` | `Infinity` | `Infinity` | `ERROR` | `Infinity` |
| `1e9223372036854775807` | `Infinity` | `Infinity` | `Infinity` | `ERROR` | `Infinity` |
| `1e9223372036854775808` | `Infinity` | `Infinity` | `ERROR` | `ERROR` | `Infinity` |
| `1e-9223372036854775807` | `0` | `0` | `0` | `ERROR` | `0` |
| `1e-9223372036854775808` | `0` | `0` | `ERROR` | `ERROR` | `0` |
| `1e9999999999999999999` | `Infinity` | `Infinity` | `ERROR` | `ERROR` | `Infinity` |
| `1e100000000000000000001` | `Infinity` | `Infinity` | `ERROR` | `ERROR` | `Infinity` |
| `1e999999999999999999998` | `Infinity` | `10` | `ERROR` | `ERROR` | `Infinity` |
| `1e999999999999999999999` | `Infinity` | `10` | `ERROR` | `ERROR` | `Infinity` |
| `1e-999999999999999999999` | `0` | `0.1` | `ERROR` | `ERROR` | `0` |
| `0x1p2147483647` | `Infinity` | `Infinity` | `Infinity` | `Infinity` | `Infinity` |
| `0x1p2147483648` | `Infinity` | `Infinity` | `Infinity` | `ERROR` | `Infinity` |
| `0x1p99999999999999999999` | `Infinity` | `Infinity` | `ERROR` | `ERROR` | `Infinity` |
| `0x1p-99999999999999999999` | `0` | `0` | `ERROR` | `ERROR` | `0` |

**All three differ, and RUST is the column that matches the reference
implementation on every row.** Each runtime fails differently, and each
boundary is a property of how it reads the exponent, not of the number
of digits:

- **Go** reads the exponent digits with `strconv.Atoi`, which parses
  into `int`, and rejects the literal when that overflows. The boundary
  is therefore the host word size, not a digit count: on a 64-bit host
  it is `math.MaxInt64`, so `1e9223372036854775807` is accepted and
  `1e9223372036854775808` is not, and on a 32-bit host it is
  `math.MaxInt32`, so it falls to `1e2147483647` and `1e2147483648`.
  The sign is applied after the parse (`expSign * n`), so a negative
  exponent has the same magnitude bound and not the extra step
  `math.MinInt` would allow. A rejected NEGATIVE exponent also quotes a
  shorter span, on either word size: the failure span is measured with
  `scanNumTokenEnd`, which stops at the sign, so the message names `1e`
  or `0x1p` rather than the whole literal, where a rejected positive
  exponent quotes the literal in full. An earlier version of this entry
  put the shorter span down to the 32-bit build; it was measured on
  both and belongs to the sign.
- **TypeScript** reads the exponent with `parseInt` and then spells the
  result back into the literal it hands to `parseFloat`. The boundary is
  where `String(n)` switches to exponent form, which is `1e21`, not a
  digit count either: `1e100000000000000000001` has 21 exponent digits
  and a value of `1e20`, and it is `Infinity`, while
  `1e999999999999999999998` has the same 21 digits, rounds to `1e21` as
  a double, and becomes the text `1e1e+21`, of which `parseFloat` reads
  the prefix `1e1`: hence `10`, and `0.1` for the negative exponent.
  The previous version of this entry said "21 digits or more" for both
  runtimes, which is true of neither.
- **Rust** saturates the exponent at a million either way, which is
  already past the double range, so the value is the infinity or the
  zero the magnitude calls for, on every row above.

Owned by the canonical TypeScript for the `10` and `0.1` rows, where the
behaviour is an artifact of the `parseInt` round trip rather than a
decision, and by the Go port for the rest, where the repair is a
`strconv.ParseInt(.., 64)` with saturation rather than rejection. Both
repairs move that runtime TOWARDS the Rust column and towards the zig
oracle, so neither costs this port anything.

Pinned by `an_absurd_decimal_exponent_saturates` in
[`rs/tests/zon_test.rs`](rs/tests/zon_test.rs), which READS the table
above out of this file and asserts the RUST column of every row it
finds, that the Rust cell is the zig cell on each, and that the table
holds sixteen rows, so a row added here is asserted without anyone
copying it and the table cannot shrink; and by
`TestExponentPastTheHostInteger` in [`go/zon_test.go`](go/zon_test.go),
which asserts the GO column, the `zon_number` code and the quoted span
of every rejected row, and picks its expectation from
`strconv.IntSize`, so it measures the host it runs on rather than
assuming a 64-bit one (`CGO_ENABLED=0 GOARCH=386 go test` runs the
32-bit column, and both were run on 2026-09-22). Nothing in this
repository fails when the TypeScript column is repaired; that one has
to be re-measured by hand.

## An option outside its declared type

| options | TypeScript | Go | Rust |
|---|---|---|---|
| `{"enumTag":"$e"}` | `{"k":{"$e":"red"}}` | the same | the same |
| `{"enumTag":""}` | `{"k":"red"}` | the same | the same |
| `{"enumTag":123}` | `{"k":{"123":"red"}}` | `{"k":"red"}` | `{"k":{"123":"red"}}` |
| `{"enumTag":1.5}` | `{"k":{"1.5":"red"}}` | `{"k":"red"}` | `{"k":{"1.5":"red"}}` |
| `{"enumTag":true}` | `{"k":{"true":"red"}}` | `{"k":"red"}` | `{"k":{"true":"red"}}` |
| `{"enumTag":10000000000000000}` | `{"k":{"10000000000000000":"red"}}` | `{"k":"red"}` | as TypeScript |
| `{"enumTag":1e21}` | `{"k":{"1e+21":"red"}}` | `{"k":"red"}` | as TypeScript |
| `{"enumTag":0.000001}` | `{"k":{"0.000001":"red"}}` | `{"k":"red"}` | as TypeScript |
| `{"enumTag":Infinity}` | `{"k":{"Infinity":"red"}}` | `{"k":"red"}` | as TypeScript |
| `{"enumTag":-Infinity}` | `{"k":{"-Infinity":"red"}}` | `{"k":"red"}` | as TypeScript |
| `{"enumTag":NaN}` | `{"k":"red"}` | the same | the same |
| `{"enumTag":[1,2]}` | `{"k":{"1,2":"red"}}` | `{"k":"red"}` | `{"k":{"[1,2]":"red"}}` |
| `{"enumTag":[]}` | `{"k":{"":"red"}}` | `{"k":"red"}` | `{"k":{"[]":"red"}}` |
| `{"enumTag":{"a":1}}` | `{"k":{"[object Object]":"red"}}` | `{"k":"red"}` | `{"k":{"{\"a\":1}":"red"}}` |
| `{"enumTag":{}}` | `{"k":{"[object Object]":"red"}}` | `{"k":"red"}` | `{"k":{"{}":"red"}}` |
| `{"enumTag":[Infinity]}` | `{"k":{"Infinity":"red"}}` | `{"k":"red"}` | `{"k":{"[null]":"red"}}` |

The input is `.{ .k = .red }` in every row.

| options | TypeScript | Go | Rust |
|---|---|---|---|
| `{"charAsNumber":true}` | `65` | the same | the same |
| `{"charAsNumber":1}` | `65` | `"A"` | `65` |
| `{"charAsNumber":"yes"}` | `65` | `"A"` | `65` |
| `{"charAsNumber":0}` | `"A"` | the same | the same |
| `{"charAsNumber":Infinity}` | `65` | `"A"` | `65` |
| `{"charAsNumber":-Infinity}` | `65` | `"A"` | `65` |
| `{"charAsNumber":NaN}` | `"A"` | the same | the same |
| `{"charAsNumber":""}` | `"A"` | the same | the same |
| `{"charAsNumber":[]}` | `65` | `"A"` | `65` |
| `{"charAsNumber":{}}` | `65` | `"A"` | `65` |
| `{"charAsNumber":true,"enumTag":false}` | `65` | the same | the same |

The input is `'A'` in every row of the second table. `Infinity` and
`NaN` have no JSON spelling, so those bags were supplied as host
numbers: a JavaScript number in TypeScript, a `float64` in Go, a
`tabnas::Value::Number` in Rust.

**Bounded, and outside the option's type.** `enumTag` is declared
`null | string` in all three runtimes. The canonical runtime uses
whatever it is given as a computed property key, which stringifies it;
the Rust port reads the option bag field by field and by JavaScript
truthiness, so a boolean names the same key there, and so does a NUMBER,
in every spelling `Number::toString` gives one: the digits written out
to `10000000000000000`, exponent form from `1e21` and from `1e-7`, and
`Infinity` or `-Infinity` for a non-finite one. The bag is read as the
engine value it is, never through `Value::to_json`, which renders a
non-finite number as `null` and would make a truthiness test see
something the canonical runtime never saw.

An ARRAY or an OBJECT keeps its JSON spelling in Rust rather than taking
`Array.prototype.toString` or `[object Object]`, which are JavaScript
object semantics rather than a value spelling. An EMPTY array or object
is truthy in JavaScript, so the tag is SET in both TypeScript and Rust
and only the key differs, where Go reads it as unset with the rest; a
non-finite element of such a container has no JSON spelling and becomes
`null` in the one Rust writes, which the last row of the first table
measures.

The same reading applies to `charAsNumber`, which is declared `boolean`.
An empty array or object is truthy in JavaScript, so `!!options.charAsNumber`
is true for both and `'A'` parses as `65` in TypeScript, and the Rust
`truthy` fallback agrees; the Go `toBool` type-asserts to `bool`, gets
false, and gives `"A"`. An empty STRING is falsy, so that row is one all
three agree on: it is in the table because a PRESENT BUT DEGENERATE
option is not an absent one, and the only way to know which side of the
line each degenerate value falls is to measure it.

The Go port asserts each option to its Go type instead: a non-string tag
and a non-boolean `charAsNumber` both read as unset, so the Go port
differs from the canonical runtime in thirteen rows of the first table
and six of the second. Those counts are the number of rows whose Go cell
is not "the same", counted off the tables above rather than carried over
from an earlier version of them; `the_divergence_register_row_counts_are_derived`
in [`rs/tests/zon_test.rs`](rs/tests/zon_test.rs) re-derives both from
this file and fails when a row is added without the sentence being
re-counted.

The `{"charAsNumber":true,"enumTag":false}` row of the second table is
the bag that motivated reading the bag field by field at all. Deserializing it as a whole failed at
`enumTag` and fell back to the DEFAULTS, so `'A'` parsed as `"A"` where
both other runtimes give `65`.

Owned by the Rust port for the array and object rows, and repairable
only by reimplementing `Array.prototype.toString` and
`Object.prototype.toString`, which no other behaviour needs. The
`charAsNumber` rows are owned by the Go port, where the repair is
JavaScript truthiness in `toBool`.

Pinned ON THE RUST SIDE ONLY by three tests, which between them assert
every row of both tables, the `charAsNumber` empty-string, empty-array
and empty-object rows included.
`an_option_bag_field_is_read_on_its_own` takes the string, boolean,
empty and container rows of both tables;
`a_numeric_tag_names_the_key_javascript_names` takes the numeric ones,
including the spellings either side of the `1e21` and `1e-7` boundaries;
and `a_non_finite_option_is_read_before_the_json_projection` takes the
`Infinity`, `-Infinity` and `NaN` rows of both tables. A fourth test,
`the_option_conversion_pair_is_lossless`, asserts that `to_value` and
`from_value` round-trip every valid typed value, the empty tag of the
second row included: the conversion keeps the string and `tag` applies
the canonical `|| null` filter at the point of use. The TypeScript and
Go columns above were measured, not pinned: no test in this repository
fails if either of those runtimes changes. What IS pinned about them is
the arithmetic: `the_divergence_register_row_counts_are_derived` reads
the two tables out of this file and re-derives "thirteen" and "six" from
the cells, so a row added without re-counting the sentence goes red.
