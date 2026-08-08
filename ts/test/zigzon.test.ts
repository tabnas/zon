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
// toolchain), so these suites skip when they are absent — the same opt-in
// convention @tabnas/toml and @tabnas/xml use for their external suites. The
// measured result is recorded in AGENTS.md; `go/zigzon_test.go` runs the
// identical corpora, so the two runtimes cannot drift.
//
// Do not shrink a corpus, add a skip list, or loosen the comparison to make
// the number look better: it is a measuring instrument.

import { describe, test } from 'node:test'
import assert from 'node:assert'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'
import { Zon } from '../dist/zon'

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

function label(c: ZigCase): string {
  const one = c.source.replace(/\s+/g, ' ').trim()
  const short = 60 < one.length ? one.slice(0, 57) + '...' : one
  return `${c.origin ?? c.name ?? ''} | ${short}`
}

function runCorpus(title: string, ...path: string[]) {
  const file = join(REPO, ...path)
  const present = existsSync(file)

  describe(title, { skip: present ? false : `${file} not present; generate it with \`bash scripts/fetch-zigzon.sh\`` }, () => {
    if (!present) return

    const doc = JSON.parse(readFileSync(file, 'utf8'))
    const cases: ZigCase[] = doc.cases
    assert.ok(Array.isArray(cases) && 0 < cases.length, 'corpus has no cases')

    const valid = cases.filter((c) => c.valid)
    const invalid = cases.filter((c) => !c.valid)
    // If either half ever empties out, the generator has broken and this
    // suite would go green while measuring half of nothing.
    assert.ok(0 < valid.length, 'corpus has no valid documents')
    assert.ok(0 < invalid.length, 'corpus has no invalid documents')

    describe('valid documents parse to the reference value', () => {
      for (const c of valid) {
        test(label(c), () => {
          const tn = new Tabnas().use(jsonic).use(Zon, OPTS)
          assert.deepStrictEqual(canon(tn.parse(c.source)), c.value)
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
        })
      }
    })
  })
}

runCorpus('zigzon (ziglang/zig 0.16.0 reference corpus)', 'test', 'zigzon', 'cases.json')
runCorpus('strictness probe (verdicts from zig 0.16.0)', 'test', 'strictness', 'cases.json')
