use crate::ast::{Attrs, Declaration, Hint, Node, NodeKind, SeriesConfig, SeriesDisplay, Value};

use super::inline::find_matching_delim;

/// Extract `{...}` at the end of a line, returning (text_before, Some(brace_content)).
pub(super) fn extract_trailing_brace_attrs(text: &str) -> (&str, Option<String>) {
    let trimmed = text.trim_end();
    if !trimmed.ends_with('}') {
        return (text, None);
    }
    let start = find_matching_open_brace(trimmed);
    if let Some(s) = start {
        let before = trimmed[..s].trim_end();
        let brace_content = &trimmed[s + 1..trimmed.len() - 1];
        (before, Some(brace_content.to_string()))
    } else {
        (text, None)
    }
}

/// Extract a `=format` token from brace attributes (e.g. `{=html}` or `{=typst #id .class}`).
/// Returns `(Some(format), remaining_attrs)` if found, `(None, original_attrs)` otherwise.
pub(super) fn extract_raw_format(attrs: Option<&str>) -> (Option<String>, Option<String>) {
    let attrs = match attrs {
        Some(a) => a.trim(),
        None => return (None, None),
    };

    // Look for =word token
    let mut result_format = None;
    let mut remaining_parts = Vec::new();

    for token in attrs.split_whitespace() {
        if result_format.is_none() && token.starts_with('=') && token.len() > 1 {
            result_format = Some(token[1..].to_string());
        } else {
            remaining_parts.push(token);
        }
    }

    let remaining = if remaining_parts.is_empty() {
        None
    } else {
        Some(remaining_parts.join(" "))
    };

    (result_format, remaining)
}

pub(super) fn apply_brace_attrs(attrs: &mut Attrs, content: &str) {
    let content = content.trim();
    let mut pos = 0;
    let chars: Vec<char> = content.chars().collect();

    while pos < chars.len() {
        if chars[pos].is_whitespace() {
            pos += 1;
            continue;
        }
        match chars[pos] {
            '#' | '.' => {
                pos = scan_id_or_class(chars[pos], &chars, pos + 1, attrs);
            }
            '@' => {
                pos = parse_attr_directive(&chars, pos, attrs);
            }
            _ => {
                pos = parse_attr_key_value(&chars, pos, attrs);
            }
        }
    }
}

/// Check the last child for trailing brace attrs and apply to parent or preceding node.
pub(super) fn maybe_extract_trailing_attrs_from_children(children: &mut Vec<Node>) {
    let Some((attr_str, before)) = extract_trailing_brace_from_last_child(children) else {
        return;
    };
    if before.is_empty() {
        children.pop();
        if let Some(prev) = children.last_mut() {
            apply_brace_attrs(&mut prev.attrs, &attr_str);
        }
    } else {
        *children.last_mut().expect("children non-empty") = Node::text(before);
        let len = children.len();
        if len >= 2 {
            let (head, _tail) = children.split_at_mut(len - 1);
            apply_brace_attrs(
                &mut head.last_mut().expect("head non-empty").attrs,
                &attr_str,
            );
        }
    }
}

/// Parse a standalone `@directive(...)` into a node carrying declarations/hints.
pub(super) fn parse_directive_to_node(content: &str) -> Option<Node> {
    let mut node = Node::new(NodeKind::Custom {
        content_type: "directive".to_string(),
    });
    // content looks like: @series(...) or @hint(...)
    let content = content.trim();
    if !content.starts_with('@') {
        return None;
    }
    let rest = &content[1..];
    let paren = rest.find('(')?;
    let cmd = &rest[..paren];
    let close = rest.rfind(')')?;
    let args = &rest[paren + 1..close];
    apply_directive(&mut node.attrs, cmd, args);
    Some(node)
}

// ── Internal helpers ──

fn scan_token(chars: &[char], pos: usize) -> (String, usize) {
    let start = pos;
    let mut p = pos;
    while p < chars.len() && !chars[p].is_whitespace() {
        p += 1;
    }
    (chars[start..p].iter().collect(), p)
}

