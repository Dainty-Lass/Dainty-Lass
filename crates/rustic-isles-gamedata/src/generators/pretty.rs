// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `JSON.stringify(value, null, n)` as V8 writes it (spec §2.1).

use serde_json::{Map, Value};

/// The canonical decimal text of an integer below 4294967295: V8 lists such keys first, ascending.
fn index_key(key: &str) -> Option<u32> {
    let canonical = key == "0" || (key.starts_with(|c: char| ('1'..='9').contains(&c)) && key.len() <= 10);
    if !canonical || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u32>().ok().filter(|&n| n < u32::MAX)
}

pub(crate) fn ordered(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut indexed: Vec<(u32, &String, &Value)> = Vec::new();
    let mut out = Vec::with_capacity(map.len());
    for (k, v) in map {
        match index_key(k) {
            Some(n) => indexed.push((n, k, v)),
            None => out.push((k, v)),
        }
    }
    indexed.sort_by_key(|(n, _, _)| *n);
    let mut keys: Vec<(&String, &Value)> = indexed.into_iter().map(|(_, k, v)| (k, v)).collect();
    keys.append(&mut out);
    keys
}

fn scalar(value: &Value) -> String {
    service_kit::jsjson::stringify(value)
}

fn newline(out: &mut String, depth: usize, indent: usize) {
    out.push('\n');
    out.extend(std::iter::repeat_n(' ', depth * indent));
}

fn write(out: &mut String, value: &Value, depth: usize, indent: usize) {
    match value {
        Value::Array(items) if !items.is_empty() => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1, indent);
                write(out, item, depth + 1, indent);
            }
            newline(out, depth, indent);
            out.push(']');
        }
        Value::Object(map) if !map.is_empty() => {
            out.push('{');
            for (i, (k, v)) in ordered(map).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1, indent);
                out.push_str(&scalar(&Value::String(k.clone())));
                out.push_str(": ");
                write(out, v, depth + 1, indent);
            }
            newline(out, depth, indent);
            out.push('}');
        }
        other => out.push_str(&scalar(other)),
    }
}

/// `JSON.stringify(value, null, indent)`: compact at 0, no trailing newline.
pub fn stringify_pretty(value: &Value, indent: usize) -> String {
    if indent == 0 {
        return service_kit::jsjson::stringify(value);
    }
    let mut out = String::new();
    write(&mut out, value, 0, indent);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v8_layout() {
        let value = json!({"b": 1, "10": 2, "9": 3, "a": [], "o": {}});
        assert_eq!(
            stringify_pretty(&value, 2),
            "{\n  \"9\": 3,\n  \"10\": 2,\n  \"b\": 1,\n  \"a\": [],\n  \"o\": {}\n}"
        );
        let nested = json!({"x": [1.0, {"y": "\u{1}\n\"é"}], "4294967295": 0, "01": 1});
        assert_eq!(
            stringify_pretty(&nested, 1),
            "{\n \"x\": [\n  1,\n  {\n   \"y\": \"\\u0001\\n\\\"é\"\n  }\n ],\n \"4294967295\": 0,\n \"01\": 1\n}"
        );
        assert_eq!(
            stringify_pretty(&nested, 0),
            r#"{"x":[1,{"y":"\u0001\n\"é"}],"4294967295":0,"01":1}"#
        );
        assert_eq!(stringify_pretty(&json!(1e21), 2), "1e+21");
        assert_eq!(stringify_pretty(&json!([]), 3), "[]");
        for n in 0..=4 {
            let back = service_kit::jsjson::parse(&stringify_pretty(&nested, n)).unwrap();
            assert!(service_kit::jsjson::deep_equal(
                &back,
                &service_kit::jsjson::to_js(&nested)
            ));
        }
    }
}
