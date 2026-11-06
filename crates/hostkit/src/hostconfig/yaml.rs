// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Minimal, zero-dependency YAML subset parser converting YAML configuration documents into
//! `serde_json::Value` (`docs/app-structure-plan.md`).
//!
//! Avoids pulling in the unmaintained `serde_yaml` crate (which is rejected by `cargo deny`) while
//! supporting standard server configuration YAML files (nested maps, inline arrays/maps, scalars,
//! quoted strings, booleans, numbers, and `#` comments). The host writes the file, but a hostile or
//! broken one must only ever be refused: nesting and size are capped, so nothing recurses without end
//! (docs/security/controller.md; the `host_config` fuzz target).

use serde_json::{Map, Number, Value};

/// A settings file is a few hundred bytes; this is far more than any needs.
pub const MAX_YAML_BYTES: usize = 256 * 1024;
/// Blocks inside blocks, and brackets inside brackets: host.json's deepest is 2.
pub const MAX_DEPTH: usize = 32;

fn too_deep() -> String {
    format!("nested more than {MAX_DEPTH} levels deep")
}

/// Parses the YAML subset a host.yaml uses into JSON; the error says what is wrong, and too big or too deep is refused.
pub fn parse_yaml_to_json(input: &str) -> Result<Value, String> {
    if input.len() > MAX_YAML_BYTES {
        return Err(format!("larger than {} KiB", MAX_YAML_BYTES / 1024));
    }
    let mut lines = Vec::new();
    for (idx, raw) in input.lines().enumerate() {
        let stripped = strip_comment(raw);
        let trimmed_end = stripped.trim_end();
        if trimmed_end.trim().is_empty() || trimmed_end.trim() == "---" || trimmed_end.trim() == "..." {
            continue;
        }
        let indent = trimmed_end.chars().take_while(|&c| c == ' ').count();
        if trimmed_end[indent..].starts_with('\t') {
            return Err(format!("line {}: tabs are not allowed for YAML indentation", idx + 1));
        }
        lines.push((idx + 1, indent, trimmed_end[indent..].to_string()));
    }
    if lines.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let mut pos = 0;
    parse_block(&lines, &mut pos, lines[0].1, 0)
}

fn strip_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (i, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_double => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double && (i == 0 || line[..i].ends_with(' ') || line[..i].ends_with('\t')) => {
                return &line[..i];
            }
            _ => {}
        }
    }
    line
}

fn parse_block(
    lines: &[(usize, usize, String)],
    pos: &mut usize,
    base_indent: usize,
    depth: usize,
) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(too_deep());
    }
    if *pos >= lines.len() {
        return Ok(Value::Object(Map::new()));
    }
    if lines[*pos].2.starts_with("- ") || lines[*pos].2 == "-" {
        return parse_seq(lines, pos, base_indent, depth);
    }
    let mut map = Map::new();
    while *pos < lines.len() {
        let (line_no, indent, ref content) = lines[*pos];
        if indent < base_indent {
            break;
        }
        if indent > base_indent {
            return Err(format!(
                "line {line_no}: unexpected indentation ({indent} > {base_indent})"
            ));
        }
        let Some((raw_key, raw_val)) = split_key_value(content) else {
            return Err(format!("line {line_no}: expected `key: value`, got `{content}`"));
        };
        let key = unquote(raw_key.trim());
        let val_trimmed = raw_val.trim();
        *pos += 1;
        if val_trimmed.is_empty() {
            if *pos < lines.len() && lines[*pos].1 > base_indent {
                let child_indent = lines[*pos].1;
                let child = parse_block(lines, pos, child_indent, depth + 1)?;
                map.insert(key, child);
            } else {
                map.insert(key, Value::Null);
            }
        } else {
            map.insert(key, parse_scalar_or_inline(val_trimmed, 0)?);
        }
    }
    Ok(Value::Object(map))
}