fn parse_attr_directive(chars: &[char], pos: usize, attrs: &mut Attrs) -> usize {
    let mut p = pos + 1; // skip '@'
    while p < chars.len() && (chars[p].is_alphanumeric() || chars[p] == '_') {
        p += 1;
    }
    let cmd: String = chars[pos + 1..p].iter().collect();
    if p < chars.len() && chars[p] == '(' {
        let paren_start = p;
        if let Some(close) = find_matching_delim(chars, p, '(', ')') {
            let args: String = chars[paren_start + 1..close].iter().collect();
            apply_directive(attrs, &cmd, &args);
            return close + 1;
        }
    }
    p
}

fn parse_attr_key_value(chars: &[char], pos: usize, attrs: &mut Attrs) -> usize {
    let key_start = pos;
    let mut p = pos;
    while p < chars.len() && chars[p] != '=' && !chars[p].is_whitespace() {
        p += 1;
    }
    let key: String = chars[key_start..p].iter().collect();
    if key.is_empty() {
        return p + 1;
    }
    if p >= chars.len() || chars[p] != '=' {
        // Bare token without value — treat as boolean true.
        attrs.properties.insert(key, Value::Bool(true));
        return p;
    }
    p += 1; // skip =
    let (value, new_pos) = parse_attr_value(chars, p);
    attrs.properties.insert(key, Value::Text(value));
    new_pos
}

fn parse_attr_value(chars: &[char], pos: usize) -> (String, usize) {
    if pos < chars.len() && chars[pos] == '"' {
        let mut p = pos + 1;
        let val_start = p;
        while p < chars.len() && chars[p] != '"' {
            p += 1;
        }
        let v: String = chars[val_start..p].iter().collect();
        if p < chars.len() {
            p += 1; // skip closing quote
        }
        (v, p)
    } else {
        let (v, p) = scan_token(chars, pos);
        (v, p)
    }
}

fn scan_id_or_class(sigil: char, chars: &[char], pos: usize, attrs: &mut Attrs) -> usize {
    let (token, new_pos) = scan_token(chars, pos);
    if sigil == '#' {
        attrs.id = token;
        attrs.auto_id = false;
    } else {
        attrs.classes.push(token);
    }
    new_pos
}

fn apply_directive(attrs: &mut Attrs, cmd: &str, args: &str) {
    match cmd {
        "series" => {
            if let Some(decl) = parse_series_directive(args) {
                attrs.declarations.push(decl);
            }
        }
        "hint" => {
            attrs.hints.extend(parse_hint_directive(args));
        }
        _ => {}
    }
}

/// Parse `@series(figs, label="Figure", display=hierarchical)`
fn parse_series_directive(args: &str) -> Option<Declaration> {
    let parts = split_directive_args(args);
    if parts.is_empty() {
        return None;
    }
    let id = parts[0].0.clone();
    let mut label = id.clone();
    let mut display = SeriesDisplay::Arabic;
    for (key, val) in &parts[1..] {
        match key.as_str() {
            "label" => label = val.clone(),
            "display" => {
                display = match val.as_str() {
                    "hierarchical" => SeriesDisplay::Hierarchical,
                    "roman" => SeriesDisplay::Roman,
                    "alpha" => SeriesDisplay::Alpha,
                    _ => SeriesDisplay::Arabic,
                };
            }
            _ => {}
        }
    }
    Some(Declaration::Series {
        id,
        config: SeriesConfig { label, display },
    })
}

/// Parse `@hint(figure.wide, layout="full", position="top")`.
///
/// The args are split into a selector portion and property assignments.
/// The boundary is the first top-level comma-separated token that looks
/// like a property assignment (`key=value` where key is a bare identifier).
/// Returns one `Hint` per property, all sharing the same selector.
fn parse_hint_directive(args: &str) -> Vec<Hint> {
    let parts = split_top_level_hint_commas(args);
    if parts.is_empty() {
        return vec![];
    }

    // Find the boundary: first token that is a property assignment.
    let boundary = parts.iter().position(|p| is_property_token(p));

    let (selector_str, prop_parts) = match boundary {
        Some(idx) => {
            let sel = parts[..idx].join(",");
            (sel, &parts[idx..])
        }
        None => return vec![],
    };

    let selector = match crate::selector::parse_selector(&selector_str) {
        Some(s) => s,
        None => return vec![],
    };

    let mut hints = Vec::new();
    for prop in prop_parts {
        let prop = prop.trim();
        if let Some(eq) = find_top_level_eq(prop) {
            let key = prop[..eq].trim().to_string();
            let val_str = prop[eq + 1..].trim();
            let value = parse_hint_value(val_str);
            hints.push(Hint {
                selector: selector.clone(),
                property: key,
                value,
            });
        }
    }
    hints
}

