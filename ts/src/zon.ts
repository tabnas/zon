/* Copyright (c) 2025 Richard Rodger, MIT License */

// The engine is the tabnas parser; jsonic supplies the relaxed-JSON
// grammar that the embedded grammar text is authored in.
import {
  Tabnas,
  Rule,
  Plugin,
  Context,
  Config,
  TabnasOptions,
  Lex,
} from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'

// Plugin options.
type ZonOptions = {
  // When true, parse Zig char literals ('x') as numeric code points.
  // When false, parse them as single-character strings.
  charAsNumber: boolean
  // When set, wrap enum literals (.foo used as value) in `{ [enumTag]: 'foo' }`
  // instead of producing the bare string 'foo'.
  enumTag: null | string
}

// --- BEGIN EMBEDDED zon-grammar.jsonic ---
const grammarText = `
# ZON Grammar Definition
# Parses Zig Object Notation (ZON) - a data format based on Zig anonymous
# struct literals.
#
# Example:
#   .{
#       .name = "example",
#       .version = "0.0.1",
#       .deps = .{ .foo = .{ .url = "https://..." } },
#       .paths = .{ "build.zig", "src" },
#   }
#
# The custom zon-dot lex matcher distinguishes struct (map) and tuple (list)
# openings at lex time by peeking ahead:
#   .{  followed by  .ident =    -> emits #OB  (struct / map)
#   .{  otherwise                -> emits #OS  (tuple / list)
# Both close on } which lexes as #CB. The list rules below use #CB (not
# the default #CS) as the list terminator so that the single } character
# closes both struct and tuple literals.
#
# A bare .identifier emits #TX with val = identifier (the leading dot is
# stripped). This token is both a valid KEY (when followed by =) and a
# valid VAL (when used as an enum literal).
#
# The grammar is applied with { rule: { alt: { g: 'zon' } } } so every
# alt below is automatically tagged with the 'zon' group.

{
  rule: val: open: [
    # Empty .{} -> empty list.
    { s: '#OS #CB' b: 2 p: list g: 'list,empty' }
  ]

  rule: list: open: [
    # @array$ allocates the list node (an empty array). Under the new core
    # the list node is no longer auto-seeded: @tabnas/json's @array$ only
    # runs on its own #OS open alts, which ZON replaces here. Without this
    # the elem rule's @elem-bc/replace would push onto an undefined node.
    { s: '#OS #CB' b: 1 a: '@array$' g: 'list,empty' }
    { s: '#OS' p: elem a: '@array$' g: 'list,open' }
  ]
  rule: list: close: [
    { s: '#CB' g: 'list,close' }
  ]

  rule: elem: close: [
    { s: '#CA #CB' b: 1 g: 'elem,trailing,comma' }
    { s: '#CA' r: elem g: 'elem,next,comma' }
    { s: '#CB' b: 1 g: 'elem,end' }
  ]

  rule: pair: close: [
    { s: '#CA #CB' b: 1 g: 'pair,trailing,comma' }
    { s: '#CA' r: pair g: 'pair,next,comma' }
    { s: '#CB' b: 1 g: 'pair,end' }
  ]
}
`
// --- END EMBEDDED zon-grammar.jsonic ---

