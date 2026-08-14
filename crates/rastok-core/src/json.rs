//! The house JSON reader (D-2026-08-10-7): one complete JSON value,
//! parsed EXACTLY — duplicate keys refuse, numbers are lossless
//! (integers at honest width, everything else an exact decimal
//! within the decimal128 budget, refusal beyond), and every fault
//! carries its byte offset so the compiler can point INTO the
//! source. Shared by the `json` literal today and a lazy decoder
//! tomorrow; `serde_json` verifies it differentially in dev tests.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::value::Value;

/// The significant-digit budget — Arrow `decimal128`.
pub const MAX_DIGITS: u32 = 38;
/// The scale bounds — the IEEE decimal128 exponent domain, exactly
/// as identity.md's registry pins it: scale ∈ [−6143, +6144].
pub const MAX_SCALE: i32 = 6144;
/// The lower scale bound (see [`MAX_SCALE`]).
pub const MIN_SCALE: i32 = -6143;

/// The decimal128 DOMAIN (D-2026-08-10-7) any decimal — declared
/// scale included — must inhabit: at most [`MAX_DIGITS`]
/// significant digits, scale within [`MIN_SCALE`]..=[`MAX_SCALE`].
/// The machine's doors hold values to this before they travel;
/// the writer keeps a defensive mirror.
#[must_use]
pub fn decimal_in_domain(unscaled: i128, scale: i32) -> bool {
    (MIN_SCALE..=MAX_SCALE).contains(&scale) && unscaled.unsigned_abs() < 10_u128.pow(MAX_DIGITS)
}

/// A declared exact number, as `decimal("…")` reads it
/// (D-2026-08-13-1): the WRITTEN scale is preserved — trailing
/// zeros significant, exponents reaching negative scales — and the
/// D-2026-08-10-7 integer fold applies to the written scale alone
/// (scale ≤ 0 whose value fits i64 is an int).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredNumber {
    /// The written scale was ≤ 0 and the value fits i64.
    Int(i64),
    /// A declared decimal at its WRITTEN scale.
    Decimal {
        /// The coefficient, sign included.
        unscaled: i128,
        /// The written scale (fraction digits minus exponent).
        scale: i32,
    },
}

/// Reads a standard JSON number AS WRITTEN (D-2026-08-13-1) — the
/// `decimal("…")` constructor's parser. One number grammar in the
/// system: JSON's exactly (optional `-`, `0 | [1-9]digits`,
/// optional fraction, optional exponent). The coefficient keeps
/// every written digit (at most [`MAX_DIGITS`] significant); the
/// scale is declared, bounded by [`MIN_SCALE`]..=[`MAX_SCALE`].
///
/// # Errors
///
/// A human-readable reason: not a JSON number, trailing content,
/// or outside the decimal128 domain.
pub fn parse_declared_number(text: &str) -> Result<DeclaredNumber, String> {
    let bytes = text.as_bytes();
    let mut pos = 0usize;
    let negative = bytes.first() == Some(&b'-');
    if negative {
        pos += 1;
    }
    let int_start = pos;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        pos += 1;
    }
    let int_digits = &text[int_start..pos];
    if int_digits.is_empty() {
        return Err(String::from("a JSON number starts with a digit"));
    }
    if int_digits.len() > 1 && int_digits.starts_with('0') {
        return Err(String::from("a JSON number has no leading zero"));
    }
    let mut frac_digits = "";
    if bytes.get(pos) == Some(&b'.') {
        pos += 1;
        let frac_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        frac_digits = &text[frac_start..pos];
        if frac_digits.is_empty() {
            return Err(String::from("a fraction needs at least one digit"));
        }
    }
    let mut exponent: i64 = 0;
    if matches!(bytes.get(pos), Some(b'e' | b'E')) {
        pos += 1;
        let exp_negative = match bytes.get(pos) {
            Some(b'-') => {
                pos += 1;
                true
            }
            Some(b'+') => {
                pos += 1;
                false
            }
            _ => false,
        };
        let exp_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        let digits = &text[exp_start..pos];
        if digits.is_empty() {
            return Err(String::from("an exponent needs at least one digit"));
        }
        exponent = digits
            .parse::<i64>()
            .map_err(|_| String::from("the exponent is out of range"))?;
        if exp_negative {
            exponent = -exponent;
        }
    }
    if pos != bytes.len() {
        return Err(alloc::format!(
            "trailing content after the number: `{}`",
            &text[pos..]
        ));
    }
    // The coefficient keeps every WRITTEN digit.
    let mut unscaled: i128 = 0;
    let mut significant = 0u32;
    for byte in int_digits.bytes().chain(frac_digits.bytes()) {
        let digit = i128::from(byte - b'0');
        if unscaled != 0 || digit != 0 {
            significant += 1;
        }
        if significant > MAX_DIGITS {
            return Err(alloc::format!("more than {MAX_DIGITS} significant digits"));
        }
        unscaled = unscaled
            .checked_mul(10)
            .and_then(|value| value.checked_add(digit))
            .ok_or_else(|| String::from("the coefficient overflows"))?;
    }
    if negative {
        unscaled = -unscaled;
    }
    let scale = i64::try_from(frac_digits.len())
        .ok()
        .and_then(|frac| frac.checked_sub(exponent))
        .and_then(|scale| i32::try_from(scale).ok())
        .ok_or_else(|| String::from("the scale is out of range"))?;
    if !(MIN_SCALE..=MAX_SCALE).contains(&scale) {
        return Err(alloc::format!(
            "the scale {scale} is outside [{MIN_SCALE}, {MAX_SCALE}]"
        ));
    }
    // The integer fold, on the WRITTEN scale (D-2026-08-10-7).
    if scale <= 0 {
        let folded = 10_i128
            .checked_pow(scale.unsigned_abs())
            .and_then(|factor| unscaled.checked_mul(factor))
            .and_then(|value| i64::try_from(value).ok());
        if let Some(value) = folded {
            return Ok(DeclaredNumber::Int(value));
        }
    }
    Ok(DeclaredNumber::Decimal { unscaled, scale })
}

