/* Copyright (c) 2025-2026 Richard Rodger, MIT License */

//! The six custom lex matchers, ported from `ts/src/zon.ts`, for Zig
//! syntax the relaxed-JSON lexer cannot express, or would wrongly accept.
//!
//! They are registered by name on the instance and named from the
//! grammar document's `options.lex.match`, with the orders the canonical
//! runtime gives them: all below the engine's first built-in band (1e6),
//! so `zonDot` owns the `.` prefix ahead of the fixed-token matcher,
//! `zonString` owns `"` ahead of the engine's string matcher (which is
//! switched off besides) and `zonDocComment` sees `//!` and `///` before
//! the comment matcher eats them.
//!
//! Every matcher reads the text the lexer holds, from the cursor, by byte
//! index, through [`Text`]: in a parse from a reader that is the window,
//! which may end inside the token (`lex.window`, admin ADR-30), and a cut
//! that read past its end is made again over a window read on twice as
//! far ([`windowed`]), so it is the cut the whole source gives. The only
//! non-ASCII bytes any of them cares about are whole characters (a raw
//! character literal, the body of a string or a `.@"..."` identifier),
//! and those are decoded as characters; everything else is compared
//! against ASCII, so a byte index is always a character boundary where
//! it is used to slice. Cursor advancement goes through [`Lexer::advance_chars`], which
//! keeps the row and column honest across the newlines a token may span,
//! the bookkeeping the TypeScript `advance` helpers do by hand.

use std::cell::Cell;

use indexmap::IndexMap;
use tabnas::{Context, Lexer, Rule, Tabnas, Token, Value, TIN_NR, TIN_OB, TIN_OS, TIN_ST, TIN_TX};

use crate::number::{self, is_hex, is_id_cont, is_id_start, BigUint, Scanned};

/// The function references the grammar document names.
pub(crate) const DOT: &str = "@zonDot";
pub(crate) const MULTI_STRING: &str = "@zonMultiString";
pub(crate) const CHAR: &str = "@zonChar";
pub(crate) const NUMBER: &str = "@zonNumber";
pub(crate) const DOC_COMMENT: &str = "@zonDocComment";
pub(crate) const STRING: &str = "@zonString";

/// The token detail the `zonDot` matcher sets on an identifier token, so
/// the enum-tag hook can tell `.foo` from a string.
pub(crate) const ENUM_MARK: &str = "zonEnum";

/// Register the matchers on `parser`, ahead of the grammar document that
/// names them. `char_as_number` is baked into the character matcher, as
/// the TypeScript `buildZonCharMatcher(charAsNumber)` bakes it in.
pub(crate) fn register(parser: &mut Tabnas, char_as_number: bool) {
    parser.imperative_lex_match_ref(DOT, dot_matcher);
    parser.imperative_lex_match_ref(MULTI_STRING, multi_string_matcher);
    parser.imperative_lex_match_ref(CHAR, move |lexer, _rule, _context| {
        char_matcher(lexer, char_as_number)
    });
    parser.imperative_lex_match_ref(NUMBER, number_matcher);
    parser.imperative_lex_match_ref(DOC_COMMENT, doc_comment_matcher);
    parser.imperative_lex_match_ref(STRING, string_matcher);
}

// ---------------------------------------------------------------------------
// Reading through the window
// ---------------------------------------------------------------------------

/// The text the lexer holds from the cursor, as a matcher reads it: the
/// whole source, or in a parse from a reader the window, which may end
/// inside the token. A read past what it holds answers as the end of the
/// source would, and notes that the text may be short ([`Text::is_short`]),
/// so that [`windowed`] makes the cut again over a wider window.
pub(crate) struct Text<'a> {
    text: &'a str,
    short: Cell<bool>,
}

