/* Copyright (c) 2025 Richard Rodger and other contributors, MIT License */

// strictness.test.ts — the LENIENCY probe.
//
// @tabnas/zon is a JSONIC plugin. `new Tabnas().use(Zon)` alone cannot parse
// even `.{ .a = 1 }`, so unlike (say) the json5 plugin there is no stricter
// standalone mode to fall back to: @tabnas/jsonic's relaxed number lexer and
// its "first value wins, ignore the rest" top-level behaviour are inherited
// unconditionally by the documented stack. This suite measures how much of
// that leaks through as accepted-but-not-ZON.
//
// The INPUTS are locally authored (`test/strictness/inputs.txt`, tracked).
// The VERDICTS are not: `scripts/fetch-zigzon.sh` hands every input to the
// pinned Zig reference implementation and writes `test/strictness/cases.json`
// (generated, never committed). This repo never decides what ZON means.
//
// Same rules as zigzon.test.ts: never skips, checks both halves, compares
// values rather than "it did not throw", and is RED on purpose.

import { describe, test, after } from 'node:test'
import assert from 'node:assert'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'
import { Zon } from '../dist/zon'

const REPO = join(__dirname, '..', '..')
const CASES = join(REPO, 'test', 'strictness', 'cases.json')

const MISSING = `
strictness probe verdicts not found at:
  ${CASES}

Generate them (idempotent):
  bash scripts/fetch-zigzon.sh

This suite deliberately FAILS rather than skipping.
`

type ProbeCase = {
  source: string
  valid: boolean
  value?: unknown
  error?: string
}

// Identical to zigzon.test.ts — see the note there.
const OPTS = { enumTag: '$enum', charAsNumber: true }

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

function label(src: string): string {
  const one = src.replace(/\n/g, '\\n').replace(/\t/g, '\\t')
  return 60 < one.length ? one.slice(0, 57) + '...' : one
}

describe('strictness probe (verdicts from zig 0.16.0)', () => {
  if (!existsSync(CASES)) {
    test('probe verdicts present', () => assert.fail(MISSING))
    return
  }

  const doc = JSON.parse(readFileSync(CASES, 'utf8'))
  const cases: ProbeCase[] = doc.cases
  assert.ok(Array.isArray(cases) && 0 < cases.length, 'probe has no cases')

  const valid = cases.filter((c) => c.valid)
  const invalid = cases.filter((c) => !c.valid)
  assert.ok(0 < valid.length, 'probe has no valid inputs')
  assert.ok(0 < invalid.length, 'probe has no invalid inputs')

  let validOk = 0
  let invalidOk = 0

  describe('zig-valid inputs parse to the reference value', () => {
    for (const c of valid) {
      test(label(c.source), () => {
        const tn = new Tabnas().use(jsonic).use(Zon, OPTS)
        assert.deepStrictEqual(canon(tn.parse(c.source)), c.value)
        validOk++
      })
    }
  })

  describe('zig-invalid inputs are rejected', () => {
    for (const c of invalid) {
      test(label(c.source), () => {
        const tn = new Tabnas().use(jsonic).use(Zon, OPTS)
        let threw = false
        let got: unknown
        try {
          got = tn.parse(c.source)
        } catch {
          threw = true
        }
        assert.ok(
          threw,
          `LENIENCY LEAK: accepted as ${JSON.stringify(got)} ` +
          `an input zig rejects (${c.error})`,
        )
        invalidOk++
      })
    }
  })

  after(() => {
    const pct = (a: number, b: number) => (100 * a / b).toFixed(1) + '%'
    process.stderr.write(
      `\n[strictness] TS baseline @ zig ${doc.source.zig}\n` +
      `[strictness]   valid-accepted   ${validOk}/${valid.length} (${pct(validOk, valid.length)})\n` +
      `[strictness]   invalid-rejected ${invalidOk}/${invalid.length} (${pct(invalidOk, invalid.length)})\n`,
    )
  })
})
