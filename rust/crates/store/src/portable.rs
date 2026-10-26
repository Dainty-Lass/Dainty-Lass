//! Raw SQL both engines run the same way, for what `MultiConnection`'s DSL can't express (DF7 spike):
//! upserts (`on_conflict` doesn't compile through it), insert-or-ignore, batch inserts.
//!
//! Placeholders: `$1..$n`, numbered in order of first appearance. `?` is a syntax error on Postgres, and
//! SQLite takes `$N` as a named parameter numbered by first appearance, so `SELECT $2, $1` would swap the
//! values on SQLite only. [`numbered`] rewrites a `?` query, [`check`] catches the rest. Identifiers are
//! quoted: Postgres folds unquoted ones to lower case, so a camelCase column needs it.

/// `"name"`, with any `"` doubled.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `'text'`, with any `'` doubled.
pub fn quote_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

enum Piece<'a> {
    /// SQL outside strings, identifiers and comments.
    Code(&'a str),
    Other(&'a str),
}

/// Splits SQL into code and the quoted strings, quoted identifiers and comments between it.
fn pieces(sql: &str) -> Vec<Piece<'_>> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        let end = match (bytes[i], bytes.get(i + 1)) {
            (q @ (b'\'' | b'"'), _) => {
                let mut j = i + 1;
                loop {
                    match bytes.get(j) {
                        None => break bytes.len(),
                        Some(&c) if c == q && bytes.get(j + 1) == Some(&q) => j += 2,
                        Some(&c) if c == q => break j + 1,
                        Some(_) => j += 1,
                    }
                }
            }
            (b'-', Some(b'-')) => sql[i..].find('\n').map_or(bytes.len(), |n| i + n),
            (b'/', Some(b'*')) => sql[i + 2..].find("*/").map_or(bytes.len(), |n| i + 2 + n + 2),
            _ => {
                i += 1;
                continue;
            }
        };
        if start < i {
            out.push(Piece::Code(&sql[start..i]));
        }
        out.push(Piece::Other(&sql[i..end]));
        start = end;
        i = end;
    }
    if start < bytes.len() {
        out.push(Piece::Code(&sql[start..]));
    }
    out
}

/// A query written with `?` placeholders (the metagame's 52, RV3), with `$1..$n` in their place.
pub fn numbered(sql: &str) -> Result<String, String> {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0;
    for piece in pieces(sql) {
        match piece {
            Piece::Other(text) => out.push_str(text),
            Piece::Code(code) => {
                if code.contains('$') {
                    return Err(format!("the query mixes ? and $n placeholders: {sql}"));
                }
                for c in code.chars() {
                    if c == '?' {
                        n += 1;
                        out.push_str(&format!("${n}"));
                    } else {
                        out.push(c);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Refuses SQL whose placeholders would bind differently on the two engines: a `?`, or `$n` that
/// first appear out of order or skip a number.
pub fn check(sql: &str) -> Result<(), String> {
    let mut highest = 0u32;
    for piece in pieces(sql) {
        let Piece::Code(code) = piece else { continue };
        if code.contains('?') {
            return Err(format!("? is a syntax error on Postgres: use $1..$n ({sql})"));
        }
        let mut rest = code;
        while let Some(at) = rest.find('$') {
            let digits: String = rest[at + 1..].chars().take_while(char::is_ascii_digit).collect();
            rest = &rest[at + 1 + digits.len()..];
            let Ok(number) = digits.parse::<u32>() else { continue };
            if number > highest + 1 {
                return Err(format!(
                    "${number} first appears before ${}: SQLite numbers them by first appearance ({sql})",
                    highest + 1
                ));
            }
            highest = highest.max(number);
        }
    }
    Ok(())
}

fn idents(names: &[&str]) -> String {
    names.iter().map(|n| quote_ident(n)).collect::<Vec<_>>().join(", ")
}

fn placeholders(from: usize, count: usize) -> String {
    (from..from + count)
        .map(|n| format!("${n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `INSERT .. ON CONFLICT (conflict) DO UPDATE SET col = excluded.col`, one row, binds in `columns`'
/// order. With nothing to update, the conflict is ignored.
pub fn upsert(table: &str, columns: &[&str], conflict: &[&str], update: &[&str]) -> String {
    let insert = format!(
        "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({})",
        quote_ident(table),
        idents(columns),
        placeholders(1, columns.len()),
        idents(conflict)
    );
    if update.is_empty() {
        return insert + " DO NOTHING";
    }
    let set: Vec<String> = update
        .iter()
        .map(|c| format!("{0} = excluded.{0}", quote_ident(c)))
        .collect();
    format!("{insert} DO UPDATE SET {}", set.join(", "))
}

/// `INSERT .. ON CONFLICT DO NOTHING`: SQLite's `INSERT OR IGNORE` on both, for any unique constraint.
pub fn insert_ignore(table: &str, columns: &[&str]) -> String {
    format!(
        "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT DO NOTHING",
        quote_ident(table),
        idents(columns),
        placeholders(1, columns.len())
    )
}

/// `INSERT .. VALUES (..), (..)` for `rows` rows, binds row by row. Diesel's batch insert doesn't compile
/// through `MultiConnection`. Keep `rows * columns` under SQLite's 32766 parameters.
pub fn insert_rows(table: &str, columns: &[&str], rows: usize) -> String {
    let values: Vec<String> = (0..rows)
        .map(|r| format!("({})", placeholders(1 + r * columns.len(), columns.len())))
        .collect();
    format!(
        "INSERT INTO {} ({}) VALUES {}",
        quote_ident(table),
        idents(columns),
        values.join(", ")
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn question_marks_become_numbers_outside_strings_and_comments() {
        assert_eq!(
            numbered("SELECT '?', \"a?\" FROM t WHERE x = ? AND y = ? -- why?\n AND z = ? /* ? */").unwrap(),
            "SELECT '?', \"a?\" FROM t WHERE x = $1 AND y = $2 -- why?\n AND z = $3 /* ? */"
        );
        assert_eq!(numbered("SELECT 'it''s ?' , ?").unwrap(), "SELECT 'it''s ?' , $1");
        assert!(numbered("SELECT ?, $1").is_err());
        check(&numbered("UPDATE t SET a = ?, b = ? WHERE id = ?").unwrap()).unwrap();
    }

    #[test]
    fn placeholders_must_appear_in_order() {
        check("SELECT $1, $2, $1, $3").unwrap();
        check("SELECT '$9', \"$5\" FROM t WHERE a = $1").unwrap();
        assert!(check("SELECT $2, $1").is_err());
        assert!(check("SELECT $1, $3").is_err());
        assert!(check("SELECT ?").is_err());
        check("SELECT 1 -- $4 ?\n").unwrap();
    }

    #[test]
    fn upserts_and_batches() {
        let sql = upsert("player", &["id", "displayName"], &["id"], &["displayName"]);
        assert_eq!(
            sql,
            "INSERT INTO \"player\" (\"id\", \"displayName\") VALUES ($1, $2) ON CONFLICT (\"id\") \
             DO UPDATE SET \"displayName\" = excluded.\"displayName\""
        );
        check(&sql).unwrap();
        assert!(upsert("t", &["a"], &["a"], &[]).ends_with("ON CONFLICT (\"a\") DO NOTHING"));
        assert_eq!(
            insert_rows("t", &["a", "b"], 2),
            "INSERT INTO \"t\" (\"a\", \"b\") VALUES ($1, $2), ($3, $4)"
        );
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(quote_literal("it's"), "'it''s'");
    }
}
