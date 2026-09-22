/* Copyright (c) 2025-2026 Richard Rodger, MIT License */

//! Zig numeric literals, as ZON defines them: the scanner behind the
//! `zonNumber` lex matcher, and the small arbitrary-precision integer
//! the exactness rule needs.
//!
//! The scanner is a line-for-line port of `scanZonNumber` in
//! `ts/src/zon.ts`: decimal / `0x` / `0o` / `0b` integers with `_`
//! separators between digits, decimal and hexadecimal floats (`1.5e3`,
//! `0x1.8p1`, `0x103.70`), a lowercase base prefix, no leading zero, no
//! `+`, and nothing alphanumeric left attached.
//!
//! The canonical runtime returns an integer whose exact value is not
//! representable as an IEEE-754 double as a `bigint`. The engine's
//! [`tabnas::Value`] has no big-integer variant, so [`BigUint`] holds the
//! digits long enough to decide exactness and, when the double would
//! lose precision, to render the decimal string the `$big` object carries
//! (see `big_value` in the lexer module).

/// One scanned unsigned literal.
pub(crate) enum Scanned {
    /// An integer literal, with its exact digits.
    Int { end: usize, big: BigUint },
    /// A float literal, already narrowed to a double.
    Float { end: usize, value: f64 },
}

/// Scan one unsigned Zig numeric literal starting at `start`, which must
/// be an ASCII digit. `Err(end)` spans the whole malformed literal, so the
/// error can quote it rather than its first character.
pub(crate) fn scan(src: &str, start: usize) -> Result<Scanned, usize> {
    let bytes = src.as_bytes();
    let mut i = start;
    let mut base: u32 = 10;
    let fail = || Err(token_end(src, start));

    if bytes[i] == b'0' {
        match bytes.get(i + 1).copied() {
            Some(b'x') => {
                base = 16;
                i += 2;
            }
            Some(b'o') => {
                base = 8;
                i += 2;
            }
            Some(b'b') => {
                base = 2;
                i += 2;
            }
            // The base prefix must be lowercase.
            Some(b'X' | b'O' | b'B') => return fail(),
            // A leading zero.
            Some(c) if c == b'_' || c.is_ascii_digit() => return fail(),
            _ => {}
        }
    }

    let (int_end, int_count, int_bad) = digit_run(bytes, i, base);
    if int_bad {
        return fail();
    }
    let int_text = src[i..int_end].replace('_', "");
    i = int_end;

    let mut is_float = false;
    let mut frac_text = String::new();
    if bytes.get(i) == Some(&b'.') {
        let after = bytes.get(i + 1).copied();
        let after_digit = after.map_or(-1, digit_val);
        // A `.` only starts a fraction when a digit of this base, or this
        // base's exponent letter, follows; otherwise it is a stray token and
        // the number ends here: `1.` and `0.1.2` are rejected by the parser.
        // The fraction itself may then be EMPTY: zig reads `1.e3` and
        // `0xF.p1` as one float token each.
        let starts_frac = (0 <= after_digit && (after_digit as u32) < base)
            || (base == 16 && matches!(after, Some(b'p' | b'P')))
            || (base == 10 && matches!(after, Some(b'e' | b'E')));
        if starts_frac {
            if base != 16 && base != 10 {
                return fail(); // no floats in this base
            }
            is_float = true;
            i += 1;
            let (frac_end, _, frac_bad) = digit_run(bytes, i, base);
            if frac_bad {
                return fail();
            }
            frac_text = src[i..frac_end].replace('_', "");
            i = frac_end;
        }
    }

    let mut exp_val: i64 = 0;
    let mut has_exp = false;
    let exp_chars: &[u8] = match base {
        16 => b"pP",
        10 => b"eE",
        _ => b"",
    };
    if bytes.get(i).is_some_and(|c| exp_chars.contains(c)) {
        has_exp = true;
        is_float = true;
        i += 1;
        let mut exp_sign: i64 = 1;
        match bytes.get(i) {
            Some(b'+') => i += 1,
            Some(b'-') => {
                exp_sign = -1;
                i += 1;
            }
            _ => {}
        }
        let (exp_end, exp_count, exp_bad) = digit_run(bytes, i, 10);
        if exp_bad || exp_count == 0 {
            return fail();
        }
        // An exponent no double can survive is saturated rather than
        // parsed: past a million either way the value is already
        // infinity or zero.
        let digits = src[i..exp_end].replace('_', "");
        exp_val = exp_sign * digits.parse::<i64>().unwrap_or(1_000_000).min(1_000_000);
        i = exp_end;
    }

    if int_count == 0 && frac_text.is_empty() {
        return fail();
    }

    // Anything alphanumeric still attached is a digit invalid for this
    // base.
    if bytes.get(i).is_some_and(|&c| is_id_cont(c)) {
        return fail();
    }

    let int_or_zero = if int_text.is_empty() { "0" } else { &int_text };

    if is_float {
        let value = if base == 16 {
            // The mantissa, correctly rounded to a double, scaled by the
            // power of two the exponent and the radix point call for: the
            // arithmetic `Number(BigInt('0x' + digits)) * 2 ** e` of the
            // canonical runtime.
            let mantissa = BigUint::from_digits(&format!("{int_or_zero}{frac_text}"), 16);
            let scale = exp_val - 4 * frac_text.len() as i64;
            mantissa.to_f64() * pow2(scale)
        } else {
            let mut text = int_or_zero.to_string();
            if !frac_text.is_empty() {
                text.push('.');
                text.push_str(&frac_text);
            }
            if has_exp {
                text.push_str(&format!("e{exp_val}"));
            }
            // The text is digits, an optional fraction and an optional
            // signed exponent, which always parses; an overflow is
            // infinity, as it is in Zig.
            text.parse::<f64>().unwrap_or(f64::NAN)
        };
        return Ok(Scanned::Float { end: i, value });
    }

    Ok(Scanned::Int {
        end: i,
        big: BigUint::from_digits(int_or_zero, base),
    })
}

