# Divergences

TypeScript in [`ts/`](ts/) is canonical; the Go port in [`go/`](go/) and
the Rust port in [`rs/`](rs/) track it. This file records where a runtime
produces a **different result for the same input**, and why that
difference is allowed to stand.

Every row below was MEASURED, on 2026-09-21, by running the input in its
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
| `.@"\u{D800}"` | `"\uD800"` | `U+FFFD` | `U+FFFD` |
| `"\u{D800}"` | `"\uD800"` | `U+FFFD` | `U+FFFD` |
| `'\u{D800}'` with `charAsNumber` | `55296` | `55296` | `55296` |

**Inherited, and not Rust-only.** A JavaScript string is a sequence of
UTF-16 code units and can hold an unpaired surrogate; a Go `string` and
a Rust `String` hold Unicode scalar values and cannot. Both ports
substitute U+FFFD, as the engine does throughout, and as
`@tabnas/parser`'s own `DIVERGENCE.md` records for the engine. The code
point itself survives under `charAsNumber`, where the value is a number
rather than a string, which the last row measures.

Owned by the engine ports. Pinned in Rust by
`a_lone_surrogate_folds_to_the_replacement_character`, which covers all
four rows. No shared fixture carries a surrogate: a fixture row holds
one expected value for all three runtimes, and these rows are exactly
where the three do not agree.

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
`a_multi_line_string_leaves_the_column_honest`.

## A decimal exponent of 21 digits or more

| input | TypeScript | Go | Rust |
|---|---|---|---|
| `1e999999999999999999999` | `10` | `ERROR:zon_number` | `Infinity` |
| `1e-999999999999999999999` | `0.1` | `ERROR:zon_number` | `0` |
| `1e99999999999999999999` | `Infinity` | `ERROR:zon_number` | `Infinity` |
| `1e400` | `Infinity` | `Infinity` | `Infinity` |
| `0x1p-99999999999999999999` | `0` | `ERROR:zon_number` | `0` |

**All three differ, and none matches another.** The canonical runtime
reads the exponent with `parseInt` and then spells it back into the
literal it hands to `parseFloat`. Past `1e21` that spelling is itself in
exponent form, so `1e999999999999999999999` becomes the text `1e1e+21`,
of which `parseFloat` reads the prefix `1e1`: hence `10`, and `0.1` for
the negative exponent. The Go port rejects any exponent that overflows
its integer parse. The Rust port saturates the exponent at a million
either way, which is already past the double range, so the value is the
infinity or the zero the magnitude calls for.

A 20-digit exponent, the third row, is inside the saturation and agrees
with the canonical runtime exactly, as do ordinary out-of-range
exponents. The hexadecimal `p` form saturates the same way and agrees
with the canonical runtime at both ends, where the Go port rejects it.

Owned by the canonical TypeScript, where the behaviour is an artifact of
the `parseInt` round trip rather than a decision. Pinned by
`an_absurd_decimal_exponent_saturates`.

## An option outside its declared type

| options | TypeScript | Go | Rust |
|---|---|---|---|
| `{"enumTag":"$e"}` | `{"k":{"$e":"red"}}` | the same | the same |
| `{"enumTag":123}` | `{"k":{"123":"red"}}` | `{"k":"red"}` | `{"k":{"123":"red"}}` |
| `{"enumTag":1.5}` | `{"k":{"1.5":"red"}}` | `{"k":"red"}` | `{"k":{"1.5":"red"}}` |
| `{"enumTag":true}` | `{"k":{"true":"red"}}` | `{"k":"red"}` | `{"k":{"true":"red"}}` |
| `{"enumTag":[1,2]}` | `{"k":{"1,2":"red"}}` | `{"k":"red"}` | `{"k":{"[1,2]":"red"}}` |
| `{"enumTag":{"a":1}}` | `{"k":{"[object Object]":"red"}}` | `{"k":"red"}` | `{"k":{"{\"a\":1}":"red"}}` |

The input is `.{ .k = .red }` in every row.

| options | TypeScript | Go | Rust |
|---|---|---|---|
| `{"charAsNumber":true}` | `65` | the same | the same |
| `{"charAsNumber":1}` | `65` | `"A"` | `65` |
| `{"charAsNumber":"yes"}` | `65` | `"A"` | `65` |
| `{"charAsNumber":0}` | `"A"` | the same | the same |
| `{"charAsNumber":true,"enumTag":false}` | `65` | the same | the same |

The input is `'A'` in every row of the second table.

**Bounded, and outside the option's type.** `enumTag` is declared
`null | string` in all three runtimes. The canonical runtime uses
whatever it is given as a computed property key, which stringifies it;
the Rust port reads the option bag field by field and by JavaScript
truthiness, so a number and a boolean name the same key there, measured
above. An ARRAY or an OBJECT keeps its JSON spelling in Rust rather than
taking `Array.prototype.toString` or `[object Object]`, which are
JavaScript object semantics rather than a value spelling. The Go port
asserts each option to its Go type instead: a non-string tag and a
non-boolean `charAsNumber` both read as unset, so the Go port differs
from the canonical runtime in four rows of the first table and two of
the second.

The last row of the second table is the bag that motivated reading the
bag field by field at all. Deserializing it as a whole failed at
`enumTag` and fell back to the DEFAULTS, so `'A'` parsed as `"A"` where
both other runtimes give `65`.

Owned by the Rust port for the array and object rows, and repairable
only by reimplementing `Array.prototype.toString` and
`Object.prototype.toString`, which no other behaviour needs. The
`charAsNumber` rows are owned by the Go port, where the repair is
JavaScript truthiness in `toBool`.

Pinned by `an_option_bag_field_is_read_on_its_own`, which asserts every
row of both tables ON THE RUST SIDE ONLY. The TypeScript and Go columns
above were measured, not pinned: no test in this repository fails if
either of those runtimes changes.