// Plugin implementation.
const Zon: Plugin = (tn: Tabnas, options: ZonOptions) => {
  const charAsNumber = !!options.charAsNumber
  const enumTag = options.enumTag || null

  // Human descriptions for the ZON tokens, surfaced in railroad diagram
  // legends (read off the live config by @tabnas/railroad). ZON uses Zig
  // anonymous-struct syntax: both maps and tuples open with `.{` and close
  // with `}`; bare { [ ] are disabled and keys are `.identifier` field names.
  tn.options({
    config: {
      modify: {
        'zon-tokendesc': (cfg: any) => {
          cfg.tokenDesc = Object.assign(cfg.tokenDesc || {}, {
            '#OB': '.{ — start of a struct (map)',
            '#OS': '.{ — start of a tuple (list)',
            '#CS': '} — end of a tuple (list)',
            'KEY': 'field name: .identifier or .@"..." (dot stripped)',
            'VAL': 'value: number, string, true/false/null, or .enum',
          })
        },
      },
    },
  })

  // If enumTag is set, wrap enum-literal values (produced by zonDot) into
  // `{ [enumTag]: name }` objects. The relaxed-JSON grammar takes ownership
  // of the `@val-bc` (val close) phase via `@val-bc/replace`, which sets
  // r.node from the matched token; once a phase is "replaced" the engine
  // suppresses any `/prepend` on it. So run in the `@val-ac` (after-close)
  // phase and rewrap the node the close handler produced from the enum token.
  const refs: Record<string, Function> = {
    '@val-ac': (r: Rule, _ctx: Context) => {
      if (!enumTag) return
      if (undefined !== r.child.node) return
      if (0 === r.os) return
      const tkn: any = r.o0
      if (tkn && tkn.use && tkn.use.zonEnum) {
        r.node = { [enumTag]: tkn.val }
      }
    },

    // Zig rejects a struct literal that repeats a field name. jsonic's own
    // `@pair-bc` performs the assignment (last one wins), so this guard has
    // to run *before* it — hence `/prepend`, which is available because
    // jsonic declares a plain `@pair-bc` rather than taking the phase with
    // `/replace` (contrast `@val-bc`, see the note above).
    '@pair-bc/prepend': (r: Rule, ctx: Context) => {
      if (!r.u.pair) return
      const node: any = r.node
      if (null == node || 'object' !== typeof node) return
      const key = r.u.key
      if (key in node) {
        const tkn: any = r.o0 || ctx.t0
        tkn.err = 'zon_dup_field'
        tkn.use = Object.assign(tkn.use || {}, { key })
        return tkn
      }
    },
  }

  const grammarDef = new Tabnas().use(jsonic).parse(grammarText)
  grammarDef.ref = refs
  // All jsonic option overrides live on the grammar object so the plugin
  // applies them atomically alongside its rule alts.
  grammarDef.options = {
    rule: {
      // Remove jsonic extensions (implicit maps/lists, top-level commas,
      // path dives). ZON uses explicit struct literals only.
      exclude: 'jsonic,imp',
      start: 'val',
    },
    fixed: {
      token: {
        // Bare `{`, `[`, `]` are not valid in ZON. Struct opening is `.{`
        // which is handled by the custom zonDot lex matcher below.
        '#OB': null,
        '#OS': null,
        '#CS': null,
        // `=` replaces `:` as the key/value separator.
        '#CL': '=',
      },
    },
    tokenSet: {
      // ZON field names are identifiers (`.ident` or `.@"..."`) only.
      //
      // The three trailing nulls are LOAD-BEARING. The engine overlays a
      // named token set onto the installed one BY INDEX, so a bare
      // `['#TX']` overwrites slot 0 and leaves the default `#NR`, `#ST`
      // and `#VL` live behind it -- and `.{ .a = 1, "b" = 2 }` parsed. A
      // `null` member clears its position. Pinned in test/zon.test.ts, and
      // honoured by the Rust port too since engine 0.12.3
      // (tabnas/parser#217).
      KEY: ['#TX', null, null, null],
    },
    // The engine's string matcher is off: `"..."` is lexed by the
    // zonString matcher below with Zig's escape set, which is narrower
    // than the relaxed-JSON one and refuses a surrogate `\u{...}`.
    string: {
      lex: false,
    },
    // jsonic's relaxed number lexer accepts `+1`, `.5`, `5.`, `0123`,
    // `1__0` and friends, none of which are ZON. The zonNumber matcher
    // below implements Zig's numeric literal grammar exactly instead.
    number: {
      lex: false,
    },
    error: {
      zon_number: 'invalid ZON number literal: {src}',
      zon_ident: 'invalid ZON identifier: {src}',
      zon_char: 'invalid ZON character literal: {src}',
      zon_doc_comment: 'doc comments are not allowed in ZON: {src}',
      zon_dup_field: 'duplicate struct field name: {src}',
    },
    // A hint for each code, keyed alike. Without one the engine falls back
    // to its hint for an UNKNOWN code, which tells the reader the error is
    // probably a bug in jsonic or a plugin.
    hint: {
      zon_number: 'A ZON number is a Zig literal, such as 0, 42, 1_000, 3.14, 1.e3, 0x2a,\n0x1.8p3, 0o17 or 0b101. After a lowercase base prefix every digit is of\nthat base, and there is at least one. A decimal integer part other\nthan 0 does not start with 0. Only decimal and hex numbers take a\nfraction or an exponent (e, or p for hex), and an exponent needs\ndigits. An underscore goes only between two digits. A negative zero is\nwritten -0.0, not -0, and nan takes no sign.',
      zon_ident: 'A ZON field name is .name, a letter or underscore then letters, digits\nand underscores, or .@"text" for any other name; the quoted form must\nnot be empty or hold an invalid escape.',
      zon_char: 'A ZON character literal holds exactly one character or escape between\nsingle quotes, as in \'a\', and names a code point no higher than U+10FFFF.',
      zon_doc_comment: 'ZON allows only plain // comments. Doc comments, /// and //!, belong to\nZig source; change them to //.',
      zon_dup_field: 'A struct literal may name each field only once, and {src} appears\nagain. Remove or rename the repeated field.',
    },
    // Only `//` line comments in ZON.
    comment: {
      lex: true,
      def: {
        hash: { lex: false },
        slash: { line: true, start: '//', lex: true, eatline: false },
        multi: { lex: false },
      },
    },
    value: {
      lex: true,
      def: {
        true: { val: true },
        false: { val: false },
        null: { val: null },
      },
    },
    // The default jsonic text matcher is disabled; identifiers are only
    // produced by the custom zonDot matcher below.
    text: {
      lex: false,
    },
    lex: {
      match: {
        zonDot: { order: 1e5, make: buildZonDotMatcher() },
        zonMultiString: { order: 1.1e5, make: buildZonMultiStringMatcher() },
        zonChar: { order: 1.2e5, make: buildZonCharMatcher(charAsNumber) },
        zonNumber: { order: 1.3e5, make: buildZonNumberMatcher() },
        // Must out-order jsonic's comment matcher (6e6) so `//!` and `///`
        // are rejected instead of being eaten as ordinary line comments.
        zonDocComment: { order: 1.4e5, make: buildZonDocCommentMatcher() },
        zonString: { order: 1.5e5, make: buildZonStringMatcher() },
      },
    },
  }

  // Tag every alt in this grammar with the 'zon' group so callers can
  // selectively exclude zon alts via `rule.exclude: 'zon'`.
  tn.grammar(grammarDef, { rule: { alt: { g: 'zon' } } })
}