/// Scan a run of `base` digits with Zig's digit-separator rules: a `_`
/// must sit directly between two digits (no leading, trailing, or
/// repeated `_`). Returns the end index, the digit count, and whether the
/// run was malformed.
fn digit_run(bytes: &[u8], start: usize, base: u32) -> (usize, usize, bool) {
    let mut i = start;
    let mut count = 0;
    let mut prev_digit = false;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'_' {
            if !prev_digit {
                return (i, count, true);
            }
            prev_digit = false;
            i += 1;
            continue;
        }
        let dv = digit_val(c);
        if 0 <= dv && (dv as u32) < base {
            count += 1;
            prev_digit = true;
            i += 1;
            continue;
        }
        break;
    }
    if start < i && !prev_digit {
        return (i, count, true);
    }
    (i, count, false)
}

/// The greedy extent of a malformed numeric token, so the error span
/// covers the whole literal rather than its first character.
fn token_end(src: &str, start: usize) -> usize {
    let bytes = src.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        let attached = is_id_cont(bytes[i])
            || (bytes[i] == b'.' && bytes.get(i + 1).is_some_and(|&c| is_id_cont(c)));
        if !attached {
            break;
        }
        i += 1;
    }
    i
}

pub(crate) fn is_id_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

pub(crate) fn is_id_cont(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The value of an ASCII digit in bases up to 16, or -1.
pub(crate) fn digit_val(c: u8) -> i32 {
    match c {
        b'0'..=b'9' => i32::from(c - b'0'),
        b'a'..=b'f' => i32::from(c - b'a') + 10,
        b'A'..=b'F' => i32::from(c - b'A') + 10,
        _ => -1,
    }
}

/// Whether `text` is one or more hex digits.
pub(crate) fn is_hex(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|c| c.is_ascii_hexdigit())
}

/// Exactly `2^k` as a double, saturating: infinity above the largest
/// exponent, the subnormals below the normal range, zero past them. The
/// `Math.pow(2, k)` of the canonical runtime, without a rounding step.
pub(crate) fn pow2(k: i64) -> f64 {
    if k > 1023 {
        f64::INFINITY
    } else if k >= -1022 {
        f64::from_bits(((k + 1023) as u64) << 52)
    } else if k >= -1074 {
        f64::from_bits(1u64 << (k + 1074))
    } else {
        0.0
    }
}

/// An unsigned integer of any size. A decimal literal keeps its digits,
/// so reading it, rounding it to a double and spelling it back are all
/// linear in its length; a literal in a power-of-two base is packed into
/// base 2^32 limbs, least significant first, which is linear too. Only
/// spelling a limb value in decimal, the `$big` form of a long hex, octal
/// or binary literal that no double holds, walks the limbs once per nine
/// digits. It does the three things the number matcher needs and nothing
/// else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BigUint {
    repr: Repr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Repr {
    /// Decimal digits with no leading zero, except for zero itself.
    Decimal(String),
    /// Base 2^32 limbs, least significant first, with no zero limb on
    /// top (zero is no limbs at all).
    Limbs(Vec<u32>),
}

