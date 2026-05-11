# ZON plugin for Jsonic (TypeScript)

A Jsonic syntax plugin that parses
[Zig Object Notation (ZON)](https://ziglang.org/documentation/master/#ZON)
into JavaScript values, with support for anonymous struct literals,
tuples, enum literals, numeric bases, character literals, multi-line
strings, and trailing commas.

```bash
npm install @jsonic/zon
```

Requires `jsonic` >= 2 as a peer dependency.


## Tutorials

### Parse a basic ZON document

Register the plugin and parse a top-level struct literal:

```typescript
import { Jsonic } from 'jsonic'
import { Zon } from '@jsonic/zon'

const j = Jsonic.make().use(Zon)

j('.{ .name = "Alice", .age = 30 }')
// { name: 'Alice', age: 30 }

j('.{ 1, 2, 3 }')
// [1, 2, 3]
```

### Parse a realistic build.zig.zon

ZON files typically have nested structs mixed with tuple-style
`paths` lists:

```typescript
import { Jsonic } from 'jsonic'
import { Zon } from '@jsonic/zon'

const j = Jsonic.make().use(Zon)

j(`.{
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
}`)
// {
//   name: 'example',
//   version: '0.0.1',
//   minimum_zig_version: '0.14.0',
//   dependencies: { foo: { url: '...', hash: '1220deadbeef' } },
//   paths: ['build.zig', 'src'],
// }
```

### Parse numbers in every ZON base

ZON numbers accept hex, octal, binary, and `_` separators:

```typescript
const j = Jsonic.make().use(Zon)

j('0x2a')      // 42
j('0o52')      // 42
j('0b101010')  // 42
j('1_000_000') // 1000000
j('3.14')      // 3.14
```


## How-to guides

### Parse character literals as code points

By default Zig char literals (`'A'`, `'\n'`, `'\u{1F600}'`) parse as
one-character strings. Set `charAsNumber: true` to receive numeric
code points instead:

```typescript
const j = Jsonic.make().use(Zon, { charAsNumber: true })

j("'A'")         // 65
j("'\\n'")       // 10
j("'\\u{1F600}'") // 128512
```

### Tag enum literals to distinguish them from strings

Without options, an enum literal value like `.red` becomes the plain
string `'red'`. If you need to tell it apart from an ordinary string
in the parsed tree, set `enumTag`:

```typescript
const j = Jsonic.make().use(Zon, { enumTag: '$enum' })

j('.{ .kind = .red, .label = "red" }')
// { kind: { $enum: 'red' }, label: 'red' }
```

### Read multi-line Zig strings

Consecutive lines prefixed with `\\` become a single string joined by
`\n`:

```typescript
const j = Jsonic.make().use(Zon)

j(`.{
  .description =
    \\\\first line
    \\\\second line
  ,
}`)
// { description: 'first line\nsecond line' }
```

### Reject extra alternates contributed by this plugin

Every grammar alternate added by the plugin carries the group tag
`zon`. To re-enable strict JSON while the plugin is loaded (rarely
useful, but supported), exclude that tag:

```typescript
const j = Jsonic.make().use(Zon).options({
  rule: { exclude: 'zon' },
})
```


## Explanation

### How ZON parsing works

ZON is not a superset of JSON — it uses a distinct opening syntax
(`.{`), a different key/value separator (`=`), and key identifiers
prefixed with `.`. The plugin reshapes Jsonic into a ZON parser by
combining four mechanisms:

1. **Custom lex matchers** for the `.`-prefixed tokens:

   - `.{` peeks ahead and emits `#OB` (struct/map) if followed by
     `<ws>.ident<ws>=` or `#OS` (tuple/list) otherwise. This resolves
     the ambiguity at lex time so only two-token grammar lookahead is
     needed.
   - `.identifier` emits `#TX` whose `val` is the identifier (dot
     stripped) and whose `use.zonEnum` flag marks it for optional
     enum-tag wrapping.
   - `\\`-prefixed multi-line strings emit a single `#ST` token with
     the joined content.
   - Character literals (`'x'`, `'\n'`, `'\xNN'`, `'\u{...}'`) emit a
     `#NR` token whose value is either the one-char string or the
     numeric code point (controlled by `charAsNumber`).

2. **Token remapping**: `#CL` is rebound from `:` to `=`; `#OB`,
   `#OS`, and `#CS` drop their default char mappings so stray `{`,
   `[`, or `]` in source produce a syntax error rather than silently
   opening a map or list.

3. **Key-set restriction**: the `KEY` token set is narrowed to `#TX`
   alone so only identifiers (not numbers or strings) can appear on
   the left of `=`.

4. **Grammar overlay**: small alts prepended to `val`, `list`,
   `elem`, and `pair` swap the list terminator from `#CS` to `#CB`
   and accept trailing commas before `}`.

All four are applied atomically through the `GrammarSpec` passed to
`jsonic.grammar(grammarDef, { rule: { alt: { g: 'zon' } } })`, which
tags every ZON alt with the `zon` group.

### Struct vs tuple disambiguation

ZON uses the same `.{ ... }` opener for both struct literals (with
`.field = value` pairs) and tuple literals (bare values). Jsonic's
parser allows only two tokens of lookahead, so the decision is made
by the lex matcher: it scans past the opening `.{`, whitespace, and
`//` comments, then checks for `.ident` followed by `=`. This means
the grammar only ever sees an already-classified `#OB` or `#OS`
token.

### Enum literals as values

A bare `.foo` token is both a valid key (when followed by `=`) and a
valid value (enum literal). The `#TX` token set membership in both
`KEY` and `VAL` lets the parser pick the right interpretation by
context — no grammar branching is needed.


## Reference

### `Zon` (Plugin)

The plugin function. Register with `Jsonic.make().use(Zon, options)`.
`Zon.defaults` holds the merged default options.

### `ZonOptions`

```typescript
type ZonOptions = {
  // When true, parse Zig char literals ('x') as numeric code points.
  // When false (default), parse them as one-character strings.
  charAsNumber: boolean

  // When set, wrap enum literals (.foo used as value) in
  // `{ [enumTag]: name }` objects instead of producing bare strings.
  enumTag: null | string
}
```

Defaults:

```typescript
{
  charAsNumber: false,
  enumTag: null,
}
```

### Supported ZON syntax

| Construct            | Example                          | Result                   |
| -------------------- | -------------------------------- | ------------------------ |
| Struct literal       | `.{ .a = 1, .b = 2 }`            | `{ a: 1, b: 2 }`         |
| Empty struct literal | `.{}`                            | `[]` (empty list)        |
| Tuple literal        | `.{ 1, 2, 3 }`                   | `[1, 2, 3]`              |
| Nested               | `.{ .a = .{ .b = 1 } }`          | `{ a: { b: 1 } }`        |
| String               | `"hello\nworld"`                 | `'hello\nworld'`         |
| Multi-line string    | `\\line1\n\\line2`               | `'line1\nline2'`         |
| Number               | `42`, `0x2a`, `0o52`, `0b101010` | `42`                     |
| Number separator     | `1_000_000`                      | `1000000`                |
| Float                | `3.14`                           | `3.14`                   |
| Boolean / null       | `true`, `false`, `null`          | `true`, `false`, `null`  |
| Char literal         | `'A'`                            | `'A'` (or `65`)          |
| Enum literal         | `.red`                           | `'red'`                  |
| Trailing comma       | `.{ .a = 1, }`                   | `{ a: 1 }`               |
| Line comment         | `// ...`                         | *(ignored)*              |

### Grammar group tags

All grammar alternates added by the plugin carry the group tag
`zon`, so callers may exclude them via `rule.exclude: 'zon'`.
