//! Minimal, zero-dependency YAML subset parser converting YAML configuration documents into
//! `serde_json::Value` (`docs/app-structure-plan.md`).
//!
//! Avoids pulling in the unmaintained `serde_yaml` crate (which is rejected by `cargo deny`) while
//! supporting standard server configuration YAML files (nested maps, inline arrays/maps, scalars,
//! quoted strings, booleans, numbers, and `#` comments).

use serde_json::{Map, Number, Value};

pub fn parse_yaml_to_json(input: &str) -> Result<Value, String> {
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
    parse_block(&lines, &mut pos, lines[0].1)
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

fn parse_block(lines: &[(usize, usize, String)], pos: &mut usize, base_indent: usize) -> Result<Value, String> {
    if *pos >= lines.len() {
        return Ok(Value::Object(Map::new()));
    }
    if lines[*pos].2.starts_with("- ") || lines[*pos].2 == "-" {
        return parse_seq(lines, pos, base_indent);
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
                let child = parse_block(lines, pos, child_indent)?;
                map.insert(key, child);
            } else {
                map.insert(key, Value::Null);
            }
        } else {
            map.insert(key, parse_scalar_or_inline(val_trimmed)?);
        }
    }
    Ok(Value::Object(map))
}

fn parse_seq(lines: &[(usize, usize, String)], pos: &mut usize, base_indent: usize) -> Result<Value, String> {
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
                items.push(parse_block(lines, pos, child_indent)?);
            } else {
                items.push(Value::Null);
            }
        } else {
            items.push(parse_scalar_or_inline(rest)?);
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

fn parse_scalar_or_inline(s: &str) -> Result<Value, String> {
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
                arr.push(parse_scalar_or_inline(part.trim())?);
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
}
