//! A tiny JSON writer that reproduces Python's `json.dumps` output byte for byte.
//!
//! The 1.x CLI printed `json.dumps(obj, indent=2)` (and one compact
//! `json.dumps(obj)`), whose defaults differ from serde_json: `ensure_ascii`
//! escapes every non-ASCII character as `\uXXXX`, and the compact form puts a
//! space after `,` and `:`. The schemas are fixed and small, so a hand-written
//! encoder is simpler than bending a serializer.

/// A JSON value with object keys kept in insertion order.
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// Quote `s` the way `json.dumps(..., ensure_ascii=True)` does.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ' '..='~' => out.push(ch),
            _ => {
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}

/// `json.dumps(value, indent=2)`.
pub fn dumps_indent(value: &Value) -> String {
    let mut out = String::new();
    write_indent(value, 0, &mut out);
    out
}

/// `json.dumps(value)` (no indent, `", "` and `": "` separators).
pub fn dumps_compact(value: &Value) -> String {
    match value {
        Value::Str(s) => quote(s),
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(dumps_compact).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{}: {}", quote(k), dumps_compact(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

fn write_indent(value: &Value, level: usize, out: &mut String) {
    let pad = |n: usize| " ".repeat(n * 2);
    match value {
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Object(fields) if fields.is_empty() => out.push_str("{}"),
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&pad(level + 1));
                write_indent(item, level + 1, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(level));
            out.push(']');
        }
        Value::Object(fields) => {
            out.push_str("{\n");
            for (i, (key, item)) in fields.iter().enumerate() {
                out.push_str(&pad(level + 1));
                out.push_str(&quote(key));
                out.push_str(": ");
                write_indent(item, level + 1, out);
                if i + 1 < fields.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(level));
            out.push('}');
        }
        other => out.push_str(&dumps_compact(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(fields: Vec<(&str, Value)>) -> Value {
        Value::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    #[test]
    fn ensure_ascii_escapes_non_ascii_and_controls() {
        // Expected escapes are assembled from parts so no editor or tool can
        // decode them back into the characters they stand for.
        let u = |hex: &str| format!("{}u{hex}", '\\');
        assert_eq!(
            quote("\u{ae30}\u{b2a5}/\u{d55c}\u{ae00}"),
            format!("\"{}{}/{}{}\"", u("ae30"), u("b2a5"), u("d55c"), u("ae00"))
        );
        assert_eq!(
            quote("a\"b\\c\nd\u{7f}\u{1}"),
            format!("\"a\\\"b\\\\c\\nd{}{}\"", u("007f"), u("0001"))
        );
        assert_eq!(
            quote("\u{1f600}"),
            format!("\"{}{}\"", u("d83d"), u("de00"))
        );
    }

    #[test]
    fn compact_matches_python_separators() {
        let v = obj(vec![
            ("worktrees", Value::Array(vec![])),
            (
                "summary",
                obj(vec![("total", Value::Int(0)), ("ok", Value::Bool(true))]),
            ),
        ]);
        assert_eq!(
            dumps_compact(&v),
            r#"{"worktrees": [], "summary": {"total": 0, "ok": true}}"#
        );
    }

    #[test]
    fn indent_matches_python_layout() {
        let v = Value::Array(vec![obj(vec![
            ("branch", Value::Str("main".into())),
            ("detached", Value::Bool(false)),
        ])]);
        assert_eq!(
            dumps_indent(&v),
            "[\n  {\n    \"branch\": \"main\",\n    \"detached\": false\n  }\n]"
        );
        assert_eq!(dumps_indent(&Value::Array(vec![])), "[]");
    }
}