fn parse_seq(
    lines: &[(usize, usize, String)],
    pos: &mut usize,
    base_indent: usize,
    depth: usize,
) -> Result<Value, String> {
    let mut items = Vec::new();
    while *pos < lines.len() {
        let (line_no, indent, ref content) = lines[*pos];
        if indent < base_indent {
            break;
        }
        if indent > base_indent {
            return Err(format!("line {line_no}: unexpected list indentation"));
        }
        let Some(rest) = content.strip_prefix('-') else {
            break;
        };
        let rest = rest.trim();
        *pos += 1;
        if rest.is_empty() {
            if *pos < lines.len() && lines[*pos].1 > base_indent {
                let child_indent = lines[*pos].1;
                items.push(parse_block(lines, pos, child_indent, depth + 1)?);
            } else {
                items.push(Value::Null);
            }
        } else {
            items.push(parse_scalar_or_inline(rest, 0)?);
        }
    }
    Ok(Value::Array(items))
}

fn split_key_value(s: &str) -> Option<(&str, &str)> {
    let mut in_single = false;
    let mut in_double = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ':' if !in_single && !in_double => {
                let after = &s[i + 1..];
                if after.is_empty() || after.starts_with(' ') || after.starts_with('\t') {
                    return Some((&s[..i], after));
                }
            }
            _ => {}
        }
    }
    None
}

fn unquote(s: &str) -> String {
    if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) {
        s[1..s.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}

fn parse_scalar_or_inline(s: &str, depth: usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(too_deep());
    }
    if (s.starts_with('[') && s.ends_with(']')) || (s.starts_with('{') && s.ends_with('}')) {
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            return Ok(v);
        }
        if s.starts_with('[') && s.ends_with(']') {
            let inner = s[1..s.len() - 1].trim();
            if inner.is_empty() {
                return Ok(Value::Array(Vec::new()));
            }
            let mut arr = Vec::new();
            for part in inner.split(',') {
                arr.push(parse_scalar_or_inline(part.trim(), depth + 1)?);
            }
            return Ok(Value::Array(arr));
        }
    }
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        return Ok(Value::String(unquote(s)));
    }
    match s {
        "true" | "True" | "TRUE" => return Ok(Value::Bool(true)),
        "false" | "False" | "FALSE" => return Ok(Value::Bool(false)),
        "null" | "Null" | "NULL" | "~" => return Ok(Value::Null),
        _ => {}
    }
    if let Ok(i) = s.parse::<i64>() {
        return Ok(Value::Number(Number::from(i)));
    }
    if let Ok(u) = s.parse::<u64>() {
        return Ok(Value::Number(Number::from(u)));
    }
    if let Ok(f) = s.parse::<f64>()
        && let Some(n) = Number::from_f64(f)
    {
        return Ok(Value::Number(n));
    }
    Ok(Value::String(s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_yaml_config() {
        let yaml = r#"
# Server configuration
server_name: "Test Server"
gateway_port: 8443
admin_enabled: true
editions:
  reforged:
    enabled: true
    backend_port: 61000
  refined:
    enabled: false
    backend_port: 61010
"#;
        let val = parse_yaml_to_json(yaml).unwrap();
        assert_eq!(val["server_name"], "Test Server");
        assert_eq!(val["gateway_port"], 8443);
        assert_eq!(val["admin_enabled"], true);
        assert_eq!(val["editions"]["reforged"]["enabled"], true);
        assert_eq!(val["editions"]["refined"]["enabled"], false);
    }

    #[test]
    fn refuses_what_would_recurse_without_end_or_is_too_large() {
        let deep: String = (0..200)
            .map(|i| {
                format!(
                    "{}k{i}:
",
                    " ".repeat(i)
                )
            })
            .collect();
        assert!(parse_yaml_to_json(&deep).unwrap_err().contains("levels deep"));
        let brackets = format!(
            "k: {}a{}
",
            "[".repeat(5000),
            "]".repeat(5000)
        );
        assert!(parse_yaml_to_json(&brackets).unwrap_err().contains("levels deep"));
        let list: String = (0..200)
            .map(|i| {
                format!(
                    "{}-
",
                    " ".repeat(i * 2)
                )
            })
            .collect();
        assert!(parse_yaml_to_json(&list).is_err());
        assert!(
            parse_yaml_to_json(
                &"a: b
"
                .repeat(60_000)
            )
            .unwrap_err()
            .contains("KiB")
        );
        let shallow = "a:
  b:
    c: [1, [2, 3]]
";
        assert_eq!(parse_yaml_to_json(shallow).unwrap()["a"]["b"]["c"][1][0], 2);
    }
}