// The text of the token being cut, read through the lexer's window, so that
// a parse from a reader may hold a window of the source rather than all of
// it (`lex.window`, admin ADR-30). While a token is cut, `lex.src` is the
// window's text and `lex.pnt` counts from the window's start, and the window
// may end inside the token. Every read here goes through `has`, which reads
// the window on where a read would pass its end, asking it to reach twice as
// far past the token's start as has been read, so a long token costs its
// length and not its square; the text it held keeps its positions, since the
// window drops nothing while a token is cut. So every read is of the text
// the whole source has at that place. In a whole text `lex.fill` only
// answers. The matchers that read through it run before every engine
// matcher (orders 1e5 to 1.5e5), so reading the window on here never hides
// from a matcher of the same cut what it would read.
class Source {
  private readonly lex: Lex
  private readonly from: number

  constructor(lex: Lex) {
    this.lex = lex
    this.from = lex.pnt.sI
  }

  // Whether the source has a character at `i`.
  has(i: number): boolean {
    const src: string = (this.lex as any).src
    if (i < src.length) return true
    const read = i - this.from
    ;(this.lex as any).fill?.(Math.max(read + 1, 2 * read))
    return i < (this.lex as any).src.length
  }

  // The UTF-16 unit at `i`, or undefined past the end of the source.
  at(i: number): string | undefined {
    return this.has(i) ? (this.lex as any).src[i] : undefined
  }

  // The unit's code at `i`, or NaN past the end of the source.
  code(i: number): number {
    return this.has(i) ? (this.lex as any).src.charCodeAt(i) : NaN
  }

  // The code point at `i`. The window holds whole characters, so a pair's
  // second half is held with its first.
  point(i: number): number | undefined {
    return this.has(i) ? (this.lex as any).src.codePointAt(i) : undefined
  }

  // The text from `a` to `b`, cut at the end of the source.
  text(a: number, b: number): string {
    this.has(b - 1)
    return (this.lex as any).src.substring(a, b)
  }

  // `end`, or the end of the source where it ends sooner.
  cap(end: number): number {
    return this.has(end - 1) ? end : (this.lex as any).src.length
  }
}

// Custom lex matcher for `.`-prefixed tokens.
//   `.{`            -> #OB if followed by `.ident =`, otherwise #OS
//   `.identifier`   -> #TX (val = identifier, use.zonEnum = true)
//   `.@"any name"`  -> #TX (val = the decoded string, use.zonEnum = true)
// Runs ahead of the fixed-token matcher so it reliably owns the `.` prefix.
function buildZonDotMatcher() {
  return function makeZonDotMatcher(cfg: Config, _opts: TabnasOptions) {
    return function zonDotMatcher(lex: Lex) {
      const { pnt } = lex
      const src = new Source(lex)
      const { sI, cI, rI } = pnt
      if ('.' !== src.at(sI)) return undefined

      // Zig's tokenizer emits `.` and what follows as separate tokens, so
      // whitespace and comments may sit between them (`. foo`, `. {}`).
      const pos = skipInsigPos(cfg, src, sI + 1, rI, cI + 1)
      const dI = pos.i

      // Advance pnt past a token running from sI to `end`, given that the
      // insignificant run up to dI may have crossed rows.
      const advance = (end: number) => {
        pnt.sI = end
        pnt.rI = pos.rI
        pnt.cI = pos.cI + (end - dI)
      }

      // `.{` opens a struct literal. Decide map vs list by peeking ahead.
      if ('{' === src.at(dI)) {
        const isMap = peekIsMapOpen(cfg, src, dI + 1)
        const tin = isMap ? '#OB' : '#OS'
        const tkn = lex.token(tin, undefined, src.text(sI, dI + 1), pnt)
        advance(dI + 1)
        return tkn
      }

      // `.@"..."` - escaped identifier: any string may name a field or an
      // enum literal. Zig rejects an empty one (`.@""`).
      if ('@' === src.at(dI) && '"' === src.at(dI + 1)) {
        const dec = decodeZigString(src, dI + 2)
        // Zig rejects an empty identifier and one containing a null byte.
        if (null === dec || '' === dec.val || -1 !== dec.val.indexOf('\u0000')) {
          return lex.bad('zon_ident', sI, null === dec ? dI + 2 : dec.end)
        }
        const srcText = src.text(sI, dec.end)
        const tkn = lex.token('#TX', dec.val, srcText, pnt, { zonEnum: true })
        advance(dec.end)
        return tkn
      }

      // `.identifier` - field name or enum literal.
      if (!isIdStart(src.code(dI))) return undefined
      let eI = dI
      while (src.has(eI) && isIdCont(src.code(eI))) eI++

      const srcText = src.text(sI, eI)
      const name = src.text(dI, eI)
      const tkn = lex.token('#TX', name, srcText, pnt, { zonEnum: true })
      advance(eI)
      return tkn
    }
  }
}