/// The invariants of a DECLARED-scale decimal (D-2026-08-13-1,
/// review round), shared by the package decoder and the plan
/// validator (`encode ⇒ decode` demands one rule): the decimal128
/// budgets, and the written-scale integer fold — a value the fold
/// would have collapsed to an integer is not a decimal constant.
/// Trailing zeros are SIGNIFICANT here (`decimal("1.50")` is
/// {150, 2}); the minimal-scale rule below governs DOCUMENT-origin
/// values only, at their construction.
#[must_use]
pub fn declared_decimal_well_formed(unscaled: i128, scale: i32) -> bool {
    if !(MIN_SCALE..=MAX_SCALE).contains(&scale) {
        return false;
    }
    if unscaled.unsigned_abs() >= 10_u128.pow(MAX_DIGITS) {
        return false;
    }
    if scale <= 0 {
        let mut expanded = Some(unscaled);
        for _ in 0..scale.unsigned_abs() {
            expanded = expanded.and_then(|value| value.checked_mul(10));
        }
        if expanded.is_some_and(|value| i64::try_from(value).is_ok()) {
            // The written-scale fold would have made this an Int.
            return false;
        }
    }
    true
}

/// The D-2026-08-10-7 invariants of a DYNAMIC decimal, enforced at
/// the JSON reader's construction: budget bounds, minimal scale,
/// and the integer-domain disjointness.
#[must_use]
pub fn dynamic_decimal_well_formed(unscaled: i128, scale: i32) -> bool {
    if !(MIN_SCALE..=MAX_SCALE).contains(&scale) {
        return false;
    }
    if unscaled.unsigned_abs() >= 10_u128.pow(MAX_DIGITS) {
        return false;
    }
    if unscaled % 10 == 0 {
        // Trailing zeros strip at construction; zero is integer 0.
        return false;
    }
    if scale <= 0 {
        let mut expanded = Some(unscaled);
        for _ in 0..scale.unsigned_abs() {
            expanded = expanded.and_then(|value| value.checked_mul(10));
        }
        if expanded.is_some_and(|value| i64::try_from(value).is_ok()) {
            return false;
        }
    }
    true
}