impl<'a> Text<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Text {
            text,
            short: Cell::new(false),
        }
    }

    /// Whether a read passed the end of the text.
    fn is_short(&self) -> bool {
        self.short.get()
    }

    /// Byte `i`, or `None` past what the text holds.
    pub(crate) fn byte(&self, i: usize) -> Option<u8> {
        let found = self.text.as_bytes().get(i).copied();
        if found.is_none() {
            self.short.set(true);
        }
        found
    }

    /// The text from byte `a` to byte `b`, or `None` where it ends sooner
    /// or the span is not on character boundaries.
    pub(crate) fn get(&self, a: usize, b: usize) -> Option<&'a str> {
        if b > self.text.len() {
            self.short.set(true);
            return None;
        }
        self.text.get(a..b)
    }

    /// The text from byte `a` to byte `b`, which the caller has read.
    pub(crate) fn slice(&self, a: usize, b: usize) -> &'a str {
        &self.text[a..b]
    }

    /// `end`, or the end of the text where it ends sooner.
    pub(crate) fn cap(&self, end: usize) -> usize {
        if end > self.text.len() {
            self.short.set(true);
            return self.text.len();
        }
        end
    }

    /// The character starting at byte `i`.
    pub(crate) fn char_at(&self, i: usize) -> Option<char> {
        self.byte(i)?;
        self.text[i..].chars().next()
    }

    /// Whether the text holds `pattern` at byte `i`.
    pub(crate) fn holds(&self, i: usize, pattern: &[u8]) -> bool {
        pattern
            .iter()
            .enumerate()
            .all(|(k, &b)| self.byte(i + k) == Some(b))
    }
}

/// What a matcher found at the cursor.
enum Cut {
    /// No token of this matcher's starts here.
    None,
    /// A token over the first `len` bytes.
    Emit {
        name: &'static str,
        tin: tabnas::Tin,
        value: Value,
        len: usize,
        enum_mark: bool,
    },
    /// A fault, quoting the byte span `start..end`.
    Bad {
        code: &'static str,
        start: usize,
        end: usize,
    },
}

