/* Copyright (c) 2026 Richard Rodger and other contributors, MIT License */

// A parse from a reader (`tn.parseReader`, admin ADR-30 and ADR-33).
//
// `make()` builds the parser whole, as `new Tabnas().use(jsonic).use(Zon,
// options)` builds it, and declares `lex.window` on it, so that a reader is
// read a window at a time. The matchers this grammar installs (zonDot,
// zonMultiString, zonChar, zonNumber, zonDocComment and zonString) read the
// source only through the lexer's window. These tests hold it to that:
// `make()` declares the window and parses as that composition does; every
// row of the shared fixtures, under its `opts`, and every probe of
// `test/strictness/inputs.txt`, under the defaults and under the options the
// zig corpora are graded with, parses the same from a reader, in pieces of
// 1, 3 and 64 bytes, under windows of a few UTF-16 units and the engine's
// own, as from its text; and so do tokens of every kind that the window's
// edge cuts at every place, and a long document. `zigzon.test.ts` grades
// every document of the zig corpora from a reader as well as from its text.

import { test, describe } from 'node:test'
import assert from 'node:assert'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { inspect } from 'node:util'

import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'
import { findSpecDir, loadSpecDir } from '@tabnas/support'

import { Zon, make } from '../dist/zon'
import type { ZonOptions } from '../dist/zon'

const SPEC = findSpecDir(__dirname)
const PROBES = join(__dirname, '..', '..', 'test', 'strictness', 'inputs.txt')

// The options the zig corpora are graded with (zigzon.test.ts).
const ZIG: Partial<ZonOptions> = { enumTag: '$enum', charAsNumber: true }

type Reader = { read(buf: Uint8Array): number }
type Sizes = { margin: number; block: number; chunk: number }
type Seen = { value?: unknown; error?: string }

// A reader that hands its bytes over at most `size` at a time.
function pieces(bytes: Uint8Array, size: number): Reader & { at: number } {
  return {
    at: 0,
    read(buf: Uint8Array): number {
      const n = Math.min(size, buf.length, bytes.length - this.at)
      buf.set(bytes.subarray(this.at, this.at + n))
      this.at += n
      return n
    },
  }
}

// What a parse shows: its value, or its error's code, position, row,
// column and token text. An error's excerpt is left out: a parse from a
// reader shows a long line only as far back as the window reaches
// (tabnas/parser DIVERGENCE.md), the one difference allowed.
function seen(parse: () => unknown): Seen {
  try {
    return { value: parse() }
  } catch (e: any) {
    if (null == e || 'function' !== typeof e.toJSON) {
      return { error: 'not a TabnasError: ' + (e && e.message) }
    }
    const d = e.toJSON()
    return {
      error:
        d.code + ' at ' + d.pos + ' ' + d.row + ':' + d.col + ' ' +
        JSON.stringify(d.token.src),
    }
  }
}

const show = (o: Seen): string =>
  undefined !== o.error ? 'error ' + o.error : 'value ' + inspect(o.value, { depth: 4 })

// The same outcome: the same error, or values deeply and strictly equal,
// so that a bigint, NaN, an infinity and -0 count.
function same(a: Seen, b: Seen): boolean {
  if (a.error !== b.error) return false
  try {
    assert.deepStrictEqual(a.value, b.value)
    return true
  } catch (e) {
    return false
  }
}

// The pieces a reader hands its bytes over in, and the windows (margin and
// block) it is read under besides the engine's own: a few units, so that
// every matcher meets the window's edge.
const PIECES = [1, 3, 64]
const WINDOWS = [[1, 1], [3, 2], [64, 64]]