// Skip whitespace, newlines and `//` line comments, tracking the row/column
// so a token that follows still reports an accurate source position.
function skipInsigPos(
  cfg: Config,
  src: Source,
  i: number,
  rI: number,
  cI: number,
): { i: number; rI: number; cI: number } {
  while (src.has(i)) {
    const c = src.at(i) as string
    if (cfg.line.chars[c]) {
      if (cfg.line.rowChars[c]) rI++
      cI = 1
      i++
    } else if (' ' === c || '\t' === c) {
      i++
      cI++
    } else if ('/' === c && '/' === src.at(i + 1)) {
      while (src.has(i) && !cfg.line.chars[src.at(i) as string]) {
        i++
        cI++
      }
    } else {
      break
    }
  }
  return { i, rI, cI }
}

// Returns true if the position inside `.{ ... }` starts with a field name
// (`.ident` or `.@"..."`) followed by `=`, indicating a struct/map literal
// rather than a tuple/list.
function peekIsMapOpen(cfg: Config, src: Source, start: number): boolean {
  let i = skipInsig(cfg, src, start)
  if ('.' !== src.at(i)) return false
  i = skipInsig(cfg, src, i + 1)
  if ('@' === src.at(i) && '"' === src.at(i + 1)) {
    const dec = decodeZigString(src, i + 2)
    if (null === dec) return false
    i = dec.end
  } else {
    if (!isIdStart(src.code(i))) return false
    i++
    while (src.has(i) && isIdCont(src.code(i))) i++
  }
  i = skipInsig(cfg, src, i)
  return '=' === src.at(i)
}

// Decode a Zig double-quoted string body starting at `i` (just past the
// opening quote). Returns the decoded value and the index just past the
// closing quote, or null when the literal is malformed. The `.@"..."`
// identifier form, which Zig lexes with the same rules as a string.
function decodeZigString(
  src: Source,
  i: number,
): { val: string; end: number } | null {
  const scan = scanZigString(src, i)
  return undefined === scan.err ? { val: scan.val, end: scan.end } : null
}

// The result of scanning a `"..."` body: the value and the index past the
// closing quote, or, when `err` is set, the ENGINE's error code for the
// fault (`unterminated_string`, `unprintable`, `invalid_unicode`,
// `invalid_ascii`, `unexpected`) and the index just past it, so the
// message can quote the literal from its opening quote up to the fault
// and a caller branching on the code sees what the engine's own string
// matcher would have said.
type ZigStringScan = { val: string; end: number; err?: string }

