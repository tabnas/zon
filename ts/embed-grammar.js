#!/usr/bin/env node

// Embed zon-grammar.jsonic into the TypeScript, Go and Rust source files.
// Run via: npm run embed  (or:  node embed-grammar.js)

const fs = require('fs')
const path = require('path')

const GRAMMAR_FILE = path.join(__dirname, '..', 'zon-grammar.jsonic')
const TS_FILE = path.join(__dirname, 'src', 'zon.ts')
const GO_FILE = path.join(__dirname, '..', 'go', 'zon.go')
const RS_FILE = path.join(__dirname, '..', 'rs', 'src', 'lib.rs')

const BEGIN = '// --- BEGIN EMBEDDED zon-grammar.jsonic ---'
const END = '// --- END EMBEDDED zon-grammar.jsonic ---'

const grammar = fs.readFileSync(GRAMMAR_FILE, 'utf8')

// --- TypeScript embedding ---
function embedTS() {
  let src = fs.readFileSync(TS_FILE, 'utf8')
  const startIdx = src.indexOf(BEGIN)
  const endIdx = src.indexOf(END)
  if (startIdx === -1 || endIdx === -1) {
    console.error('TS markers not found in', TS_FILE)
    process.exit(1)
  }

  // Escape backticks and template expressions for a JS template literal.
  const escaped = grammar
    .replace(/\\/g, '\\\\')
    .replace(/`/g, '\\`')
    .replace(/\$\{/g, '\\${')

  const replacement =
    BEGIN +
    '\nconst grammarText = `\n' +
    escaped +
    '`\n' +
    END

  src = src.substring(0, startIdx) + replacement + src.substring(endIdx + END.length)
  fs.writeFileSync(TS_FILE, src)
  console.log('Embedded grammar into', TS_FILE)
}

// --- Go embedding ---
function embedGo() {
  let src = fs.readFileSync(GO_FILE, 'utf8')
  const startIdx = src.indexOf(BEGIN)
  const endIdx = src.indexOf(END)
  if (startIdx === -1 || endIdx === -1) {
    console.error('Go markers not found in', GO_FILE)
    process.exit(1)
  }

  if (grammar.includes('`')) {
    console.error('Grammar contains backticks, incompatible with Go raw strings')
    process.exit(1)
  }

  // The blank line before END keeps the result gofmt-clean (gofmt wants
  // a blank line between the const declaration and the trailing comment).
  const replacement =
    BEGIN +
    '\nconst grammarText = `\n' +
    grammar +
    '`\n\n' +
    END

  src = src.substring(0, startIdx) + replacement + src.substring(endIdx + END.length)
  fs.writeFileSync(GO_FILE, src)
  console.log('Embedded grammar into', GO_FILE)
}

// --- Rust embedding ---
function embedRust() {
  let src = fs.readFileSync(RS_FILE, 'utf8')
  const startIdx = src.indexOf(BEGIN)
  const endIdx = src.indexOf(END)
  if (startIdx === -1 || endIdx === -1) {
    console.error('Rust markers not found in', RS_FILE)
    process.exit(1)
  }

  // A Rust raw string has no escapes, so the grammar goes in verbatim. The
  // hash count has to clear the longest `"#...` run the text contains, and
  // the grammar is full of `'#OS'`-style token names, so two hashes is the
  // floor rather than the usual one.
  if (grammar.includes('"##')) {
    console.error('Grammar contains `"##`, incompatible with the r## raw string')
    process.exit(1)
  }

  const replacement =
    BEGIN +
    '\npub const GRAMMAR_TEXT: &str = r##"\n' +
    grammar +
    '"##;\n' +
    END

  src = src.substring(0, startIdx) + replacement + src.substring(endIdx + END.length)
  fs.writeFileSync(RS_FILE, src)
  console.log('Embedded grammar into', RS_FILE)
}

embedTS()
embedGo()
embedRust()