// `text` parsed by `tn` whole, and from a reader in every size of `sizes`,
// under the engine's window and each of WINDOWS: how many reader parses
// were compared. A difference is pushed onto `failures`.
function sameFromReader(
  tn: Tabnas,
  at: string,
  text: string,
  failures: string[],
  sizes: number[] = PIECES,
): number {
  const whole = seen(() => tn.parse(text))
  const bytes = Buffer.from(text, 'utf8')
  const anyTn = tn as any
  let compared = 0
  for (const size of sizes) {
    const reads: [string, () => unknown][] = [['', () => anyTn.parseReader(pieces(bytes, size))]]
    for (const [margin, block] of WINDOWS) {
      const window: Sizes = { margin, block, chunk: size }
      reads.push([
        ', margin ' + margin + ', block ' + block,
        () => anyTn.parseReaderWithSizes(pieces(bytes, size), window),
      ])
    }
    for (const [how, parse] of reads) {
      const read = seen(parse)
      compared++
      if (!same(read, whole)) {
        failures.push(
          at + ': ' + JSON.stringify(text.slice(0, 80)) + ' from a reader in pieces of ' +
          size + how + '\n  whole:  ' + show(whole).slice(0, 200) +
          '\n  reader: ' + show(read).slice(0, 200),
        )
      }
    }
  }
  return compared
}

const optionsOf = (cell: string): Partial<ZonOptions> =>
  '' === cell.trim() ? {} : JSON.parse(cell)

// One parser per set of options, built once: building the grammar costs
// more than a parse.
function cached(build: (options: Partial<ZonOptions>) => Tabnas) {
  const built = new Map<string, Tabnas>()
  return (options: Partial<ZonOptions>): Tabnas => {
    const key = JSON.stringify(options)
    let tn = built.get(key)
    if (undefined === tn) {
      tn = build(options)
      built.set(key, tn)
    }
    return tn
  }
}

// Every row of the shared fixtures: where it is, its input and its options.
function fixtureRows(): [string, string, Partial<ZonOptions>][] {
  const rows: [string, string, Partial<ZonOptions>][] = []
  for (const spec of loadSpecDir(SPEC)) {
    for (const row of spec.rows) {
      rows.push([row.where(), row.unescNamed('input'), optionsOf(row.named('opts'))])
    }
  }
  assert.ok(190 <= rows.length, '' + rows.length)
  return rows
}

// The probes of test/strictness/inputs.txt, the sources of the strictness
// corpus, decoded as scripts/fetch-zigzon.sh decodes them: `\n`, `\r`, `\t`
// and `\\`, with blank lines and comment lines skipped.
function probes(): string[] {
  const out: string[] = []
  for (const line of readFileSync(PROBES, 'utf8').split('\n')) {
    if ('' === line.trim() || line.startsWith('#')) continue
    out.push(
      line.replace(/\\([nrt\\])/g, (_m, c) =>
        'n' === c ? '\n' : 'r' === c ? '\r' : 't' === c ? '\t' : '\\'),
    )
  }
  assert.ok(160 <= out.length, '' + out.length)
  return out
}

// Tokens of every kind the matchers cut, well formed and not, each placed
// where the window's edge cuts it at every place: alone, as a field's
// value, and as an item of a tuple, after 0 to 6 spaces.
const SHAPES = [
  // zonDot: the struct opener, which reads on to decide map or tuple, a
  // field name, an enum literal, and the quoted name.
  '.{ .a = 1 }', '.{ 1, 2 }', '.{}', '.{ .abc }', '.{ .abc = .def }',
  '. foo', '.\n// c\nfoo', '.{ // c\n .a = 1 }', '.{ .@"a b" = 1 }',
  '.{ .@"a\\u{41}" = 1, .b = 2 }', '.@"x\\u{41}y"', '.@""', '.@"a',
  '.@"\\x00"', '.{\n\t.a = 1,\n}', '.{ . a = 1 }', '.{ .a\n= 1 }', '._a_1',
  // zonString.
  '"abc"', '"a\\x41b"', '"\\xe2\\x82\\xac"', '"a\\u{1F600}b"',
  '"\\u{0000000041}"', '"\\u{D800}"', '"\\u{110000}"', '"\\u{}"',
  '"\\u{41"', '"\\u0041"', '"a\\qb"', '"abc', '"ab\ncd"', '"a\tb"',
  '"\\x4"', '"\\"', '"\\', '"€😀"', '"\\n\\r\\t\\\\\\\'\\""',
  // zonMultiString.
  '\\\\abc\n\\\\def', '\\\\abc\r\n\\\\def', '\\\\abc\n\n  \\\\def',
  '\\\\abc', '\\\\', '\\\\a\n', '.{ .t = \\\\a\n , .u = 1 }',
  // zonChar.
  "'a'", "'\\n'", "'\\x41'", "'\\u{1F600}'", "'\\u{0041}'",
  "'\\u{110000}'", "'😀'", "'\\u{41'", "'\\u{4G}'", "''", "'ab'",
  "'\\0'", "'\n'", "'a", "'\\x4'", "'é'",
  // zonNumber.
  '123', '1_000', '0x2A', '0x1.8p3', '1.5e-3', '1.e3', '0xF.p1', '-inf',
  'inf', 'nan', '-nan', '-0', '-0.0', '0123', '1__0', '0X2A', '0b102',
  '99999999999999999999999', '- 5', '1.', 'infinity', '0x_2A', '0o17',
  '1e', '1e+', '0x1p-2', '12abc', '1.2.3', '-', '0',
  // zonDocComment, and the plain comment it lets through.
  '//! x\n1', '/// x\n1', '//// x\n1', '// x\n1', '//', '///', '//!',
  '1 // x', '1 ///',
]

