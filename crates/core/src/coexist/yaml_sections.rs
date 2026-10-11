//! Edit selected top-level YAML fields while retaining unrelated source text.
use crate::error::AppError;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, ops::Range};
fn invalid() -> AppError {
    AppError::Config(
        "YAML 配置无法安全解析或编辑 / YAML configuration cannot be safely edited".into(),
    )
}
fn section_key(line: &str) -> Option<String> {
    if line.starts_with(char::is_whitespace) || line.starts_with(['#', '-', '?', '%']) {
        return None;
    }
    // Let the YAML parser interpret quoted keys, including keys containing ':'.
    for (position, _) in line.match_indices(':') {
        let tail = &line[position + 1..];
        if !tail.is_empty() && !tail.starts_with(char::is_whitespace) {
            continue;
        }
        let header = format!("{}: null", &line[..position]);
        if let Ok(Value::Object(object)) = serde_yaml::from_str::<Value>(&header) {
            if object.len() == 1 {
                return object.keys().next().cloned();
            }
        }
    }
    None
}
pub(crate) struct Document<'a> {
    source: &'a str,
    pub value: Value,
    sections: Vec<(String, Range<usize>)>,
}
impl<'a> Document<'a> {
    pub fn parse(source: &'a str) -> Result<Self, AppError> {
        let mut starts = Vec::new();
        let mut offset = 0;
        for line in source.split_inclusive('\n') {
            if let Some(key) = section_key(line.trim_end_matches(['\r', '\n'])) {
                starts.push((key, offset));
            }
            offset += line.len();
        }
        let mut sections = Vec::new();
        for (index, (key, start)) in starts.iter().enumerate() {
            let end = starts
                .get(index + 1)
                .map_or(source.len(), |(_, start)| *start);
            let mut content_end = end;
            for line in source[*start..end].split_inclusive('\n').rev() {
                if line.trim().is_empty() || line.starts_with('#') {
                    content_end -= line.len();
                } else {
                    break;
                }
            }
            sections.push((key.clone(), *start..content_end));
        }
        // Parse effective last definitions explicitly; only field writes remove
        // duplicate source sections. This accommodates older Python-written YAML.
        let mut last = BTreeMap::new();
        for (index, (key, _)) in sections.iter().enumerate() {
            last.insert(key, index);
        }
        let mut effective = source.to_string();
        for (index, (key, span)) in sections.iter().enumerate().rev() {
            if last.get(key) != Some(&index) {
                effective.replace_range(span.clone(), "");
            }
        }
        let value: Value = if effective.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_yaml::from_str(&effective).map_err(|_| invalid())?
        };
        let value = if value.is_null() {
            Value::Object(Map::new())
        } else {
            value
        };
        if !value.is_object() {
            return Err(invalid());
        }
        Ok(Self {
            source,
            value,
            sections,
        })
    }
    pub fn replace(&self, changes: &[(&str, Value)]) -> Result<String, AppError> {
        let mut desired = self.value.clone();
        let mut edits = Vec::new();
        let mut added = String::new();
        for (key, value) in changes {
            let spans: Vec<_> = self
                .sections
                .iter()
                .filter(|(name, _)| name == key)
                .map(|(_, span)| span)
                .collect();
            if desired.get(*key) == Some(value) && spans.len() <= 1 {
                continue;
            }
            let mut fragment = Map::new();
            fragment.insert((*key).into(), value.clone());
            let mut encoded =
                serde_yaml::to_string(&Value::Object(fragment)).map_err(|_| invalid())?;
            if self.source.contains("\r\n") {
                encoded = encoded.replace('\n', "\r\n");
            }
            if spans.is_empty() {
                if desired.get(*key).is_some() {
                    return Err(invalid());
                }
                added.push_str(&encoded);
            } else {
                for (index, span) in spans.iter().enumerate() {
                    edits.push((
                        (*span).clone(),
                        if index + 1 == spans.len() {
                            encoded.clone()
                        } else {
                            String::new()
                        },
                    ));
                }
            }
            desired[*key] = value.clone();
        }
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        let mut result = self.source.to_string();
        for (range, text) in edits {
            result.replace_range(range, &text);
        }
        if !added.is_empty() {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&added);
        }
        if Document::parse(&result)?.value != desired {
            return Err(invalid());
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_fields_and_unrelated_comments_survive() {
        let input = "# user\r\n\"models\": [old]\r\n# keep\r\nnotes: |\r\n  a: b\r\n";
        let doc = Document::parse(input).unwrap();
        let output = doc
            .replace(&[("models", serde_json::json!(["next"]))])
            .unwrap();
        assert!(output.contains("# keep\r\nnotes: |\r\n  a: b\r\n"));
    }
}
