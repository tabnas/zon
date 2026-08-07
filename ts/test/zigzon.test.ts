/* Copyright (c) 2025 Richard Rodger and other contributors, MIT License */

// zigzon.test.ts — conformance against the ZIG REFERENCE IMPLEMENTATION.
//
// The corpus is `test/zigzon/cases.json`, GENERATED (never committed) by
// `scripts/fetch-zigzon.sh` from the pinned ziglang/zig 0.16.0 release. Every
// document's verdict, and every valid document's expected value, comes from
// `std.zig.Ast` + `std.zig.ZonGen` at that same pinned version — not from this
// parser. See test/zigzon/README.md.
//
// Both halves are exercised:
//   * valid   -> must parse AND produce the reference VALUE (not merely "it
//                did not throw", which is the mistake that let a wrong-value
//                bug hide in the json5 plugin)
//   * invalid -> must be REJECTED with an error
//
// THIS SUITE MUST NEVER SKIP. A conformance test that quietly does not run is
// worse than no test, because the green tick is a lie. If the corpus is absent
// the suite FAILS with instructions, it does not skip.
//
// It is currently RED on purpose. It is a measuring instrument, not a gate. Do
// not shrink the corpus, add a skip list, or loosen the comparison to make the
// number look better.

import { describe, test, after } from 'node:test'
import assert from 'node:assert'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'
import { Zon } from '../dist/zon'

// Loaded from `dist-test/`, so hop up one level to reach the repo root.
const REPO = join(__dirname, '..', '..')
const CASES = join(REPO, 'test', 'zigzon', 'cases.json')

const MISSING = `
zigzon conformance corpus not found at:
  ${CASES}

Generate it (idempotent, ~1 min, downloads a pinned zig 0.16.0):
  bash scripts/fetch-zigzon.sh

This suite deliberately FAILS rather than skipping. A conformance test that
silently does not run reports a green tick for work it never did.
`

type ZigCase = {
  name: string
  origin: string
  source: string
  valid: boolean
  value?: unknown
  error?: string
}

// The plugin options that put its output in the SAME shape as the oracle's
// canonical encoding, so values can be compared directly:
//   enumTag '$enum'   -> `.foo` becomes {$enum:'foo'}, matching ZonGen's
//                        enum_literal (and distinguishing it from the string
//                        "foo", which the default flat encoding cannot do)
//   charAsNumber true -> `'x'` becomes its codepoint, matching ZonGen's
//                        char_literal
// This is representation alignment, not leniency: no input is excused by it.
const OPTS = { enumTag: '$enum', charAsNumber: true }

// Put the ACTUAL value in the oracle's canonical JSON shape. JSON.stringify
// turns Infinity/NaN into null, which would fail a case the parser actually got
// right; the oracle spells them "@inf"/"@-inf"/"@nan", so do the same here.
// This only ever makes a CORRECT parse comparable — it never excuses a wrong one.
function canon(v: unknown): unknown {
  if ('number' === typeof v) {
    if (Number.isNaN(v)) return '@nan'
    if (Infinity === v) return '@inf'
    if (-Infinity === v) return '@-inf'
    return v
  }
  if (null === v || 'object' !== typeof v) return v ?? null
  if (Array.isArray(v)) return v.map(canon)
  const out: Record<string, unknown> = {}
  for (const k of Object.keys(v as object)) {
    out[k] = canon((v as Record<string, unknown>)[k])
  }
  return out
}

function label(c: ZigCase): string {
  const one = c.source.replace(/\s+/g, ' ').trim()
  const short = 60 < one.length ? one.slice(0, 57) + '...' : one
  return `${c.origin} | ${short}`
}

describe('zigzon (ziglang/zig 0.16.0 reference corpus)', () => {
  // Hard failure, never a skip.
  if (!existsSync(CASES)) {
    test('corpus present', () => assert.fail(MISSING))
    return
  }

  const doc = JSON.parse(readFileSync(CASES, 'utf8'))
  const cases: ZigCase[] = doc.cases
  assert.ok(Array.isArray(cases) && 0 < cases.length, 'corpus has no cases')

  const valid = cases.filter((c) => c.valid)
  const invalid = cases.filter((c) => !c.valid)

  // If either half ever empties out, the corpus generator has broken and the
  // suite would go green while measuring half of nothing.
  assert.ok(0 < valid.length, 'corpus has no valid documents')
  assert.ok(0 < invalid.length, 'corpus has no invalid documents')

  let validOk = 0
  let invalidOk = 0

  describe('valid documents parse to the reference value', () => {
    for (const c of valid) {
      test(label(c), () => {
        const tn = new Tabnas().use(jsonic).use(Zon, OPTS)
        const got = canon(tn.parse(c.source))
        assert.deepStrictEqual(got, c.value)
        validOk++
      })
    }
  })

  describe('invalid documents are rejected', () => {
    for (const c of invalid) {
      test(label(c), () => {
        const tn = new Tabnas().use(jsonic).use(Zon, OPTS)
        let threw = false
        try {
          tn.parse(c.source)
        } catch {
          threw = true
        }
        assert.ok(
          threw,
          `accepted a document zig rejects (${c.error}):\n${c.source}`,
        )
        invalidOk++
      })
    }
  })

  after(() => {
    const pct = (a: number, b: number) => (100 * a / b).toFixed(1) + '%'
    process.stderr.write(
      `\n[zigzon] TS baseline @ zig ${doc.source.zig} (${doc.source.commit.slice(0, 12)})\n` +
      `[zigzon]   valid-accepted   ${validOk}/${valid.length} (${pct(validOk, valid.length)})\n` +
      `[zigzon]   invalid-rejected ${invalidOk}/${invalid.length} (${pct(invalidOk, invalid.length)})\n`,
    )
  })
})