// Scan a Zig double-quoted string body starting at `start` (just past the
// opening quote). Shared by the `"..."` string matcher and the `.@"..."`
// identifier form.
//
// The escape set is Zig's and nothing wider: `\n`, `\r`, `\t`, `\\`, `\'`,
// `\"`, `\xNN` and `\u{...}`. A `\u{...}` must name a Unicode SCALAR value,
// so the surrogate block is refused along with anything above U+10FFFF.
// Measured against the pinned zig 0.16.0 oracle, which answers `"\u{D800}"`
// and `.@"\u{D800}"` alike with "unicode escape does not correspond to a
// valid unicode scalar value"; a CHARACTER literal is an integer in Zig and
// does accept a surrogate, so buildZonCharMatcher deliberately does not
// share this test. A run of `\xNN` escapes is a run of BYTES, as it is in
// Zig, decoded as UTF-8 once the run ends: `"\xe2\x82\xac"` is the euro
// sign, and an ill-formed sequence becomes one U+FFFD per maximal subpart,
// as TextDecoder decodes it. A raw control character, a line end
// included, is not a string character.
function scanZigString(src: Source, start: number): ZigStringScan {
  let out = ''
  // The bytes of consecutive `\xNN` escapes, decoded together.
  let pending: number[] = []
  const flush = () => {
    if (0 < pending.length) {
      out += new TextDecoder().decode(Uint8Array.from(pending))
      pending = []
    }
  }
  const fault = (err: string, end: number): ZigStringScan =>
    ({ val: '', end: src.cap(end), err })

  let i = start
  while (src.has(i)) {
    const c = src.code(i)
    if (34 === c) {
      flush()
      return { val: out, end: i + 1 }
    }
    if (10 === c || 13 === c) return fault('unterminated_string', i)
    if (c < 0x20 || 0x7f === c) return fault('unprintable', i + 1)
    if (92 !== c) {
      flush()
      out += src.at(i)
      i++
      continue
    }
    const e = src.at(i + 1)
    if (undefined === e) return fault('unterminated_string', i + 1)
    if ('x' === e) {
      const hex = src.text(i + 2, i + 4)
      if (!/^[0-9a-fA-F]{2}$/.test(hex)) return fault('invalid_ascii', i + 4)
      pending.push(parseInt(hex, 16))
      i += 4
    } else if ('u' === e) {
      if ('{' !== src.at(i + 2)) return fault('invalid_unicode', i + 3)
      let close = i + 3
      while (src.has(close) && -1 !== digitVal(src.at(close) as string)) close++
      if (close === i + 3 || '}' !== src.at(close)) {
        return fault('invalid_unicode', close + 1)
      }
      const cp = parseInt(src.text(i + 3, close), 16)
      if (0x10ffff < cp || (0xd800 <= cp && cp <= 0xdfff)) {
        return fault('invalid_unicode', close + 1)
      }
      flush()
      out += String.fromCodePoint(cp)
      i = close + 1
    } else if (undefined !== ZIG_ESCAPE[e]) {
      flush()
      out += ZIG_ESCAPE[e]
      i += 2
    } else {
      return fault('unexpected', i + 2)
    }
  }
  return fault('unterminated_string', src.cap(i))
}

// Zig double-quoted strings, `"..."`, with Zig's escape set and no other.
// The engine's own string matcher is switched off (`string.lex: false`):
// its relaxed-JSON escapes (`\b`, `\f`, `\v`, `\/`, `\uXXXX`) and its
// acceptance of a surrogate `\u{...}` are not ZON, and the pinned zig
// oracle rejects every one of them. The token is `#ST`, as the engine's
// would be, and a fault carries the engine's code for it.
function buildZonStringMatcher() {
  return function makeZonStringMatcher(_cfg: Config, _opts: TabnasOptions) {
    return function zonStringMatcher(lex: Lex) {
      const { pnt } = lex
      const src = new Source(lex)
      const { sI, cI } = pnt
      if ('"' !== src.at(sI)) return undefined

      const scan = scanZigString(src, sI + 1)
      if (undefined !== scan.err) return lex.bad(scan.err, sI, scan.end)

      const tkn = lex.token('#ST', scan.val, src.text(sI, scan.end), pnt)
      pnt.sI = scan.end
      pnt.cI = cI + (scan.end - sI)
      return tkn
    }
  }
}

// The single-character Zig escapes (the `\xNN` and `\u{...}` forms are
// handled positionally by scanZigString).
const ZIG_ESCAPE: Record<string, string> = {
  n: '\n',
  r: '\r',
  t: '\t',
  '\\': '\\',
  '\'': '\'',
  '"': '"',
}

// Skip whitespace, newlines, and `//` line comments (position only).
function skipInsig(cfg: Config, src: Source, i: number): number {
  return skipInsigPos(cfg, src, i, 1, 1).i
}

function isIdStart(c: number): boolean {
  return (
    (65 <= c && c <= 90) || // A-Z
    (97 <= c && c <= 122) || // a-z
    95 === c // _
  )
}

function isIdCont(c: number): boolean {
  return (
    (48 <= c && c <= 57) || // 0-9
    (65 <= c && c <= 90) ||
    (97 <= c && c <= 122) ||
    95 === c
  )
}

