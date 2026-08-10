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

use crate::cel::Value;

/// The significant-digit budget — Arrow `decimal128`.
pub const MAX_DIGITS: u32 = 38;
/// The scale bounds — the IEEE decimal128 exponent domain, exactly
/// as identity.md's registry pins it: scale ∈ [−6143, +6144].
pub const MAX_SCALE: i32 = 6144;
/// The lower scale bound (see [`MAX_SCALE`]).
pub const MIN_SCALE: i32 = -6143;

/// The D-2026-08-10-7 invariants of a DYNAMIC decimal, shared by
/// the package decoder and the plan validator (`encode ⇒ decode`
/// demands one rule): budget bounds, minimal scale, and the
/// integer-domain disjointness.
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
            return Ok(Value::List(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::List(items));
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
            Value::List(vec![Value::Int(1), Value::Int(2)])
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
        assert_eq!(value, Value::List(vec![Value::Int(1)]));
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
            (Value::List(a), serde_json::Value::Array(b)) => {
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
