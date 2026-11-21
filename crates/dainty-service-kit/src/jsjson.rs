// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! JSON as `JSON.stringify` writes it: numbers as JavaScript prints them and integer-like keys first.

use std::fmt::Write;

use serde_json::{Map, Number, Value};

fn array_index(key: &str) -> Option<u32> {
    if key == "0" {
        return Some(0);
    }
    if !key.starts_with(|c: char| c.is_ascii_digit() && c != '0') {
        return None;
    }
    if !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u32>().ok().filter(|&n| n < u32::MAX)
}

fn ordered_entries(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut indexed: Vec<(u32, &String, &Value)> = Vec::new();
    let mut others: Vec<(&String, &Value)> = Vec::with_capacity(map.len());

    for (k, v) in map {
        if let Some(idx) = array_index(k) {
            indexed.push((idx, k, v));
        } else {
            others.push((k, v));
        }
    }
    indexed.sort_by_key(|(idx, _, _)| *idx);

    let mut out = Vec::with_capacity(map.len());
    for (_, k, v) in indexed {
        out.push((k, v));
    }
    out.extend(others);
    out
}

/// `value` as JavaScript prints it; non-finite numbers are `null`.
pub fn number(value: f64) -> String {
    if !value.is_finite() {
        return "null".to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }

    let mut out = String::new();
    if value.is_sign_negative() {
        out.push('-');
    }
    let abs = value.abs();
    let sci = format!("{abs:e}");
    let (mantissa, exp_str) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp_str.parse().unwrap_or(0);

    if (-6..21).contains(&exp) {
        let mut digits = String::with_capacity(mantissa.len());
        for ch in mantissa.chars() {
            if ch != '.' {
                digits.push(ch);
            }
        }
        let k = digits.len() as i32;
        let n = exp + 1;
        if n <= 0 {
            out.push_str("0.");
            for _ in 0..(-n) {
                out.push('0');
            }
            out.push_str(&digits);
        } else if k <= n {
            out.push_str(&digits);
            for _ in 0..(n - k) {
                out.push('0');
            }
        } else {
            let split = n as usize;
            out.push_str(&digits[..split]);
            out.push('.');
            out.push_str(&digits[split..]);
        }
    } else {
        out.push_str(mantissa);
        if exp >= 0 {
            let _ = write!(out, "e+{exp}");
        } else {
            let _ = write!(out, "e{exp}");
        }
    }
    out
}

fn write_escaped_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                out.push_str(&number(f));
            } else {
                out.push_str("null");
            }
        }
        Value::String(s) => write_escaped_str(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in ordered_entries(map).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_escaped_str(out, k);
                out.push(':');
                write_value(out, v);
            }
            out.push('}');
        }
    }
}

/// `JSON.stringify(value)`.
pub fn stringify(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

/// `value` normalized as JavaScript would hold it: numbers as doubles, integer-like keys first.
pub fn to_js(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(*b),
        Value::String(s) => Value::String(s.clone()),
        Value::Number(n) => {
            let Some(f) = n.as_f64() else {
                return Value::Null;
            };
            if !f.is_finite() {
                return Value::Null;
            }
            if f == 0.0 && f.is_sign_negative() {
                return Number::from_f64(-0.0).map_or(Value::Null, Value::Number);
            }
            if f.fract() == 0.0 {
                if (0.0..=(u64::MAX as f64)).contains(&f) {
                    let u = f as u64;
                    if (u as f64) == f {
                        return Value::Number(Number::from(u));
                    }
                } else if ((i64::MIN as f64)..0.0).contains(&f) {
                    let i = f as i64;
                    if (i as f64) == f {
                        return Value::Number(Number::from(i));
                    }
                }
            }
            Number::from_f64(f).map_or(Value::Null, Value::Number)
        }
        Value::Array(items) => Value::Array(items.iter().map(to_js).collect()),
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in ordered_entries(map) {
                out.insert(k.clone(), to_js(v));
            }
            Value::Object(out)
        }
    }
}

/// `JSON.parse(text)`, normalized with [`to_js`].
pub fn parse(text: &str) -> Result<Value, serde_json::Error> {
    let val: Value = serde_json::from_str(text)?;
    Ok(to_js(&val))
}

/// Structural equality with numbers compared as doubles and NaN equal to NaN.
pub fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(ba), Value::Bool(bb)) => ba == bb,
        (Value::String(sa), Value::String(sb)) => sa == sb,
        (Value::Number(na), Value::Number(nb)) => {
            let fa = na.as_f64().unwrap_or(f64::NAN);
            let fb = nb.as_f64().unwrap_or(f64::NAN);
            if fa.is_nan() && fb.is_nan() {
                true
            } else {
                fa.to_bits() == fb.to_bits()
            }
        }
        (Value::Array(aa), Value::Array(ab)) => {
            aa.len() == ab.len() && aa.iter().zip(ab.iter()).all(|(x, y)| deep_equal(x, y))
        }
        (Value::Object(ma), Value::Object(mb)) => {
            ma.len() == mb.len() && ma.iter().all(|(k, va)| mb.get(k).is_some_and(|vb| deep_equal(va, vb)))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_numbers_matching_ecmascript() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(f64::NAN), "null");
        assert_eq!(number(f64::INFINITY), "null");
        assert_eq!(number(1e-6), "0.000001");
        assert_eq!(number(1.5e-7), "1.5e-7");
        assert_eq!(number(1e20), "100000000000000000000");
        assert_eq!(number(1e21), "1e+21");
        assert_eq!(number(-123.456), "-123.456");
    }

    #[test]
    fn orders_array_index_keys_before_string_keys() {
        let val = parse(r#"{"b":1,"10":2,"2":3,"a":4,"0":5,"01":6,"4294967295":7}"#).unwrap();
        assert_eq!(
            stringify(&val),
            r#"{"0":5,"2":3,"10":2,"b":1,"a":4,"01":6,"4294967295":7}"#
        );
    }

    #[test]
    fn deep_equal_distinguishes_signed_zero_and_ignores_key_order() {
        let pos_zero = Value::Number(Number::from_f64(0.0).unwrap());
        let neg_zero = Value::Number(Number::from_f64(-0.0).unwrap());
        assert!(!deep_equal(&pos_zero, &neg_zero));
        assert!(deep_equal(&json!({"a": 1, "b": 2}), &json!({"b": 2, "a": 1})));
    }
}