// Multi-line Zig strings: consecutive lines starting with `\\`.
// Each `\\` line contributes its content verbatim (after the `\\`); lines
// are joined with `\n`.
function buildZonMultiStringMatcher() {
  return function makeZonMultiStringMatcher(cfg: Config, _opts: TabnasOptions) {
    return function zonMultiStringMatcher(lex: Lex) {
      const { pnt } = lex
      const src = new Source(lex)
      if ('\\' !== src.at(pnt.sI) || '\\' !== src.at(pnt.sI + 1)) return undefined

      const startI = pnt.sI
      const startCI = pnt.cI
      let sI = pnt.sI
      let rI = pnt.rI
      const parts: string[] = []

      while ('\\' === src.at(sI) && '\\' === src.at(sI + 1)) {
        sI += 2
        const lineStart = sI
        while (src.has(sI) && !cfg.line.chars[src.at(sI) as string]) sI++
        parts.push(src.text(lineStart, sI))

        // Consume line terminator (handle \r\n as one).
        if (src.has(sI) && cfg.line.chars[src.at(sI) as string]) {
          const ch = src.at(sI) as string
          if (cfg.line.rowChars[ch]) rI++
          sI++
          if (src.has(sI) && '\r' === ch && '\n' === src.at(sI)) sI++
        }

        // Look for another `\\` continuation. Zig's tokenizer treats the
        // whole run of `\\` lines as ONE token and skips the whitespace
        // between them, so blank lines in the middle continue the literal
        // (and contribute nothing) rather than ending it.
        let peek = sI
        let peekR = 0
        while (src.has(peek)) {
          const pc = src.at(peek) as string
          if (' ' === pc || '\t' === pc) {
            peek++
          } else if (cfg.line.chars[pc]) {
            if (cfg.line.rowChars[pc]) peekR++
            peek++
          } else {
            break
          }
        }
        if ('\\' !== src.at(peek) || '\\' !== src.at(peek + 1)) break
        sI = peek
        rI += peekR
      }

      const val = parts.join('\n')
      const tsrc = src.text(startI, sI)
      const tkn = lex.token('#ST', val, tsrc, pnt)
      pnt.sI = sI
      pnt.rI = rI
      pnt.cI = startCI + (sI - startI)
      return tkn
    }
  }
}

// Zig character literal: `'x'`, `'\n'`, `'\x41'`, `'\u{1F600}'`.
// Produces a numeric code point (if charAsNumber) or a one-char string.
// Unlike a string, a character literal is an INTEGER in Zig: `'\xD8'` is
// 216 and `'\u{D800}'` is 55296, so neither goes through scanZigString.
function buildZonCharMatcher(charAsNumber: boolean) {
  return function makeZonCharMatcher(_cfg: Config, _opts: TabnasOptions) {
    return function zonCharMatcher(lex: Lex) {
      const { pnt } = lex
      const src = new Source(lex)
      const { sI, cI } = pnt
      if ('\'' !== src.at(sI)) return undefined

      let i = sI + 1
      let codepoint: number | null = null

      if ('\\' === src.at(i)) {
        i++
        const esc = src.at(i)
        switch (esc) {
          case 'n': codepoint = 10; i++; break
          case 'r': codepoint = 13; i++; break
          case 't': codepoint = 9; i++; break
          case '\\': codepoint = 92; i++; break
          case '\'': codepoint = 39; i++; break
          case '"': codepoint = 34; i++; break
          // `\0` is NOT a Zig escape (`'\x00'` and `'\u{0}'` are): the
          // pinned zig 0.16.0 oracle answers `'\0'` with "invalid escape
          // character: '0'".
          case 'x': {
            i++
            const hex = src.text(i, i + 2)
            if (!/^[0-9a-fA-F]{2}$/.test(hex)) return undefined
            codepoint = parseInt(hex, 16)
            i += 2
            break
          }
          case 'u': {
            i++
            if ('{' !== src.at(i)) return undefined
            i++
            // The hex digits and the `}` they end on. A run that ends on
            // anything else is no character literal: the first `}` after
            // `{`, wherever it is, closes a run that is not all hex digits.
            let endI = i
            while (src.has(endI) && -1 !== digitVal(src.at(endI) as string)) endI++
            if ('}' !== src.at(endI)) return undefined
            const hex = src.text(i, endI)
            if (!/^[0-9a-fA-F]+$/.test(hex)) return undefined
            codepoint = parseInt(hex, 16)
            // Zig: a character literal is an INTEGER, so the escape names
            // a code point rather than a scalar value: U+10FFFF is the
            // only bound, and a lone surrogate is accepted. Measured: the
            // pinned zig 0.16.0 oracle answers `'\u{D800}'` with 55296.
            if (0x10ffff < codepoint) {
              return lex.bad('zon_char', sI, src.cap(endI + 2))
            }
            i = endI + 1
            break
          }
          default:
            return undefined
        }
      } else if (src.has(i) && '\'' !== src.at(i)) {
        codepoint = src.point(i) as number
        // A raw control character (notably a literal newline) is not a
        // character literal in Zig — it is an invalid token.
        if (codepoint < 0x20 || 0x7f === codepoint) {
          return lex.bad('zon_char', sI, i + 1)
        }
        i += codepoint > 0xffff ? 2 : 1
      } else {
        return undefined
      }

      if ('\'' !== src.at(i)) return undefined
      i++

      const tsrc = src.text(sI, i)
      const val = charAsNumber ? codepoint : String.fromCodePoint(codepoint!)
      const tkn = lex.token('#NR', val, tsrc, pnt)
      pnt.sI = i
      pnt.cI = cI + (i - sI)
      return tkn
    }
  }
}