/// A refusal, located by byte offset into the parsed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    /// Byte offset of the offending character or token.
    pub offset: usize,
    /// What went wrong — never stable text; callers key on offsets.
    pub message: String,
}

/// Parses ONE complete JSON value and returns it with the number of
/// bytes consumed (trailing input is the caller's business — the
/// DSL parser resumes right after).
///
/// # Errors
/// A located [`JsonError`] on any syntax fault, duplicate key, or
/// number outside the exact budget.
pub fn parse_value(text: &str) -> Result<(Value, usize), JsonError> {
    let bytes = text.as_bytes();
    let mut reader = Reader { bytes, pos: 0 };
    reader.skip_whitespace();
    let value = reader.value(0)?;
    Ok((value, reader.pos))
}

/// The nesting budget mirrors the format's reader budget.
const MAX_DEPTH: usize = 128;

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn err(&self, message: impl Into<String>) -> JsonError {
        JsonError {
            offset: self.pos,
            message: message.into(),
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.pos) {
            self.pos += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting beyond the reader budget"));
        }
        match self.bytes.get(self.pos) {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::Text(self.string()?)),
            Some(b't') => self.keyword("true", Value::Bool(true)),
            Some(b'f') => self.keyword("false", Value::Bool(false)),
            Some(b'n') => self.keyword("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(other) => Err(self.err(format!("unexpected `{}`", *other as char))),
            None => Err(self.err("a JSON value was expected")),
        }
    }

    fn keyword(&mut self, word: &str, value: Value) -> Result<Value, JsonError> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.err(format!("`{word}` was expected")))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.pos += 1; // {
        let mut entries = BTreeMap::new();
        self.skip_whitespace();
        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Value::Map(entries));
        }
        loop {
            self.skip_whitespace();
            let key_offset = self.pos;
            if self.bytes.get(self.pos) != Some(&b'"') {
                return Err(self.err("an object key (a string) was expected"));
            }
            let key = self.string()?;
            if entries.contains_key(&key) {
                return Err(JsonError {
                    offset: key_offset,
                    message: format!("duplicate key `{key}` — never a silent last-wins"),
                });
            }
            self.skip_whitespace();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err(self.err("`:` was expected"));
            }
            self.pos += 1;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            entries.insert(key, value);
            self.skip_whitespace();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Map(entries));
                }
                _ => return Err(self.err("`,` or `}` was expected")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.pos += 1; // [
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.err("`,` or `]` was expected")),
            }
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            match self.bytes.get(self.pos) {
                None => return Err(self.err("the string never closes")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.bytes.get(self.pos) {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{0008}'),
                        Some(b'f') => out.push('\u{000C}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            let unit = self.hex_unit()?;
                            if (0xD800..=0xDBFF).contains(&unit) {
                                // A surrogate pair.
                                if self.bytes.get(self.pos + 1) != Some(&b'\\')
                                    || self.bytes.get(self.pos + 2) != Some(&b'u')
                                {
                                    return Err(self.err("a low surrogate was expected"));
                                }
                                self.pos += 2;
                                let low = self.hex_unit()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(self.err("an invalid surrogate pair"));
                                }
                                let code = 0x10000
                                    + ((u32::from(unit) - 0xD800) << 10)
                                    + (u32::from(low) - 0xDC00);
                                out.push(
                                    char::from_u32(code)
                                        .ok_or_else(|| self.err("an invalid code point"))?,
                                );
                            } else if (0xDC00..=0xDFFF).contains(&unit) {
                                return Err(self.err("a lone low surrogate"));
                            } else {
                                out.push(
                                    char::from_u32(u32::from(unit))
                                        .ok_or_else(|| self.err("an invalid code point"))?,
                                );
                            }
                        }
                        _ => return Err(self.err("an unknown escape")),
                    }
                    self.pos += 1;
                }
                Some(&byte) if byte < 0x20 => {
                    return Err(self.err("a control character must be escaped"));
                }
                Some(_) => {
                    // Advance one UTF-8 scalar.
                    let start = self.pos;
                    let len = utf8_len(self.bytes[start]);
                    let end = start + len;
                    let slice = self
                        .bytes
                        .get(start..end)
                        .ok_or_else(|| self.err("truncated UTF-8"))?;
                    let ch = core::str::from_utf8(slice).map_err(|_| self.err("invalid UTF-8"))?;
                    out.push_str(ch);
                    self.pos = end;
                }
            }
        }
    }

    fn hex_unit(&mut self) -> Result<u16, JsonError> {
        let start = self.pos + 1;
        let slice = self
            .bytes
            .get(start..start + 4)
            .ok_or_else(|| self.err("four hex digits were expected"))?;
        let text = core::str::from_utf8(slice).map_err(|_| self.err("invalid UTF-8"))?;
        let unit =
            u16::from_str_radix(text, 16).map_err(|_| self.err("four hex digits were expected"))?;
        self.pos = start + 3;
        Ok(unit)
    }

    /// A JSON number, EXACT (D-2026-08-10-7): normalize the decimal
    /// (trailing zeros stripped); scale 0 in the signed 64-bit range
    /// → the integer domain (`-0` → integer zero); anything else a
    /// dynamic decimal within the budget; beyond it, refusal.
    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.pos;
        let negative = self.bytes.get(self.pos) == Some(&b'-');
        if negative {
            self.pos += 1;
        }
        let int_digits = self.digit_run()?;
        if int_digits.len() > 1 && int_digits.starts_with('0') {
            return Err(JsonError {
                offset: start,
                message: String::from("leading zeros are not JSON"),
            });
        }
        let mut coefficient = String::from(&int_digits);
        let mut scale: i64 = 0;
        if self.bytes.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            let frac = self.digit_run()?;
            scale = i64::try_from(frac.len()).unwrap_or(i64::MAX);
            coefficient.push_str(&frac);
        }
        if let Some(b'e' | b'E') = self.bytes.get(self.pos) {
            self.pos += 1;
            let exp_negative = match self.bytes.get(self.pos) {
                Some(b'+') => {
                    self.pos += 1;
                    false
                }
                Some(b'-') => {
                    self.pos += 1;
                    true
                }
                _ => false,
            };
            let exp_digits = self.digit_run()?;
            let exponent: i64 = exp_digits.parse().map_err(|_| JsonError {
                offset: start,
                message: String::from("the exponent is out of range"),
            })?;
            // Checked, total: an extreme exponent refuses instead
            // of a debug-build panic.
            scale = scale
                .checked_add(if exp_negative { exponent } else { -exponent })
                .ok_or_else(|| JsonError {
                    offset: start,
                    message: String::from("the exponent is out of range"),
                })?;
        }
        // Normalize: strip trailing zeros from the coefficient.
        let mut digits = String::from(coefficient.trim_start_matches('0'));
        if digits.is_empty() {
            // Zero of any spelling — `-0` included — is integer 0.
            return Ok(Value::Int(0));
        }
        while digits.ends_with('0') {
            // Trailing zeros normalize away; beyond the written
            // fraction they fold into a negative scale. Checked —
            // totality over adversarial zero runs.
            digits.pop();
            scale = scale.checked_sub(1).ok_or_else(|| JsonError {
                offset: start,
                message: String::from("the scale is outside the decimal128 domain"),
            })?;
        }
        if digits.len() as u32 > MAX_DIGITS {
            return Err(JsonError {
                offset: start,
                message: format!(
                    "more than {MAX_DIGITS} significant digits — narrowed nothing, refused instead"
                ),
            });
        }
        if scale > i64::from(MAX_SCALE) || scale < i64::from(MIN_SCALE) {
            return Err(JsonError {
                offset: start,
                message: String::from("the scale is outside the decimal128 domain"),
            });
        }
        let mut unscaled: i128 = digits.parse().map_err(|_| JsonError {
            offset: start,
            message: String::from("the coefficient is out of range"),
        })?;
        if negative {
            unscaled = -unscaled;
        }
        #[allow(clippy::cast_possible_truncation)]
        let scale = scale as i32;
        // The integer rule: a non-fractional value fitting i64 IS an
        // int — a negative scale expands when the product fits
        // (`1e2` → 100), stays an exact decimal when it does not
        // (`1e30` — beyond i64, still lossless).
        if scale <= 0 {
            let mut expanded = Some(unscaled);
            for _ in 0..scale.unsigned_abs() {
                expanded = expanded.and_then(|value| value.checked_mul(10));
            }
            if let Some(expanded) = expanded
                && let Ok(int) = i64::try_from(expanded)
            {
                return Ok(Value::Int(int));
            }
        }
        Ok(Value::Decimal { unscaled, scale })
    }

    fn digit_run(&mut self) -> Result<String, JsonError> {
        let start = self.pos;
        while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.err("a digit was expected"));
        }
        Ok(String::from(
            core::str::from_utf8(&self.bytes[start..self.pos])
                .unwrap_or_else(|_| unreachable!("digits are ASCII")),
        ))
    }
}