/// Run a matcher's `cut` over the text the lexer holds. Where the cut
/// read past the window's end, the window is read on to twice what it
/// held, so a long token costs its length to read and not its square, and
/// the cut is made again, until no read passes the window's end or the
/// source has ended: the cut the whole source gives. The matchers run
/// before every engine matcher (orders 1e5 to 1.5e5), so reading the
/// window on here never hides from a matcher of the same cut what it
/// would read. In a whole text the window is all of it.
fn windowed(lexer: &mut Lexer<'_>, cut: impl Fn(&Text<'_>) -> Cut) -> Option<Token> {
    loop {
        let window = lexer.window();
        let text = Text::new(window);
        let found = cut(&text);
        if text.is_short() {
            let held = window.chars().count();
            let before = window.len();
            lexer.fill(held.saturating_mul(2).max(held + 1));
            if lexer.window().len() > before {
                continue;
            }
        }
        return match found {
            Cut::None => None,
            Cut::Emit {
                name,
                tin,
                value,
                len,
                enum_mark,
            } => Some(emit(lexer, name, tin, value, len, enum_mark)),
            Cut::Bad { code, start, end } => Some(bad(lexer, code, start, end)),
        };
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Whether `c` is one of the configured line characters (`\n` and `\r`
/// by default). They are ASCII, so a byte test is a character test.
fn is_line(line_chars: &str, c: u8) -> bool {
    c.is_ascii() && line_chars.contains(c as char)
}

/// Build a token over the first `len` bytes of the remaining source and
/// move the cursor past them.
fn emit(
    lexer: &mut Lexer<'_>,
    name: &str,
    tin: tabnas::Tin,
    value: Value,
    len: usize,
    enum_mark: bool,
) -> Token {
    let point = lexer.point();
    let source = lexer.remaining()[..len].to_string();
    let mut token = lexer.token(name, tin, value, source.as_str(), point);
    if enum_mark {
        token
            .use_data_mut()
            .insert(ENUM_MARK.to_string(), Value::Bool(true));
    }
    lexer.advance_chars(source.chars().count());
    token
}

/// A bad token whose displayed source is the byte span `start..end` of
/// the remaining source, so the message can quote the whole offending
/// literal. Clamped the way the Go `zonBad` clamps: never past the end,
/// never empty.
fn bad(lexer: &Lexer<'_>, code: &str, start: usize, end: usize) -> Token {
    let remaining = lexer.remaining();
    let mut end = end.min(remaining.len());
    if end <= start {
        end = (start + 1).min(remaining.len());
    }
    while !remaining.is_char_boundary(end) {
        end += 1;
    }
    let base = lexer.point().site.pos;
    let from = base + remaining[..start].chars().count();
    let to = base + remaining[..end].chars().count();
    lexer.bad_span(code, from, to)
}

/// Skip whitespace, newlines and `//` line comments from byte `i`,
/// returning where the next significant byte starts. Zig's tokenizer
/// emits `.` and what follows as separate tokens, so any of this may sit
/// between them (`. foo`, `. {}`).
fn skip_insig(text: &Text<'_>, mut i: usize, line_chars: &str) -> usize {
    while let Some(c) = text.byte(i) {
        if is_line(line_chars, c) || c == b' ' || c == b'\t' {
            i += 1;
        } else if c == b'/' && text.byte(i + 1) == Some(b'/') {
            while text.byte(i).is_some_and(|c| !is_line(line_chars, c)) {
                i += 1;
            }
        } else {
            break;
        }
    }
    i
}

// ---------------------------------------------------------------------------
// zonDot
// ---------------------------------------------------------------------------

/// `.`-prefixed tokens:
///
/// - `.{` is `#OB` when `.ident =` follows, otherwise `#OS`;
/// - `.identifier` is `#TX` with the dot stripped and the enum mark set;
/// - `.@"any name"` is `#TX` holding the decoded string, likewise marked.
fn dot_matcher(lexer: &mut Lexer<'_>, _rule: &mut Rule, context: &mut Context) -> Option<Token> {
    let line_chars = context.options.line.chars.as_str();
    windowed(lexer, |text| cut_dot(text, line_chars))
}

fn cut_dot(text: &Text<'_>, line_chars: &str) -> Cut {
    if text.byte(0) != Some(b'.') {
        return Cut::None;
    }
    let d = skip_insig(text, 1, line_chars);
    let at_d = text.byte(d);

    // `.{` opens a struct literal. Decide map vs list by peeking ahead.
    if at_d == Some(b'{') {
        let (name, tin) = if peek_is_map_open(text, d + 1, line_chars) {
            ("#OB", TIN_OB)
        } else {
            ("#OS", TIN_OS)
        };
        return Cut::Emit {
            name,
            tin,
            value: Value::Undefined,
            len: d + 1,
            enum_mark: false,
        };
    }

    // `.@"..."`: any string may name a field or an enum literal. Zig
    // rejects an empty one and one containing a NUL.
    if at_d == Some(b'@') && text.byte(d + 1) == Some(b'"') {
        return match decode_zig_string(text, d + 2) {
            Some((value, end)) if !value.is_empty() && !value.contains('\0') => Cut::Emit {
                name: "#TX",
                tin: TIN_TX,
                value: Value::String(value),
                len: end,
                enum_mark: true,
            },
            Some((_, end)) => Cut::Bad {
                code: "zon_ident",
                start: 0,
                end,
            },
            None => Cut::Bad {
                code: "zon_ident",
                start: 0,
                end: d + 2,
            },
        };
    }

    // `.identifier`: field name or enum literal.
    let Some(first) = at_d else {
        return Cut::None;
    };
    if !is_id_start(first) {
        return Cut::None;
    }
    let mut e = d;
    while text.byte(e).is_some_and(is_id_cont) {
        e += 1;
    }
    Cut::Emit {
        name: "#TX",
        tin: TIN_TX,
        value: Value::String(text.slice(d, e).to_string()),
        len: e,
        enum_mark: true,
    }
}

/// Whether the source inside `.{ ... }`, from byte `start`, begins with a
/// field name (`.ident` or `.@"..."`) followed by `=`: a struct literal
/// rather than a tuple.
fn peek_is_map_open(text: &Text<'_>, start: usize, line_chars: &str) -> bool {
    let mut i = skip_insig(text, start, line_chars);
    if text.byte(i) != Some(b'.') {
        return false;
    }
    i = skip_insig(text, i + 1, line_chars);
    if text.byte(i) == Some(b'@') && text.byte(i + 1) == Some(b'"') {
        match decode_zig_string(text, i + 2) {
            Some((_, end)) => i = end,
            None => return false,
        }
    } else {
        if !text.byte(i).is_some_and(is_id_start) {
            return false;
        }
        i += 1;
        while text.byte(i).is_some_and(is_id_cont) {
            i += 1;
        }
    }
    i = skip_insig(text, i, line_chars);
    text.byte(i) == Some(b'=')
}

/// Decode a Zig double-quoted string body starting at byte `i`, just past
/// the opening quote: the decoded value and the byte just past the closing
/// quote, or `None` when the literal is malformed. The `.@"..."`
/// identifier form, which Zig lexes with the same rules as a string.
fn decode_zig_string(text: &Text<'_>, i: usize) -> Option<(String, usize)> {
    scan_zig_string(text, i).ok()
}

/// Why a `"..."` body failed to decode: the ENGINE's error code for the
/// same fault, so a caller branching on `unterminated_string`,
/// `unprintable`, `invalid_unicode`, `invalid_ascii` or `unexpected`
/// sees what the engine's own string matcher would have said, and the
/// byte just past the offending span, so the message can quote the
/// literal from its opening quote up to the fault.
struct StringFault {
    code: &'static str,
    end: usize,
}

/// Scan a Zig double-quoted string body starting at byte `start`, just
/// past the opening quote: the decoded value and the byte just past the
/// closing quote. Shared by the `"..."` string matcher and the `.@"..."`
/// identifier form.
///
/// The escape set is Zig's and nothing wider: `\n`, `\r`, `\t`, `\\`,
/// `\'`, `\"`, `\xNN` and `\u{...}`. A `\u{...}` must name a Unicode
/// SCALAR value, so the surrogate block is refused along with anything
/// above U+10FFFF. Measured against the pinned zig 0.16.0 oracle, which
/// answers `"\u{D800}"` and `.@"\u{D800}"` alike with "unicode escape does
/// not correspond to a valid unicode scalar value"; a CHARACTER literal is
/// an integer in Zig and does accept a surrogate, so `char_matcher`
/// deliberately does not share this test. A run of `\xNN` escapes is a
/// run of BYTES, as it is in Zig, decoded as UTF-8 once the run ends:
/// `"\xe2\x82\xac"` is the euro sign, and an ill-formed sequence becomes
/// one U+FFFD per maximal subpart, as `String::from_utf8_lossy` decodes
/// it. A raw control character, a line end included, is not a string
/// character.
fn scan_zig_string(text: &Text<'_>, start: usize) -> Result<(String, usize), StringFault> {
    let mut out = String::new();
    // The bytes of consecutive `\xNN` escapes, decoded together.
    let mut pending: Vec<u8> = Vec::new();
    let flush = |out: &mut String, pending: &mut Vec<u8>| {
        if !pending.is_empty() {
            out.push_str(&String::from_utf8_lossy(pending));
            pending.clear();
        }
    };
    let fault = |code: &'static str, end: usize| StringFault {
        code,
        end: text.cap(end),
    };
    let mut i = start;
    while let Some(c) = text.byte(i) {
        match c {
            b'"' => {
                flush(&mut out, &mut pending);
                return Ok((out, i + 1));
            }
            b'\n' | b'\r' => return Err(fault("unterminated_string", i)),
            c if c < 0x20 || c == 0x7f => return Err(fault("unprintable", i + 1)),
            b'\\' => {
                let Some(escape) = text.byte(i + 1) else {
                    return Err(fault("unterminated_string", i + 1));
                };
                match escape {
                    b'x' => match text.get(i + 2, i + 4).filter(|hex| is_hex(hex)) {
                        Some(hex) => {
                            pending.push(u8::from_str_radix(hex, 16).expect("two hex digits"));
                            i += 4;
                        }
                        None => return Err(fault("invalid_ascii", i + 4)),
                    },
                    b'u' => {
                        if text.byte(i + 2) != Some(b'{') {
                            return Err(fault("invalid_unicode", i + 3));
                        }
                        let mut close = i + 3;
                        while text.byte(close).is_some_and(|c| c.is_ascii_hexdigit()) {
                            close += 1;
                        }
                        if close == i + 3 || text.byte(close) != Some(b'}') {
                            return Err(fault("invalid_unicode", close + 1));
                        }
                        // A literal too long for a u32 is above U+10FFFF too.
                        let scalar = u32::from_str_radix(text.slice(i + 3, close), 16)
                            .ok()
                            .filter(|&cp| is_scalar_value(cp));
                        let Some(code_point) = scalar else {
                            return Err(fault("invalid_unicode", close + 1));
                        };
                        flush(&mut out, &mut pending);
                        out.push(char_of(code_point));
                        i = close + 1;
                    }
                    b'n' | b'r' | b't' | b'\\' | b'\'' | b'"' => {
                        flush(&mut out, &mut pending);
                        out.push(match escape {
                            b'n' => '\n',
                            b'r' => '\r',
                            b't' => '\t',
                            other => other as char,
                        });
                        i += 2;
                    }
                    _ => return Err(fault("unexpected", i + 2)),
                }
            }
            _ => {
                flush(&mut out, &mut pending);
                let c = text.char_at(i).expect("a character boundary");
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    Err(fault("unterminated_string", i))
}

/// Whether `code_point` is a Unicode SCALAR value: at most U+10FFFF and
/// outside the surrogate block U+D800..=U+DFFF. This is what zig's
/// `std.zig.string_literal` requires of a `\u{...}` escape inside a
/// string or an `@"..."` identifier, and the whole of what
/// `char::from_u32` accepts.
fn is_scalar_value(code_point: u32) -> bool {
    char::from_u32(code_point).is_some()
}

/// The character a code point names. A lone surrogate has no character,
/// and folds to U+FFFD as it does throughout the engine. Reachable only
/// from the character-literal path: [`scan_zig_string`] refuses a
/// surrogate escape outright, and decodes `\xNN` bytes as UTF-8.
fn char_of(code_point: u32) -> char {
    char::from_u32(code_point).unwrap_or('\u{FFFD}')
}

// ---------------------------------------------------------------------------
// zonString
// ---------------------------------------------------------------------------

/// A Zig double-quoted string, `"..."`, with Zig's escape set and no
/// other. The engine's own string matcher is switched off
/// (`string.lex: false`): its relaxed-JSON escapes (`\b`, `\f`, `\v`,
/// `\/`, `\uXXXX`) and its acceptance of a surrogate `\u{...}` are not
/// ZON, and the pinned zig oracle rejects every one of them. The token is
/// `#ST`, as the engine's would be, and a fault carries the engine's code
/// for it, quoting the literal from its opening quote to the fault.
fn string_matcher(
    lexer: &mut Lexer<'_>,
    _rule: &mut Rule,
    _context: &mut Context,
) -> Option<Token> {
    windowed(lexer, cut_string)
}

fn cut_string(text: &Text<'_>) -> Cut {
    if text.byte(0) != Some(b'"') {
        return Cut::None;
    }
    match scan_zig_string(text, 1) {
        Ok((value, end)) => Cut::Emit {
            name: "#ST",
            tin: TIN_ST,
            value: Value::String(value),
            len: end,
            enum_mark: false,
        },
        Err(fault) => Cut::Bad {
            code: fault.code,
            start: 0,
            end: fault.end,
        },
    }
}

// ---------------------------------------------------------------------------
// zonMultiString
// ---------------------------------------------------------------------------

/// Multi-line Zig strings: consecutive lines starting with `\\`. Each
/// line contributes its content verbatim after the `\\`, and the lines
/// join with `\n`. Zig's tokenizer treats the whole run as ONE token and
/// skips the whitespace between the lines, so a blank line in the middle
/// continues the literal (and contributes nothing) rather than ending it.
fn multi_string_matcher(
    lexer: &mut Lexer<'_>,
    _rule: &mut Rule,
    context: &mut Context,
) -> Option<Token> {
    let line_chars = context.options.line.chars.as_str();
    windowed(lexer, |text| cut_multi_string(text, line_chars))
}

fn cut_multi_string(text: &Text<'_>, line_chars: &str) -> Cut {
    if !text.holds(0, b"\\\\") {
        return Cut::None;
    }

    let mut s = 0;
    let mut parts: Vec<&str> = Vec::new();
    while text.holds(s, b"\\\\") {
        s += 2;
        let line_start = s;
        while text.byte(s).is_some_and(|c| !is_line(line_chars, c)) {
            s += 1;
        }
        parts.push(text.slice(line_start, s));

        // Consume the line terminator, `\r\n` as one.
        if let Some(c) = text.byte(s).filter(|&c| is_line(line_chars, c)) {
            s += 1;
            if c == b'\r' && text.byte(s) == Some(b'\n') {
                s += 1;
            }
        }

        // Look for another `\\` continuation past blank lines.
        let mut peek = s;
        while text
            .byte(peek)
            .is_some_and(|c| c == b' ' || c == b'\t' || is_line(line_chars, c))
        {
            peek += 1;
        }
        if !text.holds(peek, b"\\\\") {
            break;
        }
        s = peek;
    }

    Cut::Emit {
        name: "#ST",
        tin: TIN_ST,
        value: Value::String(parts.join("\n")),
        len: s,
        enum_mark: false,
    }
}

// ---------------------------------------------------------------------------
// zonChar
// ---------------------------------------------------------------------------

/// A Zig character literal: `'x'`, `'\n'`, `'\x41'`, `'\u{1F600}'`. The
/// value is the code point under `charAsNumber`, else a one-character
/// string. Either way the token is `#NR`, as in the canonical runtime.
/// Unlike a string, a character literal is an INTEGER in Zig: `'\xD8'` is
/// 216 and `'\u{D800}'` is 55296, so neither goes through
/// [`scan_zig_string`].
fn char_matcher(lexer: &mut Lexer<'_>, char_as_number: bool) -> Option<Token> {
    windowed(lexer, |text| cut_char(text, char_as_number))
}

fn cut_char(text: &Text<'_>, char_as_number: bool) -> Cut {
    if text.byte(0) != Some(b'\'') {
        return Cut::None;
    }

    let mut i = 1;
    let code_point: u32;
    match text.byte(i) {
        Some(b'\\') => {
            i += 1;
            let Some(escape) = text.byte(i) else {
                return Cut::None;
            };
            match escape {
                b'n' => {
                    code_point = 10;
                    i += 1;
                }
                b'r' => {
                    code_point = 13;
                    i += 1;
                }
                b't' => {
                    code_point = 9;
                    i += 1;
                }
                b'\\' => {
                    code_point = 92;
                    i += 1;
                }
                b'\'' => {
                    code_point = 39;
                    i += 1;
                }
                b'"' => {
                    code_point = 34;
                    i += 1;
                }
                // `\0` is NOT a Zig escape (`'\x00'` and `'\u{0}'` are):
                // the pinned zig 0.16.0 oracle answers `'\0'` with
                // "invalid escape character: '0'".
                b'x' => {
                    i += 1;
                    let Some(hex) = text.get(i, i + 2).filter(|hex| is_hex(hex)) else {
                        return Cut::None;
                    };
                    code_point = u32::from_str_radix(hex, 16).expect("two hex digits");
                    i += 2;
                }
                b'u' => {
                    i += 1;
                    if text.byte(i) != Some(b'{') {
                        return Cut::None;
                    }
                    i += 1;
                    // The hex digits and the `}` they end on. A run that
                    // ends on anything else is no character literal: the
                    // first `}` after `{`, wherever it is, closes a run that
                    // is not all hex digits.
                    let mut close = i;
                    while text.byte(close).is_some_and(|c| c.is_ascii_hexdigit()) {
                        close += 1;
                    }
                    if text.byte(close) != Some(b'}') {
                        return Cut::None;
                    }
                    let hex = text.slice(i, close);
                    if !is_hex(hex) {
                        return Cut::None;
                    }
                    // Zig: a character literal is an INTEGER, so the
                    // escape names a code point rather than a scalar
                    // value: U+10FFFF is the only bound, and a lone
                    // surrogate is accepted. Measured: the pinned zig
                    // 0.16.0 oracle answers `'\u{D800}'` with 55296 and
                    // `'\u{110000}'` with "unicode escape does not
                    // correspond to a valid unicode scalar value". This is
                    // why the test here is NOT `is_scalar_value`, which
                    // `decode_zig_string` uses for a STRING escape.
                    // A number too big for u32 is above U+10FFFF too.
                    match u32::from_str_radix(hex, 16) {
                        Ok(cp) if cp <= 0x10ffff => code_point = cp,
                        _ => {
                            return Cut::Bad {
                                code: "zon_char",
                                start: 0,
                                end: text.cap(close + 2),
                            }
                        }
                    }
                    i = close + 1;
                }
                _ => return Cut::None,
            }
        }
        Some(b'\'') | None => return Cut::None,
        Some(_) => {
            let Some(c) = text.char_at(i) else {
                return Cut::None;
            };
            code_point = c as u32;
            // A raw control character (notably a literal newline) is not
            // a character literal in Zig: it is an invalid token.
            if code_point < 0x20 || code_point == 0x7f {
                return Cut::Bad {
                    code: "zon_char",
                    start: 0,
                    end: i + 1,
                };
            }
            i += c.len_utf8();
        }
    }

    if text.byte(i) != Some(b'\'') {
        return Cut::None;
    }
    i += 1;

    let value = if char_as_number {
        Value::Number(f64::from(code_point))
    } else {
        Value::String(char_of(code_point).to_string())
    };
    Cut::Emit {
        name: "#NR",
        tin: TIN_NR,
        value,
        len: i,
        enum_mark: false,
    }
}

// ---------------------------------------------------------------------------
// zonDocComment
// ---------------------------------------------------------------------------

/// `//!` and `///` are Zig DOC comments, which ZON rejects outright.
/// `////` and longer runs are plain line comments. This matcher only ever
/// fails the lex: an ordinary `//` comment falls through to the engine's
/// comment matcher.
fn doc_comment_matcher(
    lexer: &mut Lexer<'_>,
    _rule: &mut Rule,
    _context: &mut Context,
) -> Option<Token> {
    windowed(lexer, cut_doc_comment)
}

fn cut_doc_comment(text: &Text<'_>) -> Cut {
    if !text.holds(0, b"//") {
        return Cut::None;
    }
    let Some(c) = text.byte(2) else {
        return Cut::None;
    };
    if c == b'!' || (c == b'/' && text.byte(3) != Some(b'/')) {
        return Cut::Bad {
            code: "zon_doc_comment",
            start: 0,
            end: 3,
        };
    }
    Cut::None
}

// ---------------------------------------------------------------------------
// zonNumber
// ---------------------------------------------------------------------------

/// Zig numeric literals, as ZON defines them, replacing the relaxed
/// number lexer (`number.lex: false`) because that one happily accepts
/// `+1`, `.5`, `5.`, `0123`, `00`, `1__0` and `0x_2A`, none of which are
/// ZON. Owns the leading `-` and the `inf` / `nan` keywords too: Zig
/// allows `-inf` but not `-nan`, and rejects the integer `-0`.
fn number_matcher(
    lexer: &mut Lexer<'_>,
    _rule: &mut Rule,
    _context: &mut Context,
) -> Option<Token> {
    windowed(lexer, cut_number)
}

fn cut_number(text: &Text<'_>) -> Cut {
    let number = |value: Value, len: usize| Cut::Emit {
        name: "#NR",
        tin: TIN_NR,
        value,
        len,
        enum_mark: false,
    };
    let fault = |end: usize| Cut::Bad {
        code: "zon_number",
        start: 0,
        end,
    };

    let mut i = 0;
    let mut neg = false;
    if text.byte(0) == Some(b'-') {
        neg = true;
        i = 1;
        while matches!(text.byte(i), Some(b' ' | b'\t')) {
            i += 1;
        }
    }
    let Some(c) = text.byte(i) else {
        return Cut::None;
    };

    if c == b'i' || c == b'n' {
        if let Some(keyword) = text.get(i, i + 3) {
            if (keyword == "inf" || keyword == "nan") && !text.byte(i + 3).is_some_and(is_id_cont) {
                if keyword == "nan" && neg {
                    return fault(i + 3);
                }
                let value = match (keyword, neg) {
                    ("inf", false) => f64::INFINITY,
                    ("inf", true) => f64::NEG_INFINITY,
                    _ => f64::NAN,
                };
                return number(Value::Number(value), i + 3);
            }
        }
        return Cut::None;
    }

    if !c.is_ascii_digit() {
        return Cut::None;
    }

    match number::scan(text, i) {
        Err(end) => fault(end),
        Ok(Scanned::Int { end, big }) => {
            // Zig: `-0` is an ambiguous integer literal (`-0.0` is fine).
            if neg && big.is_zero() {
                return fault(end);
            }
            let value = match big.to_f64_exact() {
                Some(exact) => Value::Number(if neg { -exact } else { exact }),
                None => big_value(neg, &big),
            };
            number(value, end)
        }
        Ok(Scanned::Float { end, value }) => {
            number(Value::Number(if neg { -value } else { value }), end)
        }
    }
}

/// The value of an integer literal a double cannot hold exactly: the
/// `{ "$big": "<decimal>" }` object (see [`crate::BIG_KEY`]).
fn big_value(neg: bool, big: &BigUint) -> Value {
    let mut digits = big.to_decimal();
    if neg {
        digits.insert(0, '-');
    }
    let mut object = IndexMap::new();
    object.insert(crate::BIG_KEY.to_string(), Value::String(digits));
    Value::object(object)
}