// `//!` and `///` are Zig DOC comments, which ZON rejects outright ("expected
// expression, found 'a document comment'"). `////` and longer runs are plain
// line comments. This matcher only ever fails the lex: an ordinary `//`
// comment falls through to jsonic's comment matcher.
function buildZonDocCommentMatcher() {
  return function makeZonDocCommentMatcher(_cfg: Config, _opts: TabnasOptions) {
    return function zonDocCommentMatcher(lex: Lex) {
      const { pnt } = lex
      const src = new Source(lex)
      const sI = pnt.sI
      if ('/' !== src.at(sI) || '/' !== src.at(sI + 1)) return undefined
      const c = src.at(sI + 2)
      if ('!' === c || ('/' === c && '/' !== src.at(sI + 3))) {
        return lex.bad('zon_doc_comment', sI, sI + 3)
      }
      return undefined
    }
  }
}

// Zig numeric literals, as ZON defines them. This replaces jsonic's relaxed
// number lexer (`number.lex: false`) because that one happily accepts `+1`,
// `.5`, `5.`, `0123`, `00`, `1__0` and `0x_2A`, none of which are ZON.
//
// Accepted: decimal / `0x` / `0o` / `0b` integers with `_` separators between
// digits, decimal and hexadecimal floats (`1.5e3`, `0x1.8p1`, `0x103.70`),
// the `inf` and `nan` keywords, and a leading `-` on any of those except
// `nan`. Integers whose exact value is not representable as an IEEE-754
// double are returned as a `bigint` rather than silently rounded.
function buildZonNumberMatcher() {
  return function makeZonNumberMatcher(_cfg: Config, _opts: TabnasOptions) {
    return function zonNumberMatcher(lex: Lex) {
      const { pnt } = lex
      const src = new Source(lex)
      const { sI, cI } = pnt

      let i = sI
      let neg = false
      if ('-' === src.at(sI)) {
        neg = true
        i++
        while (' ' === src.at(i) || '\t' === src.at(i)) i++
      }

      const c = src.at(i)
      if (undefined === c) return undefined

      // The `inf` / `nan` keywords. Zig allows `-inf` but not `-nan`.
      if ('i' === c || 'n' === c) {
        const kw = src.text(i, i + 3)
        if (('inf' === kw || 'nan' === kw) && !isIdCont(src.code(i + 3))) {
          if ('nan' === kw && neg) return lex.bad('zon_number', sI, i + 3)
          const val = 'inf' === kw ? (neg ? -Infinity : Infinity) : NaN
          const end = i + 3
          const tkn = lex.token('#NR', val, src.text(sI, end), pnt)
          pnt.sI = end
          pnt.cI = cI + (end - sI)
          return tkn
        }
        return undefined
      }

      if (!isDigit(c.charCodeAt(0))) return undefined

      const num = scanZonNumber(src, i)
      if (num.err) return lex.bad('zon_number', sI, num.end)

      let val: any
      if (num.isInt) {
        const big = num.big as bigint
        // Zig: `-0` is an ambiguous integer literal (`-0.0` is fine).
        if (neg && 0n === big) return lex.bad('zon_number', sI, num.end)
        const signed = neg ? -big : big
        const asNum = Number(signed)
        val =
          Number.isFinite(asNum) && BigInt(asNum) === signed ? asNum : signed
      } else {
        val = neg ? -(num.num as number) : (num.num as number)
      }

      const end = num.end
      const tkn = lex.token('#NR', val, src.text(sI, end), pnt)
      pnt.sI = end
      pnt.cI = cI + (end - sI)
      return tkn
    }
  }
}

type ZonNumScan = {
  end: number
  err?: boolean
  isInt?: boolean
  big?: bigint
  num?: number
}

