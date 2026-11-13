//! Reading the game's metadata files as they actually are.
//!
//! Most `ui_car.json` files shipped with the game are not valid JSON: their
//! descriptions contain raw line breaks and tabs. Some files are Windows-1252
//! rather than UTF-8, and mods add trailing commas and numbers written as
//! strings. Everything here accepts those files instead of rejecting them.

use serde_json::Value;
use std::path::Path;

use crate::{Error, Result, io_err};

/// Decodes UTF-8 (with or without a byte-order mark), falling back to
/// Windows-1252 for files saved by older Windows tools.
pub fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    }
}

/// Repairs the two defects that make the game's JSON unreadable to a strict
/// parser: control characters inside strings, and commas before a closing
/// bracket.
pub fn repair_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            match c {
                '\\' => {
                    out.push(c);
                    if let Some(escaped) = chars.next() {
                        out.push(escaped);
                    }
                }
                '"' => {
                    in_string = false;
                    out.push(c);
                }
                '\n' => out.push_str("\\n"),
                '\r' => {}
                '\t' => out.push(' '),
                c if (c as u32) < 0x20 => {}
                c => out.push(c),
            }
        } else {
            match c {
                '"' => {
                    in_string = true;
                    out.push(c);
                }
                ',' => {
                    let mut ahead = chars.clone();
                    let next = ahead.find(|c| !c.is_whitespace());
                    if !matches!(next, Some('}' | ']')) {
                        out.push(c);
                    }
                }
                c => out.push(c),
            }
        }
    }
    out
}

pub fn parse_json(bytes: &[u8]) -> std::result::Result<Value, serde_json::Error> {
    let text = decode(bytes);
    serde_json::from_str(&text).or_else(|_| serde_json::from_str(&repair_json(&text)))
}

pub fn read_json(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).map_err(io_err(path))?;
    parse_json(&bytes).map_err(|e| Error::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// A field as text, whether the file wrote it as a string or a number.
/// Empty strings count as absent.
pub fn text(value: &Value, key: &str) -> Option<String> {
    let text = match value.get(key)? {
        Value::String(s) => clean_text(s),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

pub fn text_list(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(clean_text)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The first number in a field: `"470bhp"`, `"7.004 km"`, `"270+km/h"` and
/// `195` all yield their leading value. A decimal comma is read as a point.
pub fn number(value: &Value, key: &str) -> Option<f64> {
    match value.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => leading_number(s),
        _ => None,
    }
}

pub fn leading_number(text: &str) -> Option<f64> {
    let text = text
        .trim()
        .trim_start_matches(['~', '<', '>', '≈', '+', ' ']);
    if !text.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(text.len());
    text[..end]
        .trim_end_matches(['.', ','])
        .replace(',', ".")
        .parse()
        .ok()
}

/// Descriptions use `</br>` and `<br>` for paragraphs; everything else is
/// plain text.
pub fn clean_text(text: &str) -> String {
    let mut out = text
        .replace("</br>", "\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n");
    out = out.lines().map(str::trim).collect::<Vec<_>>().join("\n");
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_line_breaks_inside_strings_are_accepted() {
        let json = "{\n\t\"name\": \"Ferrari 458 GT2\",\n\t\"description\": \"First line.</br></br>\n\t\n\tSecond line.\",\n}";
        let value = parse_json(json.as_bytes()).unwrap();
        assert_eq!(text(&value, "name").unwrap(), "Ferrari 458 GT2");
        assert_eq!(
            text(&value, "description").unwrap(),
            "First line.\n\nSecond line."
        );
    }

    #[test]
    fn windows_1252_is_decoded() {
        let bytes = b"{\"description\": \"7\xb0 banking\"}";
        let value = parse_json(bytes).unwrap();
        assert_eq!(text(&value, "description").unwrap(), "7° banking");
    }

    #[test]
    fn escaped_quotes_do_not_end_the_string() {
        let json = r#"{"a": "say \"hi\",}", "b": [1, 2,],}"#;
        let value = parse_json(json.as_bytes()).unwrap();
        assert_eq!(text(&value, "a").unwrap(), "say \"hi\",}");
        assert_eq!(value["b"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn numbers_are_read_from_strings_with_units() {
        assert_eq!(leading_number("470bhp"), Some(470.0));
        assert_eq!(leading_number("270+km/h"), Some(270.0));
        assert_eq!(leading_number("7.004 km"), Some(7.004));
        assert_eq!(leading_number("5,793"), Some(5.793));
        assert_eq!(leading_number("--s 0-100"), None);
        assert_eq!(leading_number("~300 km/h"), Some(300.0));
        assert_eq!(leading_number("n/a"), None);
    }
}
