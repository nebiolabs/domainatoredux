//! JSON as Python's `json.dump(..., separators=(",", ":"))` writes it: compact, and
//! with every non-ASCII character escaped (`ensure_ascii`, Python's default), so an
//! agent-facing document is byte-identical to upstream's, not only equal when parsed.

use serde_json::Value;

/// `value` as compact JSON with non-ASCII characters written as `\uXXXX` escapes
/// (UTF-16 surrogate pairs above the Basic Multilingual Plane, as Python does).
pub fn to_string(value: &Value) -> String {
    let compact = value.to_string();
    if compact.is_ascii() {
        return compact;
    }
    // Outside strings JSON is pure ASCII, so every non-ASCII character is inside a
    // string, where an escape means the same character.
    let mut out = String::with_capacity(compact.len() + 16);
    let mut units = [0u16; 2];
    for c in compact.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn non_ascii_is_escaped_like_python() {
        // json.dumps({"t": "∞ é 𝄞"}, separators=(",", ":"))
        let value = json!({"t": "∞ é 𝄞", "n": [1, 2.5]});
        assert_eq!(
            super::to_string(&value),
            r#"{"t":"\u221e \u00e9 \ud834\udd1e","n":[1,2.5]}"#
        );
    }
}
