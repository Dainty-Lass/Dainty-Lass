//! JSON as Node reads and writes it: `express.json` for the P2P paths (`ParseP2PJson`, 72 KiB, strict) and
//! `JSON.stringify`'s text, for replies, log lines and the byte counts the Guard budgets charge.

use serde_json::Value;

/// `P2P_MAX_HTTP_BODY_BYTES`: an evidence batch (at most 64 KiB of events) and its envelope.
pub const P2P_MAX_HTTP_BODY_BYTES: usize = 72 * 1024;

/// Why the P2P parser refused a body (`P2PBodyErrorHandler`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyError {
    /// `entity.too.large`: `GUARD_BATCH_TOO_LARGE`.
    TooLarge,
    /// A `SyntaxError` from the parser: `GUARD_BATCH_INVALID`.
    Invalid,
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Whether `type-is` sees `application/json` (parameters allowed), the only type `express.json()` parses by default.
fn is_json_type(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
}

/// `express.json({ limit: "72b"*1024, strict: true })` under Express 5: `Ok(None)` is `req.body` left undefined (no
/// body, or not JSON). Bytes that are not UTF-8 are decoded with replacement characters, as `iconv` does.
pub fn parse_p2p_body(headers: &[(String, String)], body: &[u8]) -> Result<Option<Value>, BodyError> {
    if !is_json_type(header(headers, "content-type")) {
        return Ok(None);
    }
    let has_body = header(headers, "transfer-encoding").is_some()
        || header(headers, "content-length").is_some_and(|value| value.trim().parse::<u64>().is_ok())
        || !body.is_empty();
    if !has_body {
        return Ok(None);
    }
    if body.len() > P2P_MAX_HTTP_BODY_BYTES {
        return Err(BodyError::TooLarge);
    }
    if body.is_empty() {
        return Ok(Some(Value::Object(serde_json::Map::new())));
    }
    let text = String::from_utf8_lossy(body);
    // strict: only an object or an array, after JSON whitespace
    match text.trim_start_matches([' ', '\t', '\n', '\r']).chars().next() {
        Some('{' | '[') => {}
        _ => return Err(BodyError::Invalid),
    }
    serde_json::from_str(&text).map(Some).map_err(|_| BodyError::Invalid)
}

/// A JSON body parsed leniently, for the admin routes behind the app's own parser: anything unreadable is undefined.
pub fn parse_admin_body(body: &[u8]) -> Option<Value> {
    if body.is_empty() {
        return None;
    }
    serde_json::from_slice(body).ok()
}

/// `JSON.stringify` of a string.
pub fn js_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    push_js_string(&mut out, text);
    out
}

fn push_js_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `Number.prototype.toString()` for a finite number (ECMA-262 `Number::toString`, radix 10).
pub fn js_number(value: f64) -> String {
    if value.is_nan() || value.is_infinite() {
        // JSON.stringify writes these as null
        return "null".to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let negative = value < 0.0;
    // Rust's shortest round-trip digits, as d.ddde±x
    let exp_form = format!("{:e}", value.abs());
    let (mantissa, exponent) = exp_form.split_once('e').unwrap_or((exp_form.as_str(), "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i64 = exponent.parse().unwrap_or(0);
    let k = digits.len() as i64;
    let n = exponent + 1;
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-n) as usize));
        out.push_str(&digits);
    } else {
        let e = n - 1;
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if e < 0 { '-' } else { '+' });
        out.push_str(&e.abs().to_string());
    }
    out
}

/// `JSON.stringify` of a parsed value. Keys come out sorted (`serde_json`'s map), not in insertion order: the length
/// and the canonical hash input are what this is used for.
pub fn stringify(value: &Value) -> String {
    let mut out = String::new();
    push_value(&mut out, value);
    out
}

fn push_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&js_number(number.as_f64().unwrap_or(f64::NAN))),
        Value::String(text) => push_js_string(out, text),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                push_value(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                push_js_string(out, key);
                out.push(':');
                push_value(out, item);
            }
            out.push('}');
        }
    }
}

/// `Buffer.byteLength(JSON.stringify(req.body ?? null), "utf8")`: what one request charges the Guard budgets.
pub fn stringified_len(body: Option<&Value>) -> u64 {
    body.map_or(4, |value| stringify(value).len() as u64)
}

/// `Number.isSafeInteger` of a JSON value.
pub fn safe_integer(value: &Value) -> Option<i64> {
    const MAX: f64 = 9_007_199_254_740_991.0;
    let number = value.as_f64()?;
    (number.fract() == 0.0 && number.abs() <= MAX).then_some(number as i64)
}

/// The UTF-16 length JavaScript reports as `.length`.
pub fn js_length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `text.slice(0, units)` counted in UTF-16 units, never splitting a character.
pub fn js_slice(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (index, c) in text.char_indices() {
        used += c.len_utf16();
        if used > units {
            return &text[..index];
        }
    }
    text
}