fn utf8_len(byte: u8) -> usize {
    match byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn parsed(text: &str) -> Value {
        parse_value(text).expect("parses").0
    }

    #[test]
    fn every_json_form_parses() {
        assert_eq!(parsed("42"), Value::Int(42));
        assert_eq!(parsed("\"x\""), Value::Text(String::from("x")));
        assert_eq!(parsed("true"), Value::Bool(true));
        assert_eq!(parsed("null"), Value::Null);
        assert_eq!(
            parsed("[1, 2]"),
            Value::Array(vec![Value::Int(1), Value::Int(2)])
        );
        let Value::Map(map) = parsed("{\"a\": 1}") else {
            panic!("an object is a map");
        };
        assert_eq!(map.get("a"), Some(&Value::Int(1)));
    }

    #[test]
    fn numbers_canonicalize_to_one_identity() {
        // `1`, `1.0`, `1e0` are ONE identity: int 1 (D-2026-08-10-7).
        assert_eq!(parsed("1"), Value::Int(1));
        assert_eq!(parsed("1.0"), Value::Int(1));
        assert_eq!(parsed("1e0"), Value::Int(1));
        assert_eq!(parsed("1.000e0"), Value::Int(1));
        // `-0` of any spelling is integer zero.
        assert_eq!(parsed("-0"), Value::Int(0));
        assert_eq!(parsed("-0.0"), Value::Int(0));
        // `1e2` is integral: int 100.
        assert_eq!(parsed("1e2"), Value::Int(100));
        // A true fraction is an exact decimal at minimal scale.
        assert_eq!(
            parsed("19.99"),
            Value::Decimal {
                unscaled: 1999,
                scale: 2
            }
        );
        // Trailing-zero fractions strip.
        assert_eq!(
            parsed("1.50"),
            Value::Decimal {
                unscaled: 15,
                scale: 1
            }
        );
        // Beyond i64, still exact — a scale-0 decimal.
        assert_eq!(
            parsed("9223372036854775808"),
            Value::Decimal {
                unscaled: 9_223_372_036_854_775_808_i128,
                scale: 0
            }
        );
    }

    #[test]
    fn budgets_refuse_instead_of_narrowing() {
        // 39 significant digits: refused, never rounded.
        assert!(parse_value("123456789012345678901234567890123456789").is_err());
        // A scale outside decimal128: refused.
        assert!(parse_value("1e99999").is_err());
        // The double 2^53+1 trap CANNOT happen: exact or refused.
        assert_eq!(
            parsed("9007199254740993"),
            Value::Int(9_007_199_254_740_993)
        );
    }

    #[test]
    fn duplicate_keys_refuse_with_their_offset() {
        let error = parse_value("{\"a\": 1, \"a\": 2}").unwrap_err();
        assert_eq!(error.offset, 9, "the SECOND key is named");
    }

    #[test]
    fn the_reader_reports_consumed_length() {
        let (value, consumed) = parse_value("[1] trailing").expect("parses");
        assert_eq!(value, Value::Array(vec![Value::Int(1)]));
        assert_eq!(consumed, 3, "trailing input is the caller's");
    }
}