impl BigUint {
    /// The integer the digit string denotes in `base`. Every character
    /// must be a digit of that base; the scanner guarantees it.
    pub(crate) fn from_digits(text: &str, base: u32) -> Self {
        debug_assert!(
            text.bytes().all(|c| {
                let digit = digit_val(c);
                0 <= digit && (digit as u32) < base
            }),
            "not all base-{base} digits: {text}"
        );
        let repr = if base == 10 {
            let digits = text.trim_start_matches('0');
            Repr::Decimal(if digits.is_empty() {
                "0".to_string()
            } else {
                digits.to_string()
            })
        } else {
            Repr::Limbs(pack_limbs(text, base))
        };
        BigUint { repr }
    }

    pub(crate) fn is_zero(&self) -> bool {
        match &self.repr {
            Repr::Decimal(digits) => digits == "0",
            Repr::Limbs(limbs) => limbs.is_empty(),
        }
    }

    /// The nearest double, rounding half to even: `Number(bigint)`.
    pub(crate) fn to_f64(&self) -> f64 {
        match &self.repr {
            // The standard library's decimal-to-double conversion is
            // correctly rounded for a digit string of any length, and
            // linear in it; past the double range it is infinity, which
            // is what `Number` of such a bigint gives too.
            Repr::Decimal(digits) => digits.parse::<f64>().unwrap_or(f64::INFINITY),
            Repr::Limbs(limbs) => limbs_to_f64(limbs),
        }
    }

    /// The double, when it is exact: `Number.isFinite(n) && BigInt(n) ===
    /// big` in the canonical runtime. A value needs at most 53
    /// significant bits and has to fit the exponent range.
    pub(crate) fn to_f64_exact(&self) -> Option<f64> {
        match &self.repr {
            Repr::Decimal(digits) => {
                let value = self.to_f64();
                // Every finite double this large is an integer, and `{:.0}`
                // spells its exact value, so the round trip decides
                // exactness without any big arithmetic.
                (value.is_finite() && format!("{value:.0}") == *digits).then_some(value)
            }
            Repr::Limbs(limbs) => {
                let length = bit_length(limbs);
                if length > 1024 || length - trailing_zeros(limbs) > 53 {
                    return None;
                }
                Some(limbs_to_f64(limbs))
            }
        }
    }

    /// The decimal digits.
    pub(crate) fn to_decimal(&self) -> String {
        match &self.repr {
            Repr::Decimal(digits) => digits.clone(),
            Repr::Limbs(limbs) => limbs_to_decimal(limbs),
        }
    }
}

/// The limbs of a digit string in a power-of-two base, packed bit by
/// bit from the least significant digit: linear in the digit count.
fn pack_limbs(text: &str, base: u32) -> Vec<u32> {
    let bits = base.trailing_zeros();
    let mut limbs = Vec::with_capacity(text.len() * bits as usize / 32 + 1);
    let mut acc: u64 = 0;
    let mut filled: u32 = 0;
    for c in text.bytes().rev() {
        acc |= u64::from(digit_val(c).max(0) as u32) << filled;
        filled += bits;
        if filled >= 32 {
            limbs.push(acc as u32);
            acc >>= 32;
            filled -= 32;
        }
    }
    if filled > 0 {
        limbs.push(acc as u32);
    }
    while limbs.last() == Some(&0) {
        limbs.pop();
    }
    limbs
}

/// The number of significant bits: zero for zero.
fn bit_length(limbs: &[u32]) -> u64 {
    match limbs.last() {
        None => 0,
        Some(top) => (limbs.len() as u64 - 1) * 32 + u64::from(32 - top.leading_zeros()),
    }
}

fn bit(limbs: &[u32], index: u64) -> bool {
    limbs
        .get((index / 32) as usize)
        .is_some_and(|limb| (limb >> (index % 32)) & 1 == 1)
}

fn trailing_zeros(limbs: &[u32]) -> u64 {
    for (index, limb) in limbs.iter().enumerate() {
        if *limb != 0 {
            return index as u64 * 32 + u64::from(limb.trailing_zeros());
        }
    }
    0
}

/// Bits `from..from + count` as an integer, `count` at most 64.
fn bits_from(limbs: &[u32], from: u64, count: u32) -> u64 {
    (0..count)
        .filter(|k| bit(limbs, from + u64::from(*k)))
        .fold(0u64, |acc, k| acc | (1u64 << k))
}

fn any_bit_below(limbs: &[u32], index: u64) -> bool {
    (0..index).any(|k| bit(limbs, k))
}