/// Check if a comma-separated token is a property assignment (bare identifier followed by `=`).
fn is_property_token(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty()
        || token.starts_with('.')
        || token.starts_with('#')
        || token.starts_with('[')
    {
        return false;
    }
    if token == "this" {
        return false;
    }
    let mut depth = 0i32;
    let mut in_quotes = false;
    for (i, ch) in token.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '[' | '(' if !in_quotes => depth += 1,
            ']' | ')' if !in_quotes => depth -= 1,
            '=' if !in_quotes && depth == 0 => {
                let before = &token[..i];
                return before
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
            }
            _ => {}
        }
    }
    false
}

/// Parse a hint property value string into a `Value`.
fn parse_hint_value(s: &str) -> Value {
    let s = s.trim().trim_matches('"');
    match s {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => {
            if let Ok(n) = s.parse::<f64>() {
                Value::Number(n)
            } else {
                Value::Text(s.to_string())
            }
        }
    }
}

/// Split hint args on top-level commas (not inside `[...]`, `(...)`, or `"`).
fn split_top_level_hint_commas(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut in_quotes = false;

    for (i, ch) in input.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '[' | '(' if !in_quotes => depth += 1,
            ']' | ')' if !in_quotes => depth -= 1,
            ',' if !in_quotes && depth == 0 => {
                parts.push(input[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(input[start..].to_string());
    parts
}

/// Split directive args like `figs, label="Figure", display=hierarchical`
/// into [(key, value), ...]. Positional args have key == value.
fn scan_delimited_token(chars: &[char], start: usize) -> usize {
    let mut pos = start;
    let mut depth = 0;
    let mut in_quotes = false;
    while pos < chars.len() {
        let ch = chars[pos];
        match ch {
            '"' => in_quotes = !in_quotes,
            '(' if !in_quotes => depth += 1,
            ')' if !in_quotes && depth > 0 => depth -= 1,
            ',' if !in_quotes && depth == 0 => break,
            _ => {}
        }
        pos += 1;
    }
    pos
}

fn split_kv(token: &str) -> (String, String) {
    if let Some(eq_pos) = find_top_level_eq(token) {
        let key = token[..eq_pos].trim().to_string();
        let val = token[eq_pos + 1..].trim().trim_matches('"').to_string();
        (key, val)
    } else {
        (token.to_string(), token.to_string())
    }
}

fn split_directive_args(args: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = args.chars().collect();
    let mut pos = 0;
    std::iter::from_fn(|| {
        while pos < chars.len() && (chars[pos].is_whitespace() || chars[pos] == ',') {
            pos += 1;
        }
        if pos >= chars.len() {
            return None;
        }
        let start = pos;
        pos = scan_delimited_token(&chars, start);
        let token: String = chars[start..pos].iter().collect();
        let token = token.trim();
        if token.is_empty() {
            return None;
        }
        Some(split_kv(token))
    })
    .collect()
}

fn find_top_level_eq(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_quotes = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '(' if !in_quotes => depth += 1,
            ')' if !in_quotes => depth -= 1,
            '=' if !in_quotes && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn extract_trailing_brace_from_last_child(children: &[Node]) -> Option<(String, String)> {
    let last = children.last()?;
    let NodeKind::Text(ref text) = last.kind else {
        return None;
    };
    let trimmed = text.trim_end();
    let brace_start = find_matching_open_brace(trimmed)?;
    let attr_str = trimmed[brace_start + 1..trimmed.len() - 1].to_string();
    let before = trimmed[..brace_start].trim_end().to_string();
    Some((attr_str, before))
}

fn find_matching_open_brace(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.last() != Some(&b'}') {
        return None;
    }
    let mut depth = 0i32;
    for i in (0..bytes.len()).rev() {
        if bytes[i] == b'}' {
            depth += 1;
        } else if bytes[i] == b'{' {
            depth -= 1;
        }
        if depth == 0 {
            return Some(i);
        }
    }
    None
}