#[cfg(test)]
mod oracle {
    //! The serde_json differential oracle (D-2026-08-10-7), scoped
    //! to what it can actually verify: structure, strings, booleans,
    //! nulls, and integers within i64 — the COMMON domain. Exact
    //! fractions and Rastok's stricter refusals (duplicate keys,
    //! over-budget numbers) are outside serde's vocabulary and are
    //! asserted directly in the unit tests above.

    use super::*;

    fn agrees(ours: &Value, theirs: &serde_json::Value) -> bool {
        match (ours, theirs) {
            (Value::Null, serde_json::Value::Null) => true,
            (Value::Bool(a), serde_json::Value::Bool(b)) => a == b,
            (Value::Text(a), serde_json::Value::String(b)) => a == b,
            (Value::Int(a), serde_json::Value::Number(b)) => b.as_i64() == Some(*a),
            (Value::Decimal { .. }, serde_json::Value::Number(_)) => {
                // Outside the oracle's exact vocabulary — the unit
                // tests own this half.
                true
            }
            (Value::Array(a), serde_json::Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| agrees(x, y))
            }
            (Value::Map(a), serde_json::Value::Object(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(key, value)| {
                        b.get(key.as_str())
                            .is_some_and(|other| agrees(value, other))
                    })
            }
            _ => false,
        }
    }

    #[test]
    fn the_common_domain_parses_identically() {
        let corpus = [
            "null",
            "true",
            "false",
            "0",
            "-1",
            "42",
            "9007199254740993",
            "\"hello\"",
            "\"esc \\n \\u00e9 \\ud83d\\ude00\"",
            "[]",
            "[1, 2, 3]",
            "[[[1]]]",
            "{}",
            "{\"a\": 1}",
            "{\"a\": {\"b\": [true, null, \"x\"]}}",
            "  [ 1 , {\"k\" : \"v\"} ]  ",
        ];
        for input in corpus {
            let (ours, _) = parse_value(input).expect(input);
            let theirs: serde_json::Value = serde_json::from_str(input).expect(input);
            assert!(agrees(&ours, &theirs), "diverged on `{input}`: {ours:?}");
        }
    }

    #[test]
    fn shared_refusals_agree() {
        // Syntax both must refuse — the oracle checks we are not
        // LOOSER than JSON.
        for input in ["{", "[1,", "{\"a\"}", "tru", "01", "+1", "\"\\q\"", "'x'"] {
            assert!(parse_value(input).is_err(), "`{input}` must refuse");
            assert!(
                serde_json::from_str::<serde_json::Value>(input).is_err(),
                "the oracle also refuses `{input}`"
            );
        }
    }
}

