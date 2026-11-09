/* Copyright (c) 2025 Richard Rodger and other contributors, MIT License */

// zigzon.test.ts — conformance against the ZIG REFERENCE IMPLEMENTATION.
//
// Two corpora, both GENERATED (never committed) by `scripts/fetch-zigzon.sh`
// from the pinned ziglang/zig 0.16.0 release:
//
//   test/zigzon/cases.json      every .zon file and every ZON snippet in the
//                               zig tree, plus its verdict/value
//   test/strictness/cases.json  locally authored leniency probes, judged by
//                               the same reference implementation
//
// Every verdict, and every valid document's expected VALUE, comes from
// `std.zig.Ast` + `std.zig.ZonGen` at that pinned version — this repo never
// decides what ZON means. Both halves are exercised: a valid document must
// parse AND produce the reference value (not merely "it did not throw", which
// is the mistake that let a wrong-value bug hide in a sibling plugin), and an
// invalid document must be rejected.
//
// The corpora are not bundled (generating them downloads a pinned zig
// toolchain), so `ts/package.json` runs `scripts/fetch-zigzon.sh` as a
// `pretest` hook: they are built before `npm test`, wherever it runs, CI
// included. A corpus that is still absent afterwards is a FAILURE here, never
// a skip — a suite that quietly does not run reports a green tick that means
// nothing. The one exception is a host the pinned zig oracle toolchain is not
// wired for (see ORACLE_HOST): there it reports a single explicit,
// platform-named skip.
//
// `go/zigzon_test.go` and `rs/tests/zigzon_test.rs` run the identical
// corpora, so the runtimes cannot drift.
//
// Every document is graded from a reader as well as from its text: read a
// byte at a time through `make(OPTS)`, which declares `lex.window`, under
// the engine's own window and under a window of one character, a valid
// document must parse to the same reference value, and an invalid one must
// be rejected with the code its text is (`window.test.ts`).
//
// Do not shrink a corpus, add a skip list, or loosen the comparison to make
// the number look better: it is a measuring instrument.

import { describe, test } from 'node:test'
import assert from 'node:assert'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'
import { Zon, make } from '../dist/zon'

// Loaded from `dist-test/`, so hop up one level to reach the repo root.
const REPO = join(__dirname, '..', '..')

type ZigCase = {
  name?: string
  origin?: string
  source: string
  valid: boolean
  value?: any
  error?: string
}

// The plugin options that put its output in the SAME shape as the oracle's
// canonical encoding, so values compare directly:
//   enumTag '$enum'   -> `.foo` becomes {$enum:'foo'}, matching ZonGen's
//                        enum_literal (and distinguishing it from the string
//                        "foo", which the default flat encoding cannot do)
//   charAsNumber true -> `'x'` becomes its codepoint, matching ZonGen's
//                        char_literal
// This is representation alignment, not leniency: no input is excused by it.
const OPTS = { enumTag: '$enum', charAsNumber: true }

// Put the ACTUAL value in the oracle's canonical JSON shape. The oracle spells
// the non-JSON numbers "@inf"/"@-inf"/"@nan", and an integer too large for an
// exact double as {$big:"<decimal>"} — which is exactly what the plugin
// returns as a bigint. This only makes a CORRECT parse comparable; it never
// excuses a wrong one.
function canon(v: any): any {
  if ('bigint' === typeof v) return { $big: v.toString() }
  if ('number' === typeof v) {
    if (Number.isNaN(v)) return '@nan'
    if (Infinity === v) return '@inf'
    if (-Infinity === v) return '@-inf'
    return v
  }
  if (null === v || 'object' !== typeof v) return v ?? null
  if (Array.isArray(v)) return v.map(canon)
  const out: any = {}
  for (const k of Object.keys(v)) out[k] = canon(v[k])
  return out
}

// A reader that hands the text's bytes over one at a time.
function bytewise(text: string): { read(buf: Uint8Array): number } {
  const bytes = Buffer.from(text, 'utf8')
  let at = 0
  return {
    read(buf: Uint8Array): number {
      if (at === bytes.length || 0 === buf.length) return 0
      buf[0] = bytes[at++]
      return 1
    },
  }
}

// The parses of a document from a reader: under the engine's own window,
// and under a window of one character, which every token meets the edge of.
const READS: [string, (tn: any, source: string) => any][] = [
  ['from a reader', (tn, source) => tn.parseReader(bytewise(source))],
  [
    'from a reader under a window of one character',
    (tn, source) =>
      tn.parseReaderWithSizes(bytewise(source), { margin: 1, block: 1, chunk: 1 }),
  ],
]