// Scan one unsigned Zig numeric literal starting at `start` (a digit).
function scanZonNumber(src: Source, start: number): ZonNumScan {
  let i = start
  let base = 10
  const fail = (): ZonNumScan => ({ end: scanNumTokenEnd(src, start), err: true })

  if ('0' === src.at(i)) {
    const c1 = src.at(i + 1)
    if ('x' === c1 || 'o' === c1 || 'b' === c1) {
      base = 'x' === c1 ? 16 : 'o' === c1 ? 8 : 2
      i += 2
    } else if ('X' === c1 || 'O' === c1 || 'B' === c1) {
      return fail() // base prefix must be lowercase
    } else if (undefined !== c1 && ('_' === c1 || isDigit(c1.charCodeAt(0)))) {
      return fail() // leading zero
    }
  }

  const intRun = scanDigitRun(src, i, base)
  if (intRun.bad) return fail()
  const intText = src.text(i, intRun.end).replace(/_/g, '')
  i = intRun.end

  let isFloat = false
  let fracText = ''
  if ('.' === src.at(i)) {
    const after = src.at(i + 1)
    const afterDV = undefined === after ? -1 : digitVal(after)
    // A `.` only starts a fraction when a digit of this base, or this
    // base's exponent letter, follows; otherwise it is a stray token and
    // the number ends here — `1.` and `0.1.2` are rejected by the parser.
    // The fraction itself may then be EMPTY: zig reads `1.e3` and `0xF.p1`
    // as one float token each, and both are in the corpora.
    const startsFrac =
      (0 <= afterDV && afterDV < base) ||
      (16 === base && ('p' === after || 'P' === after)) ||
      (10 === base && ('e' === after || 'E' === after))
    if (startsFrac) {
      if (16 !== base && 10 !== base) return fail() // invalid base for float
      isFloat = true
      i++
      const fracRun = scanDigitRun(src, i, base)
      if (fracRun.bad) return fail()
      fracText = src.text(i, fracRun.end).replace(/_/g, '')
      i = fracRun.end
    }
  }

  let expVal = 0
  let hasExp = false
  const expChar = 16 === base ? 'pP' : 10 === base ? 'eE' : ''
  const x = src.at(i)
  if ('' !== expChar && undefined !== x && -1 !== expChar.indexOf(x)) {
    hasExp = true
    isFloat = true
    i++
    let expSign = 1
    if ('+' === src.at(i) || '-' === src.at(i)) {
      if ('-' === src.at(i)) expSign = -1
      i++
    }
    const expRun = scanDigitRun(src, i, 10)
    if (expRun.bad || 0 === expRun.count) return fail()
    expVal = expSign * parseInt(src.text(i, expRun.end).replace(/_/g, ''), 10)
    i = expRun.end
  }

  if (0 === intRun.count && 0 === fracText.length) return fail()

  // Anything alphanumeric still attached is a digit invalid for this base.
  if (isIdCont(src.code(i))) return fail()

  if (isFloat) {
    let num: number
    if (16 === base) {
      const mant = Number(BigInt('0x' + ((intText || '0') + fracText)))
      num = mant * Math.pow(2, expVal - 4 * fracText.length)
    } else {
      num = parseFloat(
        (intText || '0') +
        (0 < fracText.length ? '.' + fracText : '') +
        (hasExp ? 'e' + expVal : ''),
      )
    }
    return { end: i, num }
  }

  const prefix = 16 === base ? '0x' : 8 === base ? '0o' : 2 === base ? '0b' : ''
  return { end: i, isInt: true, big: BigInt(prefix + (intText || '0')) }
}

// Scan a run of `base` digits with Zig's digit-separator rules: a `_` must
// sit directly between two digits (no leading, trailing, or repeated `_`).
function scanDigitRun(
  src: Source,
  i: number,
  base: number,
): { end: number; count: number; bad: boolean } {
  const start = i
  let count = 0
  let prevDigit = false
  for (; src.has(i); i++) {
    const ch = src.at(i) as string
    if ('_' === ch) {
      if (!prevDigit) return { end: i, count, bad: true }
      prevDigit = false
      continue
    }
    const dv = digitVal(ch)
    if (0 <= dv && dv < base) {
      count++
      prevDigit = true
      continue
    }
    break
  }
  if (start < i && !prevDigit) return { end: i, count, bad: true }
  return { end: i, count, bad: false }
}

// The greedy extent of a malformed numeric token, so the error span covers
// the whole literal rather than its first character.
function scanNumTokenEnd(src: Source, i: number): number {
  while (src.has(i)) {
    const c = src.code(i)
    if (isIdCont(c)) {
      i++
    } else if (46 === c && src.has(i + 1) && isIdCont(src.code(i + 1))) {
      i++
    } else {
      break
    }
  }
  return i
}

function isDigit(c: number): boolean {
  return 48 <= c && c <= 57
}

function digitVal(ch: string): number {
  const c = ch.charCodeAt(0)
  if (48 <= c && c <= 57) return c - 48
  if (97 <= c && c <= 102) return c - 87
  if (65 <= c && c <= 70) return c - 55
  return -1
}

// Default option values.
Zon.defaults = {
  charAsNumber: false,
  enumTag: null,
} as ZonOptions

// Build a ZON parser whole: jsonic's base with the plugin installed, as
// `new Tabnas().use(jsonic).use(Zon, options)` builds it, and with
// `lex.window` declared, so that `tn.parseReader` reads a reader a window at
// a time rather than whole (admin ADR-30). The matchers this grammar
// installs read the source only through the lexer's window, and jsonic's
// base is the engine's own matchers. The window is declared here, on the
// parser this package makes whole, and not in `Zon`, which a host may
// install beneath matchers of its own.
function make(options: Partial<ZonOptions> = {}): Tabnas {
  const tn = new Tabnas().use(jsonic).use(Zon, options)
  tn.options({ lex: { window: true } })
  return tn
}

export { Zon, VERSION, make }
export type { ZonOptions }

export { translate } from './translate'
export type { TranslationPart, TranslationParts } from './translate'

// VERSION is this package's version. It MUST equal package.json "version":
// the release orchestrator rewrites both, and the version test fails the
// build if they drift. Mirrors `const VERSION` in go/zon.go.
const VERSION = '0.5.18'
