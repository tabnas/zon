# How-to guide (Go)

Short, task-focused recipes. Each is self-contained and assumes you
have the module installed (see the [tutorial](tutorial.md) for the
basics). For the full API, every option, and the complete syntax,
follow the links into the [reference](reference.md).

```go
import zon "github.com/tabnas/zon/go"
```

## Parse a single string

`zon.Parse` is the simplest entry point — pass source, get a value and
an error:

```go
result, err := zon.Parse(`.{ .a = 1, .b = 2 }`)
// result: map[string]any{"a": float64(1), "b": float64(2)}
```

The no-options path reuses a single cached parser instance internally,
so repeated `zon.Parse(src)` calls do not rebuild the engine each time.
It is safe for concurrent use.

## Parse a realistic build.zig.zon

A ZON manifest mixes named struct fields with tuple-style `paths`
lists and allows trailing commas and `//` line comments:

```go
src := `.{
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
    },
}`

result, err := zon.Parse(src)
// result: map[string]any{
//   "name":                "example",
//   "version":             "0.0.1",
//   "minimum_zig_version":  "0.14.0",
//   "dependencies": map[string]any{
//     "foo": map[string]any{
//       "url": "https://example.com/foo.tar.gz", "hash": "1220deadbeef",
//     },
//   },
//   "paths": []any{"build.zig", "src"},
// }
```

## Parse numbers in every ZON base

Numbers accept decimal, hex, octal, binary, floats, and `_` digit
separators. Every number is a `float64`:

```go
zon.Parse("0x2a")      // float64(42)
zon.Parse("0o52")      // float64(42)
zon.Parse("0b101010")  // float64(42)
zon.Parse("1_000_000") // float64(1000000)
zon.Parse("3.14")      // float64(3.14)
```

## Parse character literals as code points

By default Zig char literals (`'A'`, `'\n'`, `'\u{1F600}'`) parse as
one-character strings. Set `CharAsNumber` to receive numeric code
points (as `float64`) instead:

```go
charAsNum := true
result, err := zon.Parse(`'A'`, zon.ZonOptions{CharAsNumber: &charAsNum})
// result: float64(65)
```

## Tag enum literals to tell them apart from strings

Without options, an enum-literal value like `.red` becomes the plain
string `"red"` — indistinguishable from `"red"` in the parsed tree.
Set `EnumTag` to wrap each enum value in a one-key map so you can tell
which was which:

```go
result, err := zon.Parse(
    `.{ .kind = .red, .label = "red" }`,
    zon.ZonOptions{EnumTag: "$enum"},
)
// result: map[string]any{
//   "kind":  map[string]any{"$enum": "red"},
//   "label": "red",
// }
```

## Read multi-line Zig strings

Consecutive lines prefixed with `\\` become a single string, joined
with `\n` (the `\\` prefix is stripped from each line):

```go
src := ".{\n" +
    "    .description =\n" +
    "        \\\\first line\n" +
    "        \\\\second line\n" +
    "    ,\n" +
    "}"

result, err := zon.Parse(src)
// result: map[string]any{"description": "first line\nsecond line"}
```

## Reuse a parser for many inputs (with options)

`zon.Parse(src, opts)` builds a dedicated instance per call when you
pass options, since the configuration differs per call. For a hot loop
with fixed options, build one instance with `MakeJsonic` and reuse it:

```go
j := zon.MakeJsonic(zon.ZonOptions{EnumTag: "$enum"})
for _, src := range inputs {
    result, err := j.Parse(src)
    _ = result
    _ = err
}
```

(With *no* options, plain `zon.Parse(src)` already reuses a cached
instance, so you do not need `MakeJsonic` for that case.)

## Handle a parse error

ZON deliberately rejects non-ZON input — a bare `{` opener, for
instance. The parse never panics; it returns an `error`:

```go
result, err := zon.Parse(`{ a = 1 }`) // not ZON: bare { is rejected
if err != nil {
    // handle the syntax error; result is nil
}
```

## Re-enable strict JSON while the plugin is loaded

Every grammar alternate the plugin adds carries the group tag `zon`.
To switch those alts off — restoring the plain jsonic grammar while
the plugin stays registered — exclude that tag through the underlying
jsonic instance:

```go
import (
    jsonic "github.com/tabnas/jsonic/go"
    zon "github.com/tabnas/zon/go"
)

j := jsonic.Make()
j.UseDefaults(zon.Zon, zon.Defaults)
j.SetOptions(jsonic.Options{Rule: &jsonic.RuleOptions{Exclude: "zon"}})
```

This is rarely useful — you would normally just not load the plugin —
but it is the supported way to peel the ZON layer back off.
