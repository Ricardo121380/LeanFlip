//! Ordered JSON edits with validation before serialization.
use super::{decode_utf8, KeyPath, LivePatch, LiveWriteError};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::Path;

#[derive(Clone)]
pub struct ClearScope {
    pub parent: KeyPath,
    pub is_floor: fn(&str) -> bool,
}
#[derive(Clone, Default)]
pub struct JsonPatch {
    pub clear: Vec<ClearScope>,
    pub remove: Vec<KeyPath>,
    pub set: Vec<(KeyPath, Value)>,
    pub seed: Vec<(KeyPath, Value)>,
    pub remove_if: Vec<(KeyPath, Vec<Value>)>,
}
fn wrong(path: &Path, keys: &[String]) -> LiveWriteError {
    LiveWriteError::Shape {
        path: path.into(),
        key_path: KeyPath(keys.into()),
        expected: "对象",
    }
}
fn object<'a>(
    path: &Path,
    doc: &'a mut Value,
    keys: &[String],
    create: bool,
) -> Result<Option<&'a mut Map<String, Value>>, LiveWriteError> {
    let mut node = doc;
    for (index, key) in keys.iter().enumerate() {
        let map = node
            .as_object_mut()
            .ok_or_else(|| wrong(path, &keys[..index]))?;
        if !map.contains_key(key) {
            if !create {
                return Ok(None);
            }
            map.insert(key.clone(), Value::Object(Map::new()));
        }
        node = map.get_mut(key).expect("key exists or was inserted");
    }
    node.as_object_mut()
        .map(Some)
        .ok_or_else(|| wrong(path, keys))
}
pub fn value_at<'a>(doc: &'a Value, path: &KeyPath) -> Option<&'a Value> {
    let mut node = doc;
    for key in &path.0 {
        node = node.get(key)?;
    }
    Some(node)
}
impl JsonPatch {
    pub fn apply_to(&self, path: &Path, doc: &mut Value) -> Result<(), LiveWriteError> {
        let replaces = |key: &KeyPath| self.set.iter().any(|(target, _)| target == key);
        let mut deletes = self.remove.clone();
        for scope in &self.clear {
            if let Some(map) = object(path, doc, &scope.parent.0, false)? {
                deletes.extend(
                    map.keys()
                        .filter(|key| (scope.is_floor)(key))
                        .map(|key| scope.parent.child(key)),
                );
            }
        }
        deletes.extend(
            self.remove_if
                .iter()
                .filter(|(key, allowed)| {
                    value_at(doc, key).is_some_and(|value| allowed.contains(value))
                })
                .map(|(key, _)| key.clone()),
        );
        for key in deletes.iter().filter(|key| !replaces(key)) {
            let (parents, leaf) = key.split_last().ok_or_else(|| wrong(path, &[]))?;
            if let Some(map) = object(path, doc, parents, false)? {
                map.shift_remove(leaf);
            }
        }
        for (only_missing, fields) in [(false, &self.set), (true, &self.seed)] {
            for (key, value) in fields {
                let (parents, leaf) = key.split_last().ok_or_else(|| wrong(path, &[]))?;
                let map = object(path, doc, parents, true)?.expect("created object");
                if !only_missing || !map.contains_key(leaf) {
                    map.insert(leaf.clone(), value.clone());
                }
            }
        }
        Ok(())
    }
}
impl LivePatch for JsonPatch {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let (mut document, style) = parse(path, before)?;
        self.apply_to(path, &mut document)?;
        serialize(path, &document, &style)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonStyle {
    indentation: String,
    newline: &'static str,
    final_newline: bool,
    byte_order_mark: bool,
}
impl Default for JsonStyle {
    fn default() -> Self {
        Self {
            indentation: "  ".into(),
            newline: "\n",
            final_newline: false,
            byte_order_mark: false,
        }
    }
}
pub fn parse(path: &Path, before: Option<&[u8]>) -> Result<(Value, JsonStyle), LiveWriteError> {
    let text = decode_utf8(path, before.unwrap_or_default())?;
    let mut style = JsonStyle {
        byte_order_mark: text.starts_with('\u{feff}'),
        final_newline: text.ends_with('\n'),
        newline: if text.contains("\r\n") { "\r\n" } else { "\n" },
        ..JsonStyle::default()
    };
    if let Some(indent) = text
        .lines()
        .filter_map(|line| {
            let count = line.len() - line.trim_start_matches([' ', '\t']).len();
            (count > 0 && !line.trim().is_empty()).then_some(&line[..count])
        })
        .next()
    {
        style.indentation = indent.into();
    }
    let source = text.trim_start_matches('\u{feff}');
    let value = if source.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str::<Value>(source).map_err(|error| LiveWriteError::Parse {
            path: path.into(),
            line: error.line(),
            column: error.column(),
            message: "Invalid JSON".into(),
        })?
    };
    if !value.is_object() {
        return Err(wrong(path, &[]));
    }
    Ok((value, style))
}
pub fn serialize(path: &Path, value: &Value, style: &JsonStyle) -> Result<Vec<u8>, LiveWriteError> {
    let mut bytes = Vec::new();
    value
        .serialize(&mut serde_json::Serializer::with_formatter(
            &mut bytes,
            serde_json::ser::PrettyFormatter::with_indent(style.indentation.as_bytes()),
        ))
        .map_err(|error| LiveWriteError::Io {
            path: path.into(),
            source: std::io::Error::other(error),
        })?;
    let body = String::from_utf8(bytes)
        .expect("JSON serialization produces UTF-8")
        .replace('\n', style.newline);
    Ok(format!(
        "{}{}{}",
        if style.byte_order_mark {
            "\u{feff}"
        } else {
            ""
        },
        body,
        if style.final_newline {
            style.newline
        } else {
            ""
        }
    )
    .into_bytes())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn precise_changes_preserve_user_values_and_file_style() {
        let source =
            "\u{feff}{\r\n\t\"env\": {\"API\": \"old\", \"USER\": 7},\r\n\t\"hooks\": [1]\r\n}\r\n";
        let patch = JsonPatch {
            clear: vec![ClearScope {
                parent: KeyPath::new(&["env"]),
                is_floor: |key| key == "API",
            }],
            set: vec![(KeyPath::new(&["env", "API"]), json!("new"))],
            ..Default::default()
        };
        let bytes = patch
            .apply(Path::new("fixture"), Some(source.as_bytes()))
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with('\u{feff}') && text.ends_with("\r\n"));
        let document: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap();
        assert_eq!(document, json!({"env":{"API":"new","USER":7},"hooks":[1]}));
        assert_eq!(
            patch
                .apply(Path::new("fixture"), Some(text.as_bytes()))
                .unwrap(),
            text.as_bytes()
        );
    }
    #[test]
    fn malformed_documents_and_non_object_parents_are_rejected() {
        let patch = JsonPatch {
            set: vec![(KeyPath::new(&["env", "API"]), json!("new"))],
            ..Default::default()
        };
        for input in ["{", "[]", "{\"env\":false}"] {
            assert!(patch
                .apply(Path::new("fixture"), Some(input.as_bytes()))
                .is_err());
        }
    }
}
