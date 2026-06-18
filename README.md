# @tabnas/zon

This plugin allows the [Tabnas](https://github.com/tabnas/parser) JSON parser to support Zig Object Notation (ZON) syntax.

This repository contains:

| Path | Description |
|---|---|
| [`ts/`](ts/) | TypeScript / JavaScript implementation. |
| [`go/`](go/) | Go port. |

See [`ts/README.md`](ts/README.md) for usage.

## Grammar

The grammar is defined once in the top-level [`zon-grammar.jsonic`](zon-grammar.jsonic)
and embedded into both implementations — the TypeScript ([`ts/src/zon.ts`](ts/src/zon.ts))
and Go ([`go/zon.go`](go/zon.go)) sources — by [`ts/embed-grammar.js`](ts/embed-grammar.js),
which runs as part of the TypeScript build. Edit the grammar there, not in the
generated source files.

## Grammar diagram

The grammar as a railroad/syntax diagram, generated from the live grammar
with [`@tabnas/railroad`](https://github.com/tabnas/railroad):

![zon grammar railroad diagram](ts/doc/grammar.svg)

ASCII version: [`ts/doc/grammar.txt`](ts/doc/grammar.txt).

## License

MIT. Copyright (c) Richard Rodger.
