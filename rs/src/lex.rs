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
//! Every matcher works on [`Lexer::remaining`] by byte index. The only
//! non-ASCII bytes any of them cares about are whole characters (a raw
//! character literal, the body of a string or a `.@"..."` identifier),
//! and those are decoded as characters; everything else is compared
//! against ASCII, so a byte index is always a character boundary where
//! it is used to slice. Cursor advancement goes through [`Lexer::advance_chars`], which
//! keeps the row and column honest across the newlines a token may span,
//! the bookkeeping the TypeScript `advance` helpers do by hand.

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
fn skip_insig(src: &str, mut i: usize, line_chars: &str) -> usize {
    let bytes = src.as_bytes();
    while i < bytes.len() {
        let c = bytes[i];
        if is_line(line_chars, c) || c == b' ' || c == b'\t' {
            i += 1;
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && !is_line(line_chars, bytes[i]) {
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
    let line_chars = &context.options.line.chars;
    let remaining = lexer.remaining();
    let bytes = remaining.as_bytes();
    if bytes.first() != Some(&b'.') {
        return None;
    }
    let d = skip_insig(remaining, 1, line_chars);
    let at_d = bytes.get(d).copied();
    let at_d1 = bytes.get(d + 1).copied();

    // `.{` opens a struct literal. Decide map vs list by peeking ahead.
    if at_d == Some(b'{') {
        let (name, tin) = if peek_is_map_open(remaining, d + 1, line_chars) {
            ("#OB", TIN_OB)
        } else {
            ("#OS", TIN_OS)
        };
        return Some(emit(lexer, name, tin, Value::Undefined, d + 1, false));
    }

    // `.@"..."`: any string may name a field or an enum literal. Zig
    // rejects an empty one and one containing a NUL.
    if at_d == Some(b'@') && at_d1 == Some(b'"') {
        return Some(match decode_zig_string(remaining, d + 2) {
            Some((value, end)) if !value.is_empty() && !value.contains('\0') => {
                emit(lexer, "#TX", TIN_TX, Value::String(value), end, true)
            }
            Some((_, end)) => bad(lexer, "zon_ident", 0, end),
            None => bad(lexer, "zon_ident", 0, d + 2),
        });
    }

    // `.identifier`: field name or enum literal.
    let first = at_d?;
    if !is_id_start(first) {
        return None;
    }
    let mut e = d;
    while e < bytes.len() && is_id_cont(bytes[e]) {
        e += 1;
    }
    let name = remaining[d..e].to_string();
    Some(emit(lexer, "#TX", TIN_TX, Value::String(name), e, true))
}

/// Whether the source inside `.{ ... }`, from byte `start`, begins with a
/// field name (`.ident` or `.@"..."`) followed by `=`: a struct literal
/// rather than a tuple.
fn peek_is_map_open(src: &str, start: usize, line_chars: &str) -> bool {
    let bytes = src.as_bytes();
    let mut i = skip_insig(src, start, line_chars);
    if bytes.get(i) != Some(&b'.') {
        return false;
    }
    i = skip_insig(src, i + 1, line_chars);
    if bytes.get(i) == Some(&b'@') && bytes.get(i + 1) == Some(&b'"') {
        match decode_zig_string(src, i + 2) {
            Some((_, end)) => i = end,
            None => return false,
        }
    } else {
        if !bytes.get(i).is_some_and(|&c| is_id_start(c)) {
            return false;
        }
        i += 1;
        while i < bytes.len() && is_id_cont(bytes[i]) {
            i += 1;
        }
    }
    i = skip_insig(src, i, line_chars);
    bytes.get(i) == Some(&b'=')
}

/// Decode a Zig double-quoted string body starting at byte `i`, just past
/// the opening quote: the decoded value and the byte just past the closing
/// quote, or `None` when the literal is malformed. The `.@"..."`
/// identifier form, which Zig lexes with the same rules as a string.
fn decode_zig_string(src: &str, i: usize) -> Option<(String, usize)> {
    scan_zig_string(src, i).ok()
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
fn scan_zig_string(src: &str, start: usize) -> Result<(String, usize), StringFault> {
    let bytes = src.as_bytes();
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
        end: end.min(bytes.len()),
    };
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                flush(&mut out, &mut pending);
                return Ok((out, i + 1));
            }
            b'\n' | b'\r' => return Err(fault("unterminated_string", i)),
            c if c < 0x20 || c == 0x7f => return Err(fault("unprintable", i + 1)),
            b'\\' => {
                let Some(&escape) = bytes.get(i + 1) else {
                    return Err(fault("unterminated_string", i + 1));
                };
                match escape {
                    b'x' => match src.get(i + 2..i + 4).filter(|hex| is_hex(hex)) {
                        Some(hex) => {
                            pending.push(u8::from_str_radix(hex, 16).expect("two hex digits"));
                            i += 4;
                        }
                        None => return Err(fault("invalid_ascii", i + 4)),
                    },
                    b'u' => {
                        if bytes.get(i + 2) != Some(&b'{') {
                            return Err(fault("invalid_unicode", i + 3));
                        }
                        let mut close = i + 3;
                        while close < bytes.len() && bytes[close].is_ascii_hexdigit() {
                            close += 1;
                        }
                        if close == i + 3 || bytes.get(close) != Some(&b'}') {
                            return Err(fault("invalid_unicode", close + 1));
                        }
                        // A literal too long for a u32 is above U+10FFFF too.
                        let scalar = u32::from_str_radix(&src[i + 3..close], 16)
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
                let c = src[i..].chars().next().expect("a character boundary");
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    Err(fault("unterminated_string", bytes.len()))
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
    let remaining = lexer.remaining();
    if remaining.as_bytes().first() != Some(&b'"') {
        return None;
    }
    Some(match scan_zig_string(remaining, 1) {
        Ok((value, end)) => emit(lexer, "#ST", TIN_ST, Value::String(value), end, false),
        Err(fault) => bad(lexer, fault.code, 0, fault.end),
    })
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
    let line_chars = &context.options.line.chars;
    let remaining = lexer.remaining();
    let bytes = remaining.as_bytes();
    if !bytes.starts_with(b"\\\\") {
        return None;
    }

    let mut s = 0;
    let mut parts: Vec<&str> = Vec::new();
    while bytes[s..].starts_with(b"\\\\") {
        s += 2;
        let line_start = s;
        while s < bytes.len() && !is_line(line_chars, bytes[s]) {
            s += 1;
        }
        parts.push(&remaining[line_start..s]);

        // Consume the line terminator, `\r\n` as one.
        if s < bytes.len() && is_line(line_chars, bytes[s]) {
            let c = bytes[s];
            s += 1;
            if c == b'\r' && bytes.get(s) == Some(&b'\n') {
                s += 1;
            }
        }

        // Look for another `\\` continuation past blank lines.
        let mut peek = s;
        while peek < bytes.len() {
            let c = bytes[peek];
            if c == b' ' || c == b'\t' || is_line(line_chars, c) {
                peek += 1;
            } else {
                break;
            }
        }
        if !bytes[peek..].starts_with(b"\\\\") {
            break;
        }
        s = peek;
    }

    let value = parts.join("\n");
    Some(emit(lexer, "#ST", TIN_ST, Value::String(value), s, false))
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
    let remaining = lexer.remaining();
    let bytes = remaining.as_bytes();
    if bytes.first() != Some(&b'\'') {
        return None;
    }

    let mut i = 1;
    let code_point: u32;
    match bytes.get(i).copied() {
        Some(b'\\') => {
            i += 1;
            match bytes.get(i).copied()? {
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
                    let hex = remaining.get(i..i + 2)?;
                    if !is_hex(hex) {
                        return None;
                    }
                    code_point = u32::from_str_radix(hex, 16).ok()?;
                    i += 2;
                }
                b'u' => {
                    i += 1;
                    if bytes.get(i) != Some(&b'{') {
                        return None;
                    }
                    i += 1;
                    let close = remaining.get(i..)?.find('}')? + i;
                    let hex = &remaining[i..close];
                    if !is_hex(hex) {
                        return None;
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
                        _ => return Some(bad(lexer, "zon_char", 0, close + 2)),
                    }
                    i = close + 1;
                }
                _ => return None,
            }
        }
        Some(b'\'') | None => return None,
        Some(_) => {
            let c = remaining[i..].chars().next()?;
            code_point = c as u32;
            // A raw control character (notably a literal newline) is not
            // a character literal in Zig: it is an invalid token.
            if code_point < 0x20 || code_point == 0x7f {
                return Some(bad(lexer, "zon_char", 0, i + 1));
            }
            i += c.len_utf8();
        }
    }

    if bytes.get(i) != Some(&b'\'') {
        return None;
    }
    i += 1;

    let value = if char_as_number {
        Value::Number(f64::from(code_point))
    } else {
        Value::String(char_of(code_point).to_string())
    };
    Some(emit(lexer, "#NR", TIN_NR, value, i, false))
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
    let bytes = lexer.remaining().as_bytes();
    if bytes.len() < 3 || !bytes.starts_with(b"//") {
        return None;
    }
    let c = bytes[2];
    if c == b'!' || (c == b'/' && bytes.get(3) != Some(&b'/')) {
        return Some(bad(lexer, "zon_doc_comment", 0, 3));
    }
    None
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
    let remaining = lexer.remaining();
    let bytes = remaining.as_bytes();

    let mut i = 0;
    let mut neg = false;
    if bytes.first() == Some(&b'-') {
        neg = true;
        i = 1;
        while matches!(bytes.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
    }
    let c = bytes.get(i).copied()?;

    if c == b'i' || c == b'n' {
        if let Some(keyword) = remaining.get(i..i + 3) {
            if (keyword == "inf" || keyword == "nan")
                && !bytes.get(i + 3).is_some_and(|&c| is_id_cont(c))
            {
                if keyword == "nan" && neg {
                    return Some(bad(lexer, "zon_number", 0, i + 3));
                }
                let value = match (keyword, neg) {
                    ("inf", false) => f64::INFINITY,
                    ("inf", true) => f64::NEG_INFINITY,
                    _ => f64::NAN,
                };
                return Some(emit(
                    lexer,
                    "#NR",
                    TIN_NR,
                    Value::Number(value),
                    i + 3,
                    false,
                ));
            }
        }
        return None;
    }

    if !c.is_ascii_digit() {
        return None;
    }

    match number::scan(remaining, i) {
        Err(end) => Some(bad(lexer, "zon_number", 0, end)),
        Ok(Scanned::Int { end, big }) => {
            // Zig: `-0` is an ambiguous integer literal (`-0.0` is fine).
            if neg && big.is_zero() {
                return Some(bad(lexer, "zon_number", 0, end));
            }
            let value = match big.to_f64_exact() {
                Some(exact) => Value::Number(if neg { -exact } else { exact }),
                None => big_value(neg, &big),
            };
            Some(emit(lexer, "#NR", TIN_NR, value, end, false))
        }
        Ok(Scanned::Float { end, value }) => {
            let value = if neg { -value } else { value };
            Some(emit(lexer, "#NR", TIN_NR, Value::Number(value), end, false))
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