/// The nearest double to a limb value, rounding half to even.
fn limbs_to_f64(limbs: &[u32]) -> f64 {
    let length = bit_length(limbs);
    if length <= 53 {
        return bits_from(limbs, 0, length as u32) as f64;
    }
    let shift = length - 53;
    let mut mantissa = bits_from(limbs, shift, 53);
    let half = bit(limbs, shift - 1);
    let sticky = any_bit_below(limbs, shift - 1);
    if half && (sticky || mantissa & 1 == 1) {
        mantissa += 1;
    }
    (mantissa as f64) * pow2(shift as i64)
}

/// The decimal digits of a limb value, nine at a time.
fn limbs_to_decimal(limbs: &[u32]) -> String {
    if limbs.is_empty() {
        return "0".to_string();
    }
    const CHUNK: u64 = 1_000_000_000;
    let mut limbs = limbs.to_vec();
    let mut chunks: Vec<u32> = Vec::new();
    while !limbs.is_empty() {
        let mut remainder = 0u64;
        for limb in limbs.iter_mut().rev() {
            let v = (remainder << 32) | u64::from(*limb);
            *limb = (v / CHUNK) as u32;
            remainder = v % CHUNK;
        }
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        chunks.push(remainder as u32);
    }
    let mut out = chunks.last().map_or_else(String::new, u32::to_string);
    for chunk in chunks.iter().rev().skip(1) {
        out.push_str(&format!("{chunk:09}"));
    }
    out
}

