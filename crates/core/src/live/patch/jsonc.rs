//! JSON5 edits using source ranges instead of reconstructing a syntax tree.
//! `json5` validates semantics; a small scanner locates values and separators.
//! Unchanged ranges are copied byte-for-byte, including comments and escapes.
use crate::AppError;
use serde_json::Value;
use std::{collections::HashSet, ops::Range};

fn invalid() -> AppError {
    AppError::Config("JSONC editing requires a valid, unambiguous object".into())
}
fn read_value(source: &str) -> Result<Value, AppError> {
    json5::from_str(source).map_err(|_| invalid())
}
#[derive(Debug)]
struct Node {
    span: Range<usize>,
    kind: Kind,
}
#[derive(Debug)]
enum Kind {
    Object(Vec<Member>),
    Array(Vec<Node>),
    Scalar,
}
#[derive(Debug)]
struct Member {
    name: String,
    start: usize,
    value: Node,
    comma: Option<usize>,
}
struct Scanner<'a> {
    text: &'a str,
    offset: usize,
}
impl Scanner<'_> {
    fn peek(&self) -> Option<char> {
        self.text[self.offset..].chars().next()
    }
    fn advance(&mut self) {
        if let Some(ch) = self.peek() {
            self.offset += ch.len_utf8();
        }
    }
    fn trivia(&mut self) -> Result<(), AppError> {
        loop {
            if self
                .peek()
                .is_some_and(|ch| ch.is_whitespace() || ch == '\u{feff}')
            {
                self.advance();
                continue;
            }
            if self.text[self.offset..].starts_with("//") {
                while self
                    .peek()
                    .is_some_and(|ch| !matches!(ch, '\r' | '\n' | '\u{2028}' | '\u{2029}'))
                {
                    self.advance();
                }
                continue;
            }
            if self.text[self.offset..].starts_with("/*") {
                let length = self.text[self.offset + 2..]
                    .find("*/")
                    .ok_or_else(invalid)?;
                self.offset += length + 4;
                continue;
            }
            return Ok(());
        }
    }
    fn quoted(&mut self) -> Result<(), AppError> {
        let quote = self.peek().ok_or_else(invalid)?;
        self.advance();
        while let Some(ch) = self.peek() {
            self.advance();
            if ch == quote {
                return Ok(());
            }
            if ch == '\\' {
                self.advance();
            }
        }
        Err(invalid())
    }
    fn atom(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || matches!(ch, ':' | ',' | ']' | '}' | '/') {
                break;
            }
            self.advance();
        }
    }
    fn node(&mut self) -> Result<Node, AppError> {
        self.trivia()?;
        let start = self.offset;
        let kind = match self.peek().ok_or_else(invalid)? {
            '{' => {
                self.advance();
                self.trivia()?;
                let mut members = Vec::new();
                while self.peek() != Some('}') {
                    let key_start = self.offset;
                    if matches!(self.peek(), Some('"' | '\'')) {
                        self.quoted()?;
                    } else {
                        self.atom();
                    }
                    let spelling = &self.text[key_start..self.offset];
                    let name: String = if spelling.starts_with(['"', '\'']) {
                        json5::from_str(spelling)
                    } else {
                        json5::from_str(&format!("\"{spelling}\""))
                    }
                    .map_err(|_| invalid())?;
                    self.trivia()?;
                    if self.peek() != Some(':') {
                        return Err(invalid());
                    }
                    self.advance();
                    let value = self.node()?;
                    self.trivia()?;
                    let comma = (self.peek() == Some(',')).then_some(self.offset);
                    if comma.is_some() {
                        self.advance();
                        self.trivia()?;
                    } else if self.peek() != Some('}') {
                        return Err(invalid());
                    }
                    members.push(Member {
                        name,
                        start: key_start,
                        value,
                        comma,
                    });
                }
                self.advance();
                Kind::Object(members)
            }
            '[' => {
                self.advance();
                self.trivia()?;
                let mut values = Vec::new();
                while self.peek() != Some(']') {
                    values.push(self.node()?);
                    self.trivia()?;
                    if self.peek() == Some(',') {
                        self.advance();
                        self.trivia()?;
                    } else if self.peek() != Some(']') {
                        return Err(invalid());
                    }
                }
                self.advance();
                Kind::Array(values)
            }
            '"' | '\'' => {
                self.quoted()?;
                Kind::Scalar
            }
            _ => {
                self.atom();
                if start == self.offset {
                    return Err(invalid());
                }
                Kind::Scalar
            }
        };
        Ok(Node {
            span: start..self.offset,
            kind,
        })
    }
}
fn syntax(source: &str) -> Result<Node, AppError> {
    let mut scanner = Scanner {
        text: source,
        offset: 0,
    };
    let node = scanner.node()?;
    scanner.trivia()?;
    if scanner.offset != source.len() {
        return Err(invalid());
    }
    Ok(node)
}
struct Edit {
    range: Range<usize>,
    replacement: String,
}
struct Layout<'a> {
    source: &'a str,
    newline: &'static str,
    unit: &'a str,
}
impl<'a> Layout<'a> {
    fn new(source: &'a str) -> Self {
        let unit = source
            .lines()
            .find_map(|line| {
                let length = line.len() - line.trim_start_matches([' ', '\t']).len();
                (length > 0 && !line.trim().is_empty()).then_some(&line[..length])
            })
            .unwrap_or("  ");
        Self {
            source,
            unit,
            newline: if source.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            },
        }
    }
    fn indent(&self, offset: usize) -> &str {
        let start = self.source[..offset]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let line = &self.source[start..offset];
        &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
    }
    fn print(&self, value: &Value, indent: &str) -> Result<String, AppError> {
        let text = serde_json::to_string_pretty(value)
            .map_err(|source| AppError::JsonSerialize { source })?;
        let mut lines = text.lines();
        let mut result = lines.next().unwrap_or_default().to_string();
        for line in lines {
            let content = line.trim_start_matches(' ');
            let depth = (line.len() - content.len()) / 2;
            result.push_str(self.newline);
            result.push_str(indent);
            result.push_str(&self.unit.repeat(depth));
            result.push_str(content);
        }
        Ok(result)
    }
    fn deletion(&self, member: &Member) -> Range<usize> {
        let end = member
            .comma
            .map_or(member.value.span.end, |position| position + 1);
        let start = self.source[..member.start]
            .rfind('\n')
            .map_or(0, |position| position + 1);
        if self.source[start..member.start]
            .trim_matches([' ', '\t'])
            .is_empty()
        {
            if let Some(newline) = self.source[end..].find('\n') {
                if self.source[end..end + newline]
                    .trim_matches([' ', '\t', '\r'])
                    .is_empty()
                {
                    return start..end + newline + 1;
                }
            }
        }
        member.start..end
    }
    fn diff(
        &self,
        node: &Node,
        before: &Value,
        after: &Value,
        edits: &mut Vec<Edit>,
    ) -> Result<(), AppError> {
        if before == after {
            return Ok(());
        }
        match (&node.kind, before, after) {
            (Kind::Object(members), Value::Object(old), Value::Object(new)) => {
                let mut unique = HashSet::new();
                if members.iter().any(|member| !unique.insert(&member.name)) {
                    return Err(invalid());
                }
                let additions = new
                    .iter()
                    .filter(|(key, _)| !old.contains_key(*key))
                    .collect::<Vec<_>>();
                for member in members {
                    match new.get(&member.name) {
                        Some(value) => self.diff(
                            &member.value,
                            old.get(&member.name).ok_or_else(invalid)?,
                            value,
                            edits,
                        )?,
                        None => edits.push(Edit {
                            range: self.deletion(member),
                            replacement: String::new(),
                        }),
                    }
                }
                let retained = members
                    .iter()
                    .rev()
                    .find(|member| new.contains_key(&member.name));
                let trailing_comma = members.last().is_some_and(|member| member.comma.is_some());
                if additions.is_empty() {
                    if !trailing_comma
                        && members
                            .last()
                            .is_some_and(|member| !new.contains_key(&member.name))
                    {
                        if let Some(position) = retained.and_then(|member| member.comma) {
                            edits.push(Edit {
                                range: position..position + 1,
                                replacement: String::new(),
                            });
                        }
                    }
                } else {
                    if let Some(member) = retained.filter(|member| member.comma.is_none()) {
                        edits.push(Edit {
                            range: member.value.span.end..member.value.span.end,
                            replacement: ",".into(),
                        });
                    }
                    let parent = self.indent(node.span.start);
                    let indent = format!("{parent}{}", self.unit);
                    let mut extra = Vec::new();
                    for (key, value) in additions {
                        extra.push(format!(
                            "{indent}{}: {}",
                            serde_json::to_string(key)
                                .map_err(|source| AppError::JsonSerialize { source })?,
                            self.print(value, &indent)?
                        ));
                    }
                    let close = node.span.end - 1;
                    let line = self.source[..close]
                        .rfind('\n')
                        .map_or(0, |position| position + 1);
                    let own_line = self.source[line..close]
                        .trim_matches([' ', '\t'])
                        .is_empty();
                    let at = if own_line { line } else { close };
                    let replacement = format!(
                        "{}{}{}{}{}",
                        if own_line { "" } else { self.newline },
                        extra.join(&format!(",{}", self.newline)),
                        if trailing_comma { "," } else { "" },
                        self.newline,
                        if own_line { "" } else { parent },
                    );
                    edits.push(Edit {
                        range: at..at,
                        replacement,
                    });
                }
            }
            (Kind::Array(nodes), Value::Array(old), Value::Array(new))
                if old.len() == new.len() && nodes.len() == old.len() =>
            {
                for ((node, old), new) in nodes.iter().zip(old).zip(new) {
                    self.diff(node, old, new, edits)?;
                }
            }
            _ => edits.push(Edit {
                range: node.span.clone(),
                replacement: self.print(after, self.indent(node.span.start))?,
            }),
        }
        Ok(())
    }
}

