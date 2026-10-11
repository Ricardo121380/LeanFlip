//! TOML operations built on the public `toml_edit` document API.
use super::{decode_utf8, line_column, KeyPath, LivePatch, LiveWriteError};
use std::path::Path;
use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

#[derive(Clone)]
pub struct TomlClearScope {
    pub parent: KeyPath,
    pub is_floor: fn(&str) -> bool,
}
#[derive(Clone, Default)]
pub struct TomlPatch {
    pub clear: Vec<TomlClearScope>,
    pub remove: Vec<KeyPath>,
    pub set: Vec<(KeyPath, Item)>,
    pub seed: Vec<(KeyPath, Item)>,
    pub remove_if: Vec<(KeyPath, Vec<Value>)>,
}
pub fn shape_error(path: &Path, keys: &[String]) -> LiveWriteError {
    LiveWriteError::Shape {
        path: path.into(),
        key_path: KeyPath(keys.into()),
        expected: "表",
    }
}
fn table<'a>(
    path: &Path,
    mut current: &'a mut dyn TableLike,
    keys: &[String],
    create: bool,
) -> Result<Option<&'a mut dyn TableLike>, LiveWriteError> {
    for (depth, key) in keys.iter().enumerate() {
        if !current.contains_key(key) {
            if !create {
                return Ok(None);
            }
            let mut child = Table::new();
            child.set_implicit(true);
            current.insert(key, Item::Table(child));
        }
        current = current
            .get_mut(key)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| shape_error(path, &keys[..=depth]))?;
    }
    Ok(Some(current))
}
fn install(container: &mut dyn TableLike, key: &str, value: &Item) {
    let mut next = value.clone();
    if let Some(previous) = container.get(key) {
        match (previous, &mut next) {
            (Item::Value(a), Item::Value(b)) => *b.decor_mut() = a.decor().clone(),
            (Item::Table(a), Item::Table(b)) => {
                *b.decor_mut() = a.decor().clone();
                if let Some(index) = a.position() {
                    b.set_position(index);
                }
            }
            _ => {}
        }
    }
    if let Some(slot) = container.get_mut(key) {
        *slot = next;
    } else {
        container.insert(key, next);
    }
}
impl TomlPatch {
    pub fn apply_to(&self, path: &Path, document: &mut DocumentMut) -> Result<(), LiveWriteError> {
        let mut deletes = self.remove.clone();
        for scope in &self.clear {
            if let Some(parent) = table(path, document.as_table_mut(), &scope.parent.0, false)? {
                deletes.extend(
                    parent
                        .iter()
                        .filter(|(key, _)| (scope.is_floor)(key))
                        .map(|(key, _)| scope.parent.child(key)),
                );
            }
        }
        for (key, candidates) in &self.remove_if {
            let (parents, leaf) = key.split_last().ok_or_else(|| shape_error(path, &[]))?;
            if let Some(parent) = table(path, document.as_table_mut(), parents, false)? {
                if parent
                    .get(leaf)
                    .and_then(Item::as_value)
                    .is_some_and(|current| {
                        candidates.iter().any(|other| same_value(current, other))
                    })
                {
                    deletes.push(key.clone());
                }
            }
        }
        for key in deletes
            .iter()
            .filter(|key| !self.set.iter().any(|(target, _)| target == *key))
        {
            let (parents, leaf) = key.split_last().ok_or_else(|| shape_error(path, &[]))?;
            if let Some(parent) = table(path, document.as_table_mut(), parents, false)? {
                parent.remove(leaf);
            }
        }
        for (seed, values) in [(false, &self.set), (true, &self.seed)] {
            for (key, value) in values {
                let (parents, leaf) = key.split_last().ok_or_else(|| shape_error(path, &[]))?;
                let parent = table(path, document.as_table_mut(), parents, true)?
                    .expect("requested container creation");
                if !seed || !parent.contains_key(leaf) {
                    install(parent, leaf, value);
                }
            }
        }
        Ok(())
    }
}
pub trait TomlDocPatch {
    fn apply_to(&self, path: &Path, document: &mut DocumentMut) -> Result<(), LiveWriteError>;
}
impl<T: TomlDocPatch> LivePatch for T {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let mut document = parse(path, before)?;
        self.apply_to(path, &mut document)?;
        Ok(document.to_string().into_bytes())
    }
}
impl TomlDocPatch for TomlPatch {
    fn apply_to(&self, path: &Path, document: &mut DocumentMut) -> Result<(), LiveWriteError> {
        self.apply_to(path, document)
    }
}
pub struct TomlSteps<'a>(pub Vec<&'a dyn TomlDocPatch>);
impl TomlDocPatch for TomlSteps<'_> {
    fn apply_to(&self, path: &Path, document: &mut DocumentMut) -> Result<(), LiveWriteError> {
        for patch in &self.0 {
            patch.apply_to(path, document)?;
        }
        Ok(())
    }
}
pub fn parse(path: &Path, before: Option<&[u8]>) -> Result<DocumentMut, LiveWriteError> {
    let source = decode_utf8(path, before.unwrap_or_default())?;
    source.parse::<DocumentMut>().map_err(|error| {
        let (line, column) = line_column(source, error.span().map_or(0, |span| span.start));
        LiveWriteError::Parse {
            path: path.into(),
            line,
            column,
            message: "Invalid TOML".into(),
        }
    })
}
pub fn value_text(value: &Value) -> String {
    let mut undecorated = value.clone();
    undecorated.decor_mut().clear();
    undecorated.to_string()
}
pub fn same_value(a: &Value, b: &Value) -> bool {
    let semantic = |value: &Value| {
        format!("field = {}", value_text(value))
            .parse::<toml::Table>()
            .ok()
            .and_then(|mut table| table.remove("field"))
    };
    match (semantic(a), semantic(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_containers_and_unrelated_comments_are_preserved() {
        let input = "# personal configuration\nroutes = { active = { token = 'before', timeout = 9 } }\n[ui]\ncolor = 'warm' # personal\n";
        let patch = TomlPatch {
            set: vec![(
                KeyPath::new(&["routes", "active", "token"]),
                toml_edit::value("after"),
            )],
            ..Default::default()
        };
        let output = String::from_utf8(
            patch
                .apply(Path::new("fixture"), Some(input.as_bytes()))
                .unwrap(),
        )
        .unwrap();
        assert!(output.contains("color = 'warm' # personal"));
        let parsed: toml::Table = output.parse().unwrap();
        assert_eq!(parsed["routes"]["active"]["timeout"].as_integer(), Some(9));
        assert_eq!(parsed["routes"]["active"]["token"].as_str(), Some("after"));
    }
    #[test]
    fn comparisons_use_values_instead_of_string_quote_styles() {
        let a: DocumentMut = "x = 'value'".parse().unwrap();
        let b: DocumentMut = "x = \"value\" # different formatting".parse().unwrap();
        assert!(same_value(
            a["x"].as_value().unwrap(),
            b["x"].as_value().unwrap()
        ));
        assert!(same_value(
            &Value::from(16),
            "0x10".parse::<Value>().as_ref().unwrap()
        ));
    }
}