function placed(shape: string): string[] {
  const out: string[] = []
  for (let k = 0; k <= 6; k++) {
    const pad = ' '.repeat(k)
    out.push(pad + shape, '.{ .k =' + pad + shape + ' }', '.{' + pad + shape + ', 0 }')
  }
  return out
}

// A long document: a struct of `n` fields of every kind, some of them long
// tokens, then `ending` in place of its close.
function longDocument(n: number, long: number, ending: string): string {
  const parts: string[] = ['.{\n']
  for (let i = 0; i < n; i++) {
    parts.push(
      '  .f' + i + ' = .{ .s = "s\\x41\\u{1F600}' + i + '", .c = \'\\u{41}\', ' +
      '.n = ' + (i * 1_000_003) + ', .x = 0x1.8p' + (i % 7) + ', .e = .tag' + i +
      ', .q = .@"q ' + i + '", .m = \\\\line ' + i + '\n    \\\\more\n  , .t = .{ 1, -2.5, inf } },' +
      ' // note ' + i + '\n',
    )
  }
  parts.push(
    '  .long_string = "' + 'é'.repeat(long) + '\\u{' + '0'.repeat(long) + '41}",\n',
    '  .long_multi = \\\\' + 'm'.repeat(long) + '\n' + ' '.repeat(long) + '\\\\end\n  ,\n',
    '  .long_number = ' + '1_0'.repeat(long) + '.5e-3,\n',
    '  .long_name_' + 'a'.repeat(long) + ' = .' + ' '.repeat(long) + 'b,\n',
    '  .long_char = \'\\u{' + '0'.repeat(long) + '1F600}\',\n',
    '  .long_open = .' + '\n// c'.repeat(long) + '\n{ .' + ' '.repeat(long) + 'k = 1 },\n',
  )
  parts.push(ending)
  return parts.join('')
}

const ENDINGS = [
  '}\n',
  '  .bad = "unterminated',
  '  .bad = 0X12,\n}',
  '  .bad = \'\\u{110000}\',\n}',
  '  .bad = .@"",\n}',
  '  .f0 = 1,\n}',
  '  //! doc\n}',
  '  .last = \\\\open',
]

