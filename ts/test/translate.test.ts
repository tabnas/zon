/* Copyright (c) 2021-2026 Richard Rodger, MIT License */

import * as assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import * as path from 'node:path'
import { test } from 'node:test'

import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'

const { translate, Zon } = require('../dist/zon')
const root = path.resolve(__dirname, '..', '..')

test('translation parts expose the manifest, source and explicit entry', () => {
  const parts = translate()
  assert.ok(parts)
  assert.equal(parts.manifest, readFileSync(path.join(root, 'tabnas.plugin.json'), 'utf8'))
  assert.equal(parts.lift, undefined)
  assert.equal(parts.render?.entry, 'zon-render')
  assert.equal(parts.render?.source, readFileSync(path.join(root, 'alchemy', 'render.alc'), 'utf8'))
})

// An embed takes a plain tree into a format's own schema. ZON's events
// carry a plain tree, so its manifest names none and the package carries
// none; a manifest that named one would be held to its file here, as the
// render is above.
test('translation parts carry the embed the manifest names, and none where it names none', () => {
  const parts = translate()
  const spec = JSON.parse(readFileSync(path.join(root, 'tabnas.plugin.json'), 'utf8')).translate
  if (null == spec.embed) {
    assert.equal(parts.embed, undefined)
  } else {
    assert.equal(parts.embed?.entry, 'zon-embed')
    assert.equal(parts.embed?.source, readFileSync(path.join(root, spec.embed), 'utf8'))
  }
})

// The documents the render writes for the reader's own big integer, an
// object whose only member is $big holding -123456789012345678901234567890,
// at the root and nested, as alchemy runs the render. Each is that
// integer, which the reader reads back as a bigint here, a *big.Int in Go
// and the { "$big": digits } object in Rust. No shared fixture can spell
// that value (see ../../test/AGENTS.md), so each runtime's translation
// test reads it; test/spec/render.tsv pins the rest of what the render
// writes for $big.
test("the render's big integer reads back as the reader's", () => {
  const zon = new Tabnas().use(jsonic).use(Zon)
  assert.equal(zon.parse('-123456789012345678901234567890\n'), -123456789012345678901234567890n)
  const nested = zon.parse(
    '.{\n  .@"n" = -123456789012345678901234567890,\n  .@"a" = .{\n    -123456789012345678901234567890,\n  },\n}\n',
  )
  assert.equal(nested.n, -123456789012345678901234567890n)
  assert.deepEqual([...nested.a], [-123456789012345678901234567890n])
})