/// JavaScript's `Number::toString` (ECMA-262 6.1.6.1.20), which is what
/// the canonical plugin's computed property key `{ [enumTag]: name }`
/// spells when the option it is given is a number.
///
/// Rust's own `f64` formatting differs from it in three ways a key
/// reaches. It keeps the sign of a negative zero, where JavaScript says
/// `0`. It never switches to exponent form, where JavaScript does so at
/// `1e21` and at `1e-7`, and `serde_json` switches at neither the same
/// place nor the same spelling (`1e16` for ten quadrillion, where
/// JavaScript writes the digits out). And where two equally short digit
/// strings are equally close to the value, the specification takes the
/// one ending in an even digit and the shortest form does not.
///
/// Ported from `js_number_to_string` in `/home/user/csv/rs/src/lib.rs`,
/// which `js_number` in the bnf port and `jsNumberToString` in the Go
/// csv port also spell. Fuzzed against node over 113296 distinct
/// doubles, the halves, tenths, hundredths and thousandths included.
pub(crate) fn js_number_to_string(number: f64) -> String {
    if number.is_nan() {
        return "NaN".to_string();
    }
    // Catches -0.0 as well: JavaScript spells both zeros "0".
    if number == 0.0 {
        return "0".to_string();
    }
    if number < 0.0 {
        return format!("-{}", js_number_to_string(-number));
    }
    if number.is_infinite() {
        return "Infinity".to_string();
    }

    // The specification wants the shortest digit string `s` that round-trips
    // (length `k`), and `n`, the position of the decimal point relative to
    // it. Rust's `{:e}` yields digits of exactly that shortest length.
    let shortest = format!("{number:e}");
    let shortest_k = shortest
        .split_once('e')
        .map(|(mantissa, _)| mantissa.chars().filter(char::is_ascii_digit).count())
        .expect("a finite f64 always formats with an exponent");

    // Re-render to that same length to settle a tie. Where two digit
    // strings of length `k` are equally close to `number`, the
    // specification takes the one ending in an even digit; Rust's shortest
    // form does not, but its exactly-rounded fixed-precision form does.
    let exponential = format!("{:.*e}", shortest_k - 1, number);
    let (mantissa, exponent) = exponential
        .split_once('e')
        .expect("a finite f64 always formats with an exponent");
    // Rounding can leave trailing zeros (and, on a carry, one digit too
    // many); dropping them keeps `s` shortest, which is what `k` means.
    let digits = mantissa
        .chars()
        .filter(|digit| *digit != '.')
        .collect::<String>();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let k = digits.len() as i32;
    let n = exponent
        .parse::<i32>()
        .expect("a formatted exponent is an integer")
        + 1;

    // The four cases of the specification, in its order. The range bounds
    // are `k <= n <= 21`, `0 < n <= 21` and `-6 < n <= 0`.
    if (k..=21).contains(&n) {
        // Integral, with n - k trailing zeros to restore.
        let mut text = digits.to_string();
        text.push_str(&"0".repeat((n - k) as usize));
        text
    } else if (1..=21).contains(&n) {
        let point = n as usize;
        format!("{}.{}", &digits[..point], &digits[point..])
    } else if (-5..=0).contains(&n) {
        format!("0.{}{}", "0".repeat(-n as usize), digits)
    } else {
        // Exponent form. `n - 1` is never 0 here, so the sign is never "+0".
        let sign = if n - 1 < 0 { '-' } else { '+' };
        let power = (n - 1).abs();
        if k == 1 {
            format!("{digits}e{sign}{power}")
        } else {
            format!("{}.{}e{sign}{power}", &digits[..1], &digits[1..])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(text: &str, base: u32) -> BigUint {
        BigUint::from_digits(text, base)
    }

    #[test]
    fn long_literals_stay_cheap_in_every_base() {
        // Half a million decimal digits: kept as digits, so reading,
        // rounding and spelling are linear and take milliseconds.
        let text = format!("1{}", "0".repeat(500_000));
        let value = big(&text, 10);
        assert_eq!(value.to_f64_exact(), None);
        assert_eq!(value.to_f64(), f64::INFINITY);
        assert_eq!(value.to_decimal(), text);
        // Packed bits for the power-of-two bases, spelled once.
        let hex = big(&"f".repeat(4_000), 16);
        assert_eq!(hex.to_f64_exact(), None);
        assert_eq!(hex.to_decimal().len(), 4_817);
        assert_eq!(big(&"7".repeat(3), 8).to_decimal(), "511");
        assert_eq!(
            big("10000000000000000000000000000000000", 2).to_decimal(),
            "17179869184"
        );
    }

    #[test]
    fn decimal_round_trips_through_every_base() {
        let want = "36893488147419103231"; // 2^65 - 1
        assert_eq!(big(want, 10).to_decimal(), want);
        assert_eq!(big("1ffffffffffffffff", 16).to_decimal(), want);
        assert_eq!(big("3777777777777777777777", 8).to_decimal(), want);
        assert_eq!(big(&"1".repeat(65), 2).to_decimal(), want);
        assert_eq!(big("0", 10).to_decimal(), "0");
        assert_eq!(big("000", 16).to_decimal(), "0");
    }

    #[test]
    fn exactness_follows_the_53_bit_rule() {
        assert_eq!(
            big("9007199254740992", 10).to_f64_exact(),
            Some(9007199254740992.0)
        );
        assert_eq!(big("9007199254740993", 10).to_f64_exact(), None);
        assert_eq!(
            big("18446744073709551616", 10).to_f64_exact(),
            Some(18446744073709551616.0)
        );
        assert_eq!(big("36893488147419103231", 10).to_f64_exact(), None);
        assert_eq!(big("0", 10).to_f64_exact(), Some(0.0));
        assert_eq!(big(&"1".repeat(1030), 2).to_f64_exact(), None);
    }

    #[test]
    fn rounding_is_half_to_even() {
        // 2^53 + 1 sits exactly between two doubles and rounds to the
        // even one, 2^53; 2^53 + 3 rounds up to 2^53 + 4.
        assert_eq!(big("9007199254740993", 10).to_f64(), 9007199254740992.0);
        assert_eq!(big("9007199254740995", 10).to_f64(), 9007199254740996.0);
        assert_eq!(
            big("36893488147419103231", 10).to_f64(),
            36893488147419103232.0
        );
    }

    #[test]
    fn a_number_spells_itself_as_javascript_does() {
        // Every expectation below is `String(value)` in node, measured.
        for (value, want) in [
            (0.0, "0"),
            (-0.0, "0"),
            (123.0, "123"),
            (1.5, "1.5"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            // The shortest round-tripping form breaks this midpoint away
            // from zero; the specification takes the even digit.
            (5e-324, "5e-324"),
            (1e15, "1000000000000000"),
            (9e15, "9000000000000000"),
            // Past 2^53 a whole double is still written out in full.
            (9007199254740992.0, "9007199254740992"),
            (1e16, "10000000000000000"),
            (1234567890123456800.0, "1234567890123456800"),
            // The exponent-form boundaries: 1e21 and 1e-7.
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (0.000001, "0.000001"),
            (1e-7, "1e-7"),
            (1e-10, "1e-10"),
            (1.2345678901234568e29, "1.2345678901234568e+29"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
        ] {
            assert_eq!(js_number_to_string(value), want, "{value:?}");
        }
        assert_eq!(js_number_to_string(f64::NAN), "NaN");
    }

    #[test]
    fn powers_of_two_saturate() {
        assert_eq!(pow2(0), 1.0);
        assert_eq!(pow2(-1), 0.5);
        assert_eq!(pow2(1023), f64::MAX / (2.0 - f64::EPSILON));
        assert_eq!(pow2(1024), f64::INFINITY);
        assert_eq!(pow2(-1074), 5e-324);
        assert_eq!(pow2(-1075), 0.0);
    }
}