describe('window', () => {
  test('make-declares-the-window', () => {
    assert.equal((make() as any).internal().config.lex.window, true)
    assert.equal((make(ZIG) as any).internal().config.lex.window, true)
    // The plugin alone declares nothing: a host may install it beneath
    // matchers of its own.
    const composed = new Tabnas().use(jsonic).use(Zon, {}) as any
    assert.ok(!composed.internal().config.lex.window)
  })

  test('make-parses-as-the-composition-does', () => {
    // Every fixture row, whole, through `make(opts)` and through
    // `new Tabnas().use(jsonic).use(Zon, opts)`, which dmog composed.
    const failures: string[] = []
    const made = cached(make)
    const composition = cached((options) => new Tabnas().use(jsonic).use(Zon, options))
    for (const [at, input, options] of fixtureRows()) {
      const fromMake = seen(() => made(options).parse(input))
      const composed = seen(() => composition(options).parse(input))
      if (!same(fromMake, composed)) {
        failures.push(at + '\n  make:     ' + show(fromMake) + '\n  composed: ' + show(composed))
      }
    }
    assert.deepEqual(failures, [])
  })

  test('fixtures-parse-the-same-from-a-reader', () => {
    const failures: string[] = []
    let compared = 0
    const made = cached(make)
    for (const [at, input, options] of fixtureRows()) {
      compared += sameFromReader(made(options), at, input, failures)
    }
    assert.deepEqual(failures.slice(0, 10), [])
    assert.ok(2300 <= compared, '' + compared)
  })

  test('strictness-probes-parse-the-same-from-a-reader', () => {
    const failures: string[] = []
    let compared = 0
    for (const options of [{}, ZIG]) {
      const tn = make(options)
      for (const probe of probes()) {
        compared += sameFromReader(tn, JSON.stringify(options), probe, failures)
      }
    }
    assert.deepEqual(failures.slice(0, 10), [])
    assert.ok(2500 <= compared, '' + compared)
  })

  test('tokens-meet-the-window-edge-at-every-place', () => {
    const failures: string[] = []
    let compared = 0
    for (const options of [{}, ZIG]) {
      const tn = make(options)
      for (const shape of SHAPES) {
        for (const text of placed(shape)) {
          compared += sameFromReader(tn, 'shape ' + JSON.stringify(shape), text, failures, [1, 3])
        }
      }
    }
    assert.deepEqual(failures.slice(0, 10), [])
    assert.ok(20000 <= compared, '' + compared)
  })

  test('tokens-longer-than-the-window-parse-the-same-from-a-reader', () => {
    // The engine's own window reaches 64 KiB past the cursor before each
    // token: a token longer than that meets its edge under the engine's
    // own sizes, and each matcher reads the window on as it cuts it.
    const long = (64 << 10) + 2048
    const docs = [
      '.{ .k = "' + 'é'.repeat(long / 2) + '\\u{1F600}", .z = 1 }',
      '.{ .k = "\\u{' + '0'.repeat(long) + '41}", .z = 1 }',
      '.{ .k = "' + 'q'.repeat(long) + '\\q", .z = 1 }',
      '.{ .k = "' + 'u'.repeat(long),
      '.{ .k = \\\\' + 'm'.repeat(long) + '\n' + ' '.repeat(long) + '\\\\end\n, .z = 1 }',
      '.{ .k = ' + '1'.repeat(long) + '.5, .z = 1 }',
      '.{ .k = ' + '1'.repeat(long) + 'x, .z = 1 }',
      '.{ .' + 'a'.repeat(long) + ' = 1, .z = 1 }',
      '.{ .k = .' + ' '.repeat(long) + 'b, .z = 1 }',
      '.{ .k = .' + '\n// c'.repeat(long / 5) + '\n{ .' + ' '.repeat(long) + 'k = 1 } }',
      '.{ .k = .@"' + 'w'.repeat(long) + '", .z = 1 }',
      '.{ .k = \'\\u{' + '0'.repeat(long) + '1F600}\', .z = 1 }',
      '.{ .k = \'\\u{' + '0'.repeat(long) + '110000}\', .z = 1 }',
    ]
    const tn = make()
    const failures: string[] = []
    for (const doc of docs) {
      sameFromReader(tn, 'long token', doc, failures, [4096, (64 << 10) + 3])
    }
    assert.deepEqual(failures, [])
  })

  test('a-long-document-parses-the-same-from-a-reader', () => {
    // About 60 KB, with a token of every kind longer than the small
    // windows, so that each is read on as it is cut.
    const tn = make()
    const failures: string[] = []
    for (const ending of ENDINGS) {
      const text = longDocument(100, 2500, ending)
      const whole = seen(() => tn.parse(text))
      if (ending === ENDINGS[0]) {
        assert.equal(undefined, whole.error, show(whole))
      } else {
        assert.notEqual(undefined, whole.error, ending)
      }
      sameFromReader(tn, 'long, ending ' + JSON.stringify(ending), text, failures, [64])
    }
    assert.deepEqual(failures, [])
  })
})