pub struct JsoncDocument {
    original: String,
    initial: Value,
    source: String,
    semantic: Value,
}
impl JsoncDocument {
    pub fn parse(source: &str) -> Result<Self, AppError> {
        let value = read_value(source)?;
        if !value.is_object() {
            return Err(invalid());
        }
        syntax(source)?;
        Ok(Self {
            original: source.into(),
            initial: value.clone(),
            source: source.into(),
            semantic: value,
        })
    }
    pub fn value(&self) -> &Value {
        &self.semantic
    }
    pub fn original_source(&self) -> &str {
        &self.original
    }
    pub fn root_key_count(&self, name: &str) -> Result<usize, AppError> {
        match syntax(&self.source)?.kind {
            Kind::Object(members) => {
                Ok(members.iter().filter(|member| member.name == name).count())
            }
            _ => Err(invalid()),
        }
    }
    pub fn apply(&mut self, desired: &Value) -> Result<bool, AppError> {
        if !desired.is_object() {
            return Err(invalid());
        }
        if desired == &self.semantic {
            return Ok(false);
        }
        let next = if desired == &self.initial {
            self.original.clone()
        } else {
            let tree = syntax(&self.source)?;
            let mut edits = Vec::new();
            Layout::new(&self.source).diff(&tree, &self.semantic, desired, &mut edits)?;
            edits.sort_by_key(|edit| edit.range.start);
            let mut source = self.source.clone();
            let mut limit = source.len();
            for edit in edits.into_iter().rev() {
                if edit.range.end > limit {
                    return Err(invalid());
                }
                limit = edit.range.start;
                source.replace_range(edit.range, &edit.replacement);
            }
            source
        };
        if read_value(&next)? != *desired {
            return Err(invalid());
        }
        self.source = next;
        self.semantic = desired.clone();
        Ok(true)
    }
    pub fn validated_source(&self) -> Result<String, AppError> {
        if read_value(&self.source)? != self.semantic {
            return Err(invalid());
        }
        Ok(self.source.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn comments_escaped_keys_and_untouched_values_survive_nested_updates() {
        let source = "/*独立测试*/ {\r\n  'p': { '\\u0061': 'old', keep: [/*注释*/ 0x10, 2,], },\r\n  untouched: '\\u4e2d',\r\n}\r\n//尾部中文";
        let mut doc = JsoncDocument::parse(source).unwrap();
        let initial = doc.value().clone();
        let mut desired = initial.clone();
        desired["p"]["a"] = json!("new");
        desired["p"]["added"] = json!({"nested":true});
        doc.apply(&desired).unwrap();
        let output = doc.validated_source().unwrap();
        for kept in [
            "/*独立测试*/",
            "/*注释*/ 0x10, 2,",
            "untouched: '\\u4e2d'",
            "//尾部中文",
        ] {
            assert!(output.contains(kept), "{kept}");
        }
        assert_eq!(read_value(&output).unwrap(), desired);
        doc.apply(&initial).unwrap();
        assert_eq!(doc.validated_source().unwrap(), source);
    }
    #[test]
    fn deleting_and_adding_members_keeps_a_valid_separator_layout() {
        for source in [
            "{a:1,b:2,c:3}",
            "{a:1,b:2,c:3,}",
            "{ /* hello */ }",
            "{a:1,// note\nb:2,c:3}",
        ] {
            for desired in [
                json!({}),
                json!({"a":1}),
                json!({"a":1,"d":[4,5]}),
                json!({"d":9}),
            ] {
                let mut document = JsoncDocument::parse(source).unwrap();
                document.apply(&desired).unwrap();
                assert_eq!(
                    read_value(&document.validated_source().unwrap()).unwrap(),
                    desired
                );
            }
        }
    }
    #[test]
    fn duplicate_keys_are_counted_but_ambiguous_edits_are_refused() {
        let mut doc = JsoncDocument::parse("{a:1, 'a':2}").unwrap();
        assert_eq!(doc.root_key_count("a").unwrap(), 2);
        assert!(doc.apply(&json!({"a":3})).is_err());
        assert_eq!(doc.validated_source().unwrap(), "{a:1, 'a':2}");
        doc.source = "{different: true}".into();
        assert!(doc.validated_source().is_err());
    }
}