/// `String.prototype.trim`: Unicode white space, line terminators and the BOM.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// `Number(value)` for what a JSON body can hold; `None` is `NaN`.
pub fn js_to_number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Null => Some(0.0),
        Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        Value::Number(number) => number.as_f64(),
        Value::String(text) => string_to_number(text),
        Value::Array(items) => match items.as_slice() {
            [] => Some(0.0),
            [Value::Null] => Some(0.0),
            [Value::String(text)] => string_to_number(text),
            [Value::Number(number)] => number.as_f64(),
            [Value::Array(inner)] => js_to_number(Some(&Value::Array(inner.clone()))),
            _ => None,
        },
        Value::Object(_) => None,
    }
}

fn string_to_number(text: &str) -> Option<f64> {
    let trimmed = js_trim(text);
    if trimmed.is_empty() {
        return Some(0.0);
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = trimmed.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix).ok().map(|value| value as f64);
        }
    }
    match trimmed {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let valid = trimmed
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !valid {
        return None;
    }
    trimmed.parse::<f64>().ok()
}

/// `Number.isSafeInteger(Number(value))`.
pub fn js_safe_integer(value: Option<&Value>) -> Option<i64> {
    const MAX: f64 = 9_007_199_254_740_991.0;
    let number = js_to_number(value)?;
    (number.is_finite() && number.fract() == 0.0 && number.abs() <= MAX).then_some(number as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn json_headers() -> Vec<(String, String)> {
        vec![("content-type".to_owned(), "application/json; charset=utf-8".to_owned())]
    }

    #[test]
    fn numbers_print_as_javascript_prints_them() {
        for (value, text) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (2.5e25, "2.5e+25"),
            (9007199254740993.0, "9007199254740992"),
        ] {
            assert_eq!(js_number(value), text, "{value}");
        }
    }

    #[test]
    fn strings_escape_as_json_stringify() {
        assert_eq!(
            js_string("a\"b\\c\n\u{1}\u{2028}é"),
            "\"a\\\"b\\\\c\\n\\u0001\u{2028}é\""
        );
    }

    #[test]
    fn the_p2p_parser_is_strict_and_bounded() {
        let headers = json_headers();
        assert_eq!(parse_p2p_body(&headers, b"{\"a\":1}"), Ok(Some(json!({"a": 1}))));
        assert_eq!(parse_p2p_body(&headers, b" [1]"), Ok(Some(json!([1]))));
        assert_eq!(parse_p2p_body(&headers, b"1"), Err(BodyError::Invalid));
        assert_eq!(parse_p2p_body(&headers, b"\"text\""), Err(BodyError::Invalid));
        assert_eq!(parse_p2p_body(&headers, b"   "), Err(BodyError::Invalid));
        assert_eq!(parse_p2p_body(&headers, b"{"), Err(BodyError::Invalid));
        let big = vec![b' '; P2P_MAX_HTTP_BODY_BYTES + 1];
        assert_eq!(parse_p2p_body(&headers, &big), Err(BodyError::TooLarge));
        assert_eq!(parse_p2p_body(&headers, b""), Ok(None), "no body at all");
        let mut sized = headers.clone();
        sized.push(("content-length".to_owned(), "0".to_owned()));
        assert_eq!(
            parse_p2p_body(&sized, b""),
            Ok(Some(json!({}))),
            "an empty JSON body is {{}}"
        );
        assert_eq!(parse_p2p_body(&[], b"{\"a\":1}"), Ok(None), "not JSON: left undefined");
        assert_eq!(
            parse_p2p_body(&headers, b"{\"a\":1,\"a\":2}"),
            Ok(Some(json!({"a": 2})))
        );
    }

    #[test]
    fn byte_counts_follow_the_stringified_body() {
        assert_eq!(stringified_len(None), 4);
        assert_eq!(
            stringified_len(Some(&json!({"a": [1, 2.50, "é"]}))),
            r#"{"a":[1,2.5,"é"]}"#.len() as u64
        );
    }

    #[test]
    fn number_conversion_follows_javascript() {
        assert_eq!(js_safe_integer(Some(&json!("7"))), Some(7));
        assert_eq!(js_safe_integer(Some(&json!(" 0x10 "))), Some(16));
        assert_eq!(js_safe_integer(Some(&json!(null))), Some(0));
        assert_eq!(js_safe_integer(Some(&json!(true))), Some(1));
        assert_eq!(js_safe_integer(Some(&json!([3]))), Some(3));
        assert_eq!(js_safe_integer(Some(&json!(""))), Some(0));
        assert_eq!(js_safe_integer(None), None);
        assert_eq!(js_safe_integer(Some(&json!("1.5"))), None);
        assert_eq!(js_safe_integer(Some(&json!({}))), None);
        assert_eq!(js_safe_integer(Some(&json!("abc"))), None);
    }

    #[test]
    fn utf16_helpers() {
        assert_eq!(js_length("a😀"), 3);
        assert_eq!(js_slice("a😀b", 2), "a");
        assert_eq!(js_slice("a😀b", 3), "a😀");
        assert_eq!(js_trim("\u{feff} x \n"), "x");
    }
}
