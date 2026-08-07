# strictness — the leniency probe

A locally authored probe aimed at one specific question: **how much of
`@tabnas/jsonic`'s leniency leaks through `@tabnas/zon` as
accepted-but-not-ZON?**

- [`inputs.txt`](inputs.txt) — **ours, and tracked.** One input per line.
- `cases.json` — **generated, and `.gitignore`d.** Produced by
  [`scripts/fetch-zigzon.sh`](../../scripts/fetch-zigzon.sh), which hands every
  input to the pinned Zig reference implementation
  ([`../zigzon/tools/oracle.zig`](../zigzon/README.md)) and records its verdict.

The split matters. The *questions* are ours; the *answers* are Zig's. A probe
list carrying its own expected results would just be this repo grading itself,
which is how a conformance number becomes meaningless.

## Why this probe exists

`@tabnas/zon` is a **jsonic plugin**, and it is not usable without jsonic:

```js
new Tabnas().use(Zon).parse('.{ .a = 1 }')
// => [tabnas/unexpected]: unexpected character(s): .{
```

The plugin alone cannot parse *any* ZON — not one document. So unlike the json5
plugin, where the standalone plugin is stricter than the documented
`use(jsonic).use(json5)` stack, there is **no stricter mode to fall back to**.
jsonic's relaxed number lexer and its "first value wins, ignore the rest"
top-level behaviour are inherited unconditionally by the only supported setup.

That makes leniency the structural, and probably the dominant, cause of
non-conformance here — hence a dedicated probe.

## What it covers

Relaxed numbers (`+1`, `0123`, `1__0`, `0x_2A`, `.5`, `5.`), trailing content
at the top level (`1 2`, `.{ .a = 1 },`, `.{ .a = 1 }.a`), duplicate struct
fields, JSON-isms ZON does not have (`{"a":1}`, `[1,2]`), string and char
escape edge cases, enum-literal edge cases, and comment forms.

## Runners

| | |
|---|---|
| TypeScript | [`ts/test/strictness.test.ts`](../../ts/test/strictness.test.ts) |
| Go | [`go/strictness_test.go`](../../go/strictness_test.go) |

Both halves are asserted: zig-valid inputs must parse **to the reference
value**; zig-invalid inputs must be **rejected**. A leak is reported with the
value that was wrongly accepted, so it is actionable.

Neither runner skips. If `cases.json` is absent they fail with instructions.

## Red on purpose

Same rule as [`../zigzon/README.md`](../zigzon/README.md): this is a measuring
instrument, not a gate. Do not delete an input, add a skip, or loosen a
comparison to improve the number. Adding *more* inputs is always welcome;
removing one to go green is not.