/// Writes one envelope value as CANONICAL JSON (D-2026-08-11-1):
/// object keys sorted by the key's UTF-8 bytes, structs and maps
/// alike. NOTE this is the WRITER's own canonical order: the
/// envelope seal sorts struct fields the same way but sorts MAP
/// entries by their tagged CBOR bytes (length-prefixed, so `"b"`
/// precedes `"aa"` there) — two authorities, two orders, both
/// deterministic; the wire speaks the writer's. Numbers exact (`int` plain, `decimal` in plain decimal
/// notation at its sealed scale, `double` shortest round-trip),
/// strings JSON-escaped. These bytes are `request_digest` material —
/// the writer is part of identity.
///
/// # Errors
/// A located refusal for the valueless forms: `bytes` has no JSON
/// representation, and a non-finite `double` is not JSON.
pub fn write_value(value: &crate::envelope::Value) -> Result<String, JsonError> {
    let mut out = String::new();
    write_into(value, &mut out, 0)?;
    Ok(out)
}

fn write_into(
    value: &crate::envelope::Value,
    out: &mut String,
    depth: usize,
) -> Result<(), JsonError> {
    // The reader's nesting budget, mirrored: what the writer emits,
    // the reader must accept.
    if depth > MAX_DEPTH {
        return Err(JsonError {
            offset: 0,
            message: String::from("nesting beyond the writer budget"),
        });
    }
    use crate::envelope::Value;
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
        Value::Int(v) => {
            out.push_str(&alloc::format!("{v}"));
        }
        Value::Uint(v) => out.push_str(&alloc::format!("{v}")),
        Value::Double(v) => {
            if !v.is_finite() {
                return Err(JsonError {
                    offset: 0,
                    message: String::from("a non-finite double is not JSON"),
                });
            }
            // Rust's shortest round-trip formatting, and NOTHING
            // more: the inbound domain is dynamic, so an integral
            // double's line legitimately reads back as int — the
            // static tag does not survive the wire, and forcing a
            // fraction would only make the form non-shortest
            // (D-2026-08-11-1, amended).
            out.push_str(&alloc::format!("{v}"));
        }
        Value::Text(v) => write_string(v, out),
        Value::Bytes(_) => {
            return Err(JsonError {
                offset: 0,
                message: String::from(
                    "bytes have no JSON form — the lines codec carries opaque payloads",
                ),
            });
        }
        Value::Decimal { unscaled, scale } => {
            write_decimal(*unscaled, *scale, out)?;
        }
        Value::Duration(_) | Value::Timestamp(_) => {
            return Err(JsonError {
                offset: 0,
                message: String::from("duration and timestamp require an explicit JSON projection"),
            });
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_into(item, out, depth + 1)?;
            }
            out.push(']');
        }
        Value::Struct(fields) => {
            // Sorted by the field name's UTF-8 bytes — here this
            // coincides with the seal's struct order: two
            // by-name-equal values, one digest.
            let mut sorted: Vec<(&String, &Value)> =
                fields.iter().map(|(name, value)| (name, value)).collect();
            sorted.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (index, (name, value)) in sorted.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(name, out);
                out.push(':');
                write_into(value, out, depth + 1)?;
            }
            out.push('}');
        }
        Value::Map(entries) => {
            out.push('{');
            for (index, (key, value)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_into(value, out, depth + 1)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&alloc::format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// Plain decimal notation at the sealed scale: `1999 / scale 2` is
/// `19.99`; a negative scale appends zeros (`5 / scale -2` is
/// `500` — reachable only for magnitudes past i64, the
/// written-scale fold collapsing the rest, D-2026-08-13-1). The
/// domain check is defensive: the doors hold every travelling
/// decimal to [`decimal_in_domain`], so an out-of-domain scale here
/// is a refusal, never a runaway allocation.
fn write_decimal(unscaled: i128, scale: i32, out: &mut String) -> Result<(), JsonError> {
    if !decimal_in_domain(unscaled, scale) {
        return Err(JsonError {
            offset: 0,
            message: String::from("a decimal outside the decimal128 domain is not writable"),
        });
    }
    if unscaled < 0 {
        out.push('-');
    }
    let digits = alloc::format!("{}", unscaled.unsigned_abs());
    if scale <= 0 {
        out.push_str(&digits);
        for _ in 0..scale.unsigned_abs() {
            out.push('0');
        }
        return Ok(());
    }
    let scale = scale as usize;
    if digits.len() > scale {
        let point = digits.len() - scale;
        out.push_str(&digits[..point]);
        out.push('.');
        out.push_str(&digits[point..]);
    } else {
        out.push_str("0.");
        for _ in 0..scale - digits.len() {
            out.push('0');
        }
        out.push_str(&digits);
    }
    Ok(())
}

#[cfg(test)]
mod writer_tests {
    use super::*;
    use crate::envelope::Value;
    use alloc::vec;

    #[test]
    fn the_writer_is_canonical_and_exact() {
        // Struct keys sort — two by-name-equal values, one byte
        // sequence (the digest rule).
        let ab = Value::Struct(vec![
            (String::from("b"), Value::Int(2)),
            (String::from("a"), Value::Int(1)),
        ]);
        assert_eq!(write_value(&ab).unwrap(), "{\"a\":1,\"b\":2}");
        // Decimal at its sealed scale, exact.
        assert_eq!(
            write_value(&Value::Decimal {
                unscaled: 1999,
                scale: 2
            })
            .unwrap(),
            "19.99"
        );
        assert_eq!(
            write_value(&Value::Decimal {
                unscaled: 5,
                scale: 4
            })
            .unwrap(),
            "0.0005"
        );
        // An integral double writes SHORTEST — and its line reads
        // back as int (the inbound domain is dynamic; the static
        // tag does not survive the wire).
        assert_eq!(write_value(&Value::Double(2.0)).unwrap(), "2");
        assert_eq!(write_value(&Value::Double(2.5)).unwrap(), "2.5");
        let (round, _) = parse_value("2").unwrap();
        assert_eq!(round, crate::value::Value::Int(2));
        // Escapes.
        assert_eq!(
            write_value(&Value::Text(String::from("a\"\n"))).unwrap(),
            "\"a\\\"\\n\""
        );
    }

    #[test]
    fn the_writer_round_trips_through_the_reader() {
        let value = Value::Struct(vec![
            (
                String::from("price"),
                Value::Decimal {
                    unscaled: 1999,
                    scale: 2,
                },
            ),
            (String::from("n"), Value::Int(42)),
            (
                String::from("tags"),
                Value::Array(vec![Value::Text(String::from("x"))]),
            ),
        ]);
        let written = write_value(&value).unwrap();
        let (read, _) = parse_value(&written).expect("the writer's output parses");
        // A struct reads back as a MAP (the JSON model) — compare
        // through a second write: one canonical byte sequence.
        assert_eq!(
            write_value(&read).unwrap(),
            written,
            "byte-stable round trip"
        );
    }

    #[test]
    fn the_valueless_forms_refuse() {
        assert!(write_value(&Value::Bytes(vec![1])).is_err());
        assert!(write_value(&Value::Double(f64::NAN)).is_err());
        assert!(write_value(&Value::Double(f64::INFINITY)).is_err());
    }

    #[test]
    fn the_wire_order_of_maps_is_the_keys_utf8_bytes() {
        // {"b", "aa"}: the envelope SEAL orders `b` first (tagged
        // CBOR bytes are length-prefixed); the WIRE orders `aa`
        // first (plain UTF-8 key order). Both deterministic, each
        // authority owns its own bytes — this pins the wire's.
        let mut entries = alloc::collections::BTreeMap::new();
        entries.insert(String::from("b"), Value::Int(1));
        entries.insert(String::from("aa"), Value::Int(2));
        assert_eq!(
            write_value(&Value::Map(entries)).unwrap(),
            r#"{"aa":2,"b":1}"#
        );
    }

    #[test]
    fn null_writes_as_null() {
        assert_eq!(write_value(&Value::Null).unwrap(), "null");
        let round = parse_value(&write_value(&Value::Array(vec![Value::Null])).unwrap()).unwrap();
        assert_eq!(
            round.0,
            crate::value::Value::Array(vec![crate::value::Value::Null])
        );
    }

    #[test]
    fn the_decimal_domain_bounds_hold_exactly() {
        // Both edges write; one step beyond refuses — instantly,
        // never as a runaway allocation.
        for scale in [MIN_SCALE, MAX_SCALE] {
            assert!(
                write_value(&Value::Decimal { unscaled: 7, scale }).is_ok(),
                "scale {scale} is inside the domain"
            );
        }
        for scale in [MIN_SCALE - 1, MAX_SCALE + 1, i32::MIN, i32::MAX] {
            assert!(
                write_value(&Value::Decimal { unscaled: 7, scale }).is_err(),
                "scale {scale} refuses"
            );
        }
        let too_wide = 10_i128.pow(MAX_DIGITS);
        assert!(
            write_value(&Value::Decimal {
                unscaled: too_wide,
                scale: 2
            })
            .is_err(),
            "39 digits refuse"
        );
    }

    #[test]
    fn the_writer_mirrors_the_reader_nesting_budget() {
        let nest = |depth: usize| {
            let mut value = Value::Int(1);
            for _ in 0..depth {
                value = Value::Array(vec![value]);
            }
            value
        };
        assert!(write_value(&nest(MAX_DEPTH)).is_ok(), "the budget's edge");
        assert!(
            write_value(&nest(MAX_DEPTH + 1)).is_err(),
            "one beyond refuses"
        );
    }

    #[test]
    fn the_oracle_agrees_with_the_writer() {
        // serde_json parses everything the writer emits (common
        // domain: no decimals beyond f64 here).
        let value = Value::Map(
            [
                (
                    String::from("k\n"),
                    Value::Array(vec![Value::Bool(true), Value::Int(-3)]),
                ),
                (String::from("z"), Value::Double(1.5)),
            ]
            .into_iter()
            .collect(),
        );
        let written = write_value(&value).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
        assert_eq!(parsed["k\n"][1], serde_json::json!(-3));
        assert_eq!(parsed["z"], serde_json::json!(1.5));
    }
}
