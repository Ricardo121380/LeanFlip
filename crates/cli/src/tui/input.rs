//! Width-aware single-line terminal fields, shared by connection and backup UI.
use super::*;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) fn fit(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut used = 0;
    let mut result = String::new();
    for ch in value.chars() {
        let next = ch.width().unwrap_or(0);
        if used + next > width.saturating_sub(1) {
            break;
        }
        result.push(ch);
        used += next;
    }
    result.push('…');
    result
}

pub(super) fn field(value: &str, cursor: usize, secret: bool, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let (value, cursor) = if secret {
        (
            "*".repeat(value.chars().count()),
            value[..cursor].chars().count(),
        )
    } else {
        (clean(value), clean(&value[..cursor]).len())
    };
    let mut start = 0;
    while value[start..cursor].width() >= width.saturating_sub(1).max(1) && start < cursor {
        start += value[start..].chars().next().unwrap().len_utf8();
    }
    let before = &value[start..cursor];
    let rest = fit(&value[cursor..], width.saturating_sub(before.width() + 1));
    format!("{before}▏{rest}")
}

pub(super) fn insert(value: &mut String, cursor: &mut usize, text: &str) -> bool {
    let text = text.trim_end_matches(['\r', '\n']);
    if text.chars().any(char::is_control) || value.chars().count() + text.chars().count() > 4096 {
        return false;
    }
    value.insert_str(*cursor, text);
    *cursor += text.len();
    true
}
pub(super) fn key(value: &mut String, cursor: &mut usize, key: KeyEvent) {
    match key.code {
        KeyCode::Left => {
            *cursor = value[..*cursor]
                .char_indices()
                .next_back()
                .map(|(index, _)| index)
                .unwrap_or(0)
        }
        KeyCode::Right => {
            if let Some(ch) = value[*cursor..].chars().next() {
                *cursor += ch.len_utf8();
            }
        }
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = value.len(),
        KeyCode::Backspace if *cursor > 0 => {
            let previous = value[..*cursor].char_indices().next_back().unwrap().0;
            value.drain(previous..*cursor);
            *cursor = previous;
        }
        KeyCode::Delete if *cursor < value.len() => {
            let length = value[*cursor..].chars().next().unwrap().len_utf8();
            value.drain(*cursor..*cursor + length);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            value.clear();
            *cursor = 0;
        }
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && !ch.is_control() =>
        {
            insert(value, cursor, &ch.to_string());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fields_fit_cjk_and_keep_cursor_and_password_hidden() {
        for width in 1..60 {
            let text = "很长的中文配置目录/提供商/终端-test";
            for cursor in text
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(text.len()))
            {
                let shown = field(text, cursor, false, width);
                assert!(shown.width() <= width, "{width}: {shown}");
                assert!(shown.contains('▏'));
            }
            let secret = field("private-credential", 9, true, width);
            assert!(!secret.contains("private"));
            assert!(secret.width() <= width);
        }
    }
    #[test]
    fn insertion_supports_paste_but_rejects_multiline_and_escape_sequences() {
        let mut text = "中文".to_string();
        let mut cursor = text.len();
        assert!(insert(&mut text, &mut cursor, "-paste\n"));
        let before = text.clone();
        assert!(!insert(&mut text, &mut cursor, "injected\nline"));
        assert!(!insert(&mut text, &mut cursor, "\x1b[2J"));
        assert_eq!(text, before);
        key(
            &mut text,
            &mut cursor,
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        );
        assert_eq!(cursor, 0);
    }
}