// The code a parse fails with, or undefined where it does not fail.
function codeOf(parse: () => any): string | undefined {
  try {
    parse()
    return undefined
  } catch (e: any) {
    return String(e?.code)
  }
}

function label(c: ZigCase): string {
  const one = c.source.replace(/\s+/g, ' ').trim()
  const short = 60 < one.length ? one.slice(0, 57) + '...' : one
  return `${c.origin ?? c.name ?? ''} | ${short}`
}

// Hosts scripts/fetch-zigzon.sh has a pinned zig oracle toolchain for. On
// anything else the corpus cannot be built at all, and that — not a missing
// file — is what the one permitted skip below reports.
const ORACLE_HOST =
  ('linux' === process.platform || 'darwin' === process.platform) &&
  ('x64' === process.arch || 'arm64' === process.arch)

// The census each corpus must have. A floor of "more than zero" lets a broken
// generator quietly shrink the corpus and inflate the pass rate; an exact
// count makes that go red. Re-measured 2026-08-09 against zig 0.16.0.
type Census = { valid: number; invalid: number }

function runCorpus(title: string, census: Census, ...path: string[]) {
  const file = join(REPO, ...path)
  const present = existsSync(file)

  if (!present) {
    if (!ORACLE_HOST) {
      // Not a hidden skip: it names the platform and it cannot mask a missing
      // corpus anywhere the oracle can actually be built.
      describe(title, {
        skip:
          `no pinned zig oracle toolchain for ${process.platform}/${process.arch}, ` +
          `so the corpus cannot be generated on this host`,
      }, () => { })
      return
    }
    describe(title, () => {
      test('the conformance corpus is present', () => {
        assert.fail(
          `${file} is missing.\n` +
          `It is generated by \`bash scripts/fetch-zigzon.sh\`, which \`npm test\` ` +
          `runs as a pretest hook. This suite FAILS rather than skips when the ` +
          `corpus is absent: a conformance suite that quietly does not run ` +
          `reports a green tick while measuring nothing.`,
        )
      })
    })
    return
  }

  describe(title, () => {
    // One parser that declares the window, for the reads from a reader.
    const windowed = make(OPTS)
    const doc = JSON.parse(readFileSync(file, 'utf8'))
    const cases: ZigCase[] = doc.cases
    assert.ok(Array.isArray(cases) && 0 < cases.length, 'corpus has no cases')

    const valid = cases.filter((c) => c.valid)
    const invalid = cases.filter((c) => !c.valid)

    test('corpus census', () => {
      assert.deepStrictEqual(
        { valid: valid.length, invalid: invalid.length },
        census,
        'the corpus is not the size it is pinned at — the generator changed, ' +
        'or the corpus was narrowed. Do not adjust this number to match; ' +
        'find out what changed.',
      )
    })

    describe('valid documents parse to the reference value', () => {
      for (const c of valid) {
        test(label(c), () => {
          const tn = new Tabnas().use(jsonic).use(Zon, OPTS)
          assert.deepStrictEqual(canon(tn.parse(c.source)), c.value)
          for (const [how, read] of READS) {
            assert.deepStrictEqual(canon(read(windowed, c.source)), c.value, how)
          }
        })
      }
    })

    describe('invalid documents are rejected', () => {
      for (const c of invalid) {
        test(label(c), () => {
          const tn = new Tabnas().use(jsonic).use(Zon, OPTS)
          let threw = false
          let got: any
          try {
            got = tn.parse(c.source)
          } catch {
            threw = true
          }
          assert.ok(
            threw,
            `accepted as ${JSON.stringify(canon(got))} an input zig rejects ` +
            `(${c.error}):\n${c.source}`,
          )
          const code = codeOf(() => tn.parse(c.source))
          for (const [how, read] of READS) {
            assert.equal(codeOf(() => read(windowed, c.source)), code, how)
          }
        })
      }
    })
  })
}

runCorpus(
  'zigzon (ziglang/zig 0.16.0 reference corpus)',
  { valid: 184, invalid: 44 },
  'test', 'zigzon', 'cases.json',
)
runCorpus(
  'strictness probe (verdicts from zig 0.16.0)',
  { valid: 68, invalid: 95 },
  'test', 'strictness', 'cases.json',
)
