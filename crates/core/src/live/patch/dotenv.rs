//! Single-line dotenv assignments. Opaque lines and unrelated variables retain
//! their original bytes; replacement values cannot inject another assignment.
use super::{decode_utf8, LivePatch, LiveWriteError};
use std::{collections::HashSet, path::Path};
#[derive(Clone, Default)]
pub struct DotenvPatch {
    pub clear: Option<fn(&str) -> bool>,
    pub set: Vec<(String, String)>,
    pub remove_if: Vec<(String, Vec<String>)>,
    pub remove: Vec<String>,
}
fn identifier(key: &str) -> bool {
    !key.is_empty()
        && key.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic() || byte == b'_' || (index != 0 && byte.is_ascii_digit())
        })
}
fn assignment(line: &str) -> Option<(&str, &str)> {
    let source = line.trim().strip_prefix("export ").unwrap_or(line.trim());
    let (key, value) = source.split_once('=')?;
    identifier(key.trim()).then_some((key.trim(), value.trim()))
}
fn quoted(raw: &str) -> Option<(char, &str)> {
    let quote = raw
        .chars()
        .next()
        .filter(|c| ['\'', '"', '`'].contains(c))?;
    for (index, ch) in raw.char_indices().skip(1) {
        if ch == quote {
            let tail = raw[index + ch.len_utf8()..].trim();
            if tail.is_empty() || tail.starts_with('#') {
                return Some((quote, &raw[quote.len_utf8()..index]));
            }
        }
    }
    None
}
fn decoded(raw: &str) -> String {
    if let Some((quote, inner)) = quoted(raw.trim()) {
        if quote == '"' {
            inner.replace("\\n", "\n").replace("\\r", "\r")
        } else {
            inner.to_owned()
        }
    } else {
        raw.split('#').next().unwrap_or_default().trim().into()
    }
}
fn encoded(value: &str) -> Option<String> {
    if !value.chars().any(char::is_whitespace) && !value.contains(['#', '\'', '"', '`']) {
        return Some(value.into());
    }
    // dotenv preserves backslashes in single/backtick quotes; JSON escaping
    // would change the actual credential. Choose a delimiter absent in data.
    for quote in ['\'', '`', '"'] {
        if !value.contains(quote)
            && !(quote == '"' && (value.contains("\\n") || value.contains("\\r")))
        {
            return Some(format!("{quote}{value}{quote}"));
        }
    }
    None
}
impl LivePatch for DotenvPatch {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        for (key, value) in &self.set {
            if !identifier(key)
                || value.chars().any(|ch| ch.is_control() && ch != '\t')
                || encoded(value).is_none()
            {
                return Err(LiveWriteError::Rejected(Box::new(
                    crate::AppError::InvalidInput(
                        "Dotenv entries require valid names and single-line values".into(),
                    ),
                )));
            }
        }
        let text = decode_utf8(path, before.unwrap_or_default())?;
        let separator = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let mut written = HashSet::new();
        let mut output = String::new();
        for line in text.split_inclusive('\n') {
            let Some((key, raw)) = assignment(line) else {
                output.push_str(line);
                continue;
            };
            if raw.starts_with(['\'', '"', '`']) && quoted(raw).is_none() {
                return Err(LiveWriteError::Rejected(Box::new(crate::AppError::InvalidInput(
                    "Unterminated or multiline dotenv value; edit this entry in the client first".into(),
                ))));
            }
            if let Some((_, value)) = self.set.iter().rev().find(|(target, _)| target == key) {
                if !written.insert(key.to_string()) {
                    continue;
                }
                if decoded(raw) == *value {
                    output.push_str(line);
                } else {
                    let prefix = line.split_once('=').expect("parsed assignment").0;
                    output.push_str(prefix);
                    output.push('=');
                    output.push_str(&encoded(value).expect("validated dotenv value"));
                    if line.ends_with("\r\n") {
                        output.push_str("\r\n");
                    } else if line.ends_with('\n') {
                        output.push('\n');
                    }
                }
                continue;
            }
            let discard = self.clear.is_some_and(|matches| matches(key))
                || self.remove.iter().any(|target| target == key)
                || self.remove_if.iter().any(|(target, candidates)| {
                    target == key
                        && candidates
                            .iter()
                            .any(|candidate| decoded(candidate) == decoded(raw))
                });
            if !discard {
                output.push_str(line);
            }
        }
        for (key, _) in &self.set {
            if written.insert(key.to_string()) {
                let value = &self
                    .set
                    .iter()
                    .rev()
                    .find(|(target, _)| target == key)
                    .expect("existing target")
                    .1;
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push_str(separator);
                }
                output.push_str(key);
                output.push('=');
                output.push_str(&encoded(value).expect("validated dotenv value"));
                if text.is_empty() || text.ends_with('\n') {
                    output.push_str(separator);
                }
            }
        }
        Ok(output.into_bytes())
    }
}
/// Parsed values, in first-occurrence order. A later assignment replaces its value.
pub fn entries(text: &str) -> Vec<(String, String)> {
    let mut values = indexmap::IndexMap::new();
    for (key, raw) in text.lines().filter_map(assignment) {
        values.insert(key.to_owned(), decoded(raw));
    }
    values.into_iter().collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_backslashes_are_not_json_escaped_and_multiline_edits_are_refused() {
        let expected = r"prefix\name # fragment";
        let patch = DotenvPatch {
            set: vec![("TOKEN".into(), expected.into())],
            ..Default::default()
        };
        let bytes = patch.apply(Path::new("fixture.env"), None).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text, "TOKEN='prefix\\name # fragment'\n");
        assert_eq!(entries(&text), vec![("TOKEN".into(), expected.into())]);
        assert!(patch
            .apply(Path::new("fixture.env"), Some(b"OTHER='first\nsecond'\n"))
            .is_err());
    }
    #[test]
    fn dotenv_round_trip_preserves_comments_and_encodes_literal_hashes() {
        let source = "# local preferences\r\nUSER_MODE = quiet\r\nexport API_TOKEN=old\r\nAPI_TOKEN=duplicate\r\n";
        let patch = DotenvPatch {
            set: vec![("API_TOKEN".into(), "new # literal".into())],
            ..Default::default()
        };
        let changed = patch
            .apply(Path::new("fixture"), Some(source.as_bytes()))
            .unwrap();
        let text = String::from_utf8(changed).unwrap();
        assert!(text.contains("USER_MODE = quiet\r\n"));
        assert_eq!(text.matches("API_TOKEN=").count(), 1);
        assert_eq!(
            entries(&text)[1],
            ("API_TOKEN".into(), "new # literal".into())
        );
        assert_eq!(
            patch
                .apply(Path::new("fixture"), Some(text.as_bytes()))
                .unwrap(),
            text.as_bytes()
        );
    }
    #[test]
    fn a_value_cannot_inject_a_second_environment_variable() {
        let patch = DotenvPatch {
            set: vec![("TOKEN".into(), "value\nOVERRIDE=yes".into())],
            ..Default::default()
        };
        assert!(patch
            .apply(Path::new("fixture"), Some(b"USER=keep\n"))
            .is_err());
        assert_eq!(
            entries("KEY='a#b'\nKEY=final # note\n"),
            vec![("KEY".into(), "final".into())]
        );
    }
}
