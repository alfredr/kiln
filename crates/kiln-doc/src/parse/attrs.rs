use crate::ast::{Attrs, Declaration, Hint, Node, NodeKind, SeriesConfig, Value};

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
/// Also handles the colon shorthand `{=typst:#id}` — splits at the first `:` to separate
/// format from trailing attrs.
/// Returns `(Some(format), remaining_attrs)` if found, `(None, original_attrs)` otherwise.
pub(super) fn extract_raw_format(attrs: Option<&str>) -> (Option<String>, Option<String>) {
    let attrs = match attrs {
        Some(a) => a.trim(),
        None => return (None, None),
    };

    let mut result_format = None;
    let mut remaining_parts = Vec::new();

    for token in attrs.split_whitespace() {
        if result_format.is_none() && token.starts_with('=') && token.len() > 1 {
            let (fmt, colon_rest) = split_format_token(&token[1..]);
            result_format = Some(fmt);
            remaining_parts.extend(colon_rest);
        } else {
            remaining_parts.push(token.to_string());
        }
    }

    let remaining = if remaining_parts.is_empty() {
        None
    } else {
        Some(remaining_parts.join(" "))
    };

    (result_format, remaining)
}

/// Split a format token at the first `:` for the colon shorthand.
/// `"typst:#id"` → `("typst", Some("#id"))`, `"html"` → `("html", None)`.
fn split_format_token(value: &str) -> (String, Option<String>) {
    let Some(colon) = value.find(':') else {
        return (value.to_string(), None);
    };
    let format = value[..colon].to_string();
    let after = &value[colon + 1..];
    let rest = if after.is_empty() {
        None
    } else {
        Some(after.to_string())
    };
    (format, rest)
}

/// Extract a language token from brace attributes.
/// The language is the first bare word that isn't `#id`, `.class`, `@directive`,
/// `=format`, or `key=value`.
/// Returns `(Some(lang), remaining_attrs)` or `(None, original_attrs)`.
pub(super) fn extract_lang(attrs: Option<&str>) -> (Option<String>, Option<String>) {
    let attrs = match attrs {
        Some(a) if !a.trim().is_empty() => a.trim(),
        _ => return (None, None),
    };

    let mut lang = None;
    let mut remaining = Vec::new();

    for token in attrs.split_whitespace() {
        if lang.is_none()
            && !token.starts_with('#')
            && !token.starts_with('.')
            && !token.starts_with('@')
            && !token.starts_with('=')
            && !token.contains('=')
        {
            lang = Some(token.to_string());
        } else {
            remaining.push(token);
        }
    }

    let rest = if remaining.is_empty() {
        None
    } else {
        Some(remaining.join(" "))
    };

    (lang, rest)
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
            if let Some((decl, hints)) = parse_series_directive_full(args) {
                attrs.declarations.push(decl);
                attrs.hints.extend(hints);
            }
        }
        "hint" => {
            attrs.hints.extend(parse_hint_directive(args));
        }
        _ => {}
    }
}

/// Parse `@series(fig, counts=figure, parent=sec, formats=["I","1"], cycle=true)`.
///
/// The `counts=SELECTOR` sugar generates a `Hint` on the enclosing node that
/// sets `series=ID` on matching descendants.  Returns `(Declaration, Vec<Hint>)`.
fn parse_series_directive_full(args: &str) -> Option<(Declaration, Vec<Hint>)> {
    let parts = split_directive_args(args);
    if parts.is_empty() {
        return None;
    }
    let id = parts[0].0.clone();
    let mut parent: Option<String> = None;
    let mut formats: Vec<String> = vec!["1".to_string()];
    let mut cycle = false;
    let mut counts_hints: Vec<Hint> = Vec::new();

    for (key, val) in &parts[1..] {
        match key.as_str() {
            "parent" => parent = Some(val.clone()),
            "cycle" => cycle = val == "true",
            "formats" => {
                formats = parse_format_list(val);
            }
            "counts" => {
                // `counts=SELECTOR` expands to a hint: @hint(SELECTOR, series="ID")
                counts_hints.extend(
                    crate::selector::parse_selector(val).map(|selector| Hint {
                        selector,
                        property: "series".to_string(),
                        value: Value::Text(id.clone()),
                    }),
                );
            }
            _ => {}
        }
    }
    let decl = Declaration::Series {
        id,
        config: SeriesConfig {
            parent,
            formats,
            cycle,
        },
    };
    Some((decl, counts_hints))
}

/// Parse a format list like `["I","1","a"]` into a `Vec<String>`.
/// Also accepts a single bare token like `I` → `vec!["I"]`.
fn parse_format_list(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        inner
            .split(',')
            .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|t| !t.is_empty())
            .collect()
    } else {
        vec![s.to_string()]
    }
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
            '(' | '[' if !in_quotes => depth += 1,
            ')' | ']' if !in_quotes && depth > 0 => depth -= 1,
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
            '(' | '[' if !in_quotes => depth += 1,
            ')' | ']' if !in_quotes => depth -= 1,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_trailing_brace_attrs ──

    #[test]
    fn trailing_brace_basic() {
        let (before, attrs) = extract_trailing_brace_attrs("text {#id}");
        assert_eq!(before, "text");
        assert_eq!(attrs.as_deref(), Some("#id"));
    }

    #[test]
    fn trailing_brace_none_when_no_brace() {
        let (before, attrs) = extract_trailing_brace_attrs("no braces here");
        assert_eq!(before, "no braces here");
        assert!(attrs.is_none());
    }

    #[test]
    fn trailing_brace_unmatched() {
        let (_, attrs) = extract_trailing_brace_attrs("unmatched }");
        // Unmatched } should still try to find matching {
        // Since there's no {, should return None
        assert!(attrs.is_none());
    }

    #[test]
    fn trailing_brace_nested() {
        let (before, attrs) = extract_trailing_brace_attrs("text {outer {inner}}");
        assert_eq!(before, "text");
        assert_eq!(attrs.as_deref(), Some("outer {inner}"));
    }

    // ── extract_raw_format ──

    #[test]
    fn raw_format_none_input() {
        let (fmt, rest) = extract_raw_format(None);
        assert!(fmt.is_none());
        assert!(rest.is_none());
    }

    #[test]
    fn raw_format_html() {
        let (fmt, rest) = extract_raw_format(Some("=html"));
        assert_eq!(fmt.as_deref(), Some("html"));
        assert!(rest.is_none());
    }

    #[test]
    fn raw_format_with_remaining() {
        let (fmt, rest) = extract_raw_format(Some("=html #id .class"));
        assert_eq!(fmt.as_deref(), Some("html"));
        assert_eq!(rest.as_deref(), Some("#id .class"));
    }

    #[test]
    fn raw_format_no_equals() {
        let (fmt, rest) = extract_raw_format(Some("rust #id"));
        assert!(fmt.is_none());
        assert_eq!(rest.as_deref(), Some("rust #id"));
    }

    #[test]
    fn raw_format_equals_only() {
        // "=" alone — no format extracted (len == 1, not > 1)
        let (fmt, rest) = extract_raw_format(Some("="));
        assert!(fmt.is_none());
        assert_eq!(rest.as_deref(), Some("="));
    }

    #[test]
    fn raw_format_colon_shorthand() {
        let (fmt, rest) = extract_raw_format(Some("=typst:#id"));
        assert_eq!(fmt.as_deref(), Some("typst"));
        assert_eq!(rest.as_deref(), Some("#id"));
    }

    // ── split_format_token ──

    #[test]
    fn split_format_no_colon() {
        let (fmt, rest) = split_format_token("html");
        assert_eq!(fmt, "html");
        assert!(rest.is_none());
    }

    #[test]
    fn split_format_colon_with_rest() {
        let (fmt, rest) = split_format_token("typst:#id");
        assert_eq!(fmt, "typst");
        assert_eq!(rest.as_deref(), Some("#id"));
    }

    #[test]
    fn split_format_trailing_colon() {
        let (fmt, rest) = split_format_token("typst:");
        assert_eq!(fmt, "typst");
        assert!(rest.is_none());
    }

    // ── extract_lang ──

    #[test]
    fn extract_lang_bare_word() {
        let (lang, rest) = extract_lang(Some("rust"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert!(rest.is_none());
    }

    #[test]
    fn extract_lang_with_attrs() {
        let (lang, rest) = extract_lang(Some("rust #id .class"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(rest.as_deref(), Some("#id .class"));
    }

    #[test]
    fn extract_lang_no_bare_word() {
        let (lang, rest) = extract_lang(Some("#id .class"));
        assert!(lang.is_none());
        assert_eq!(rest.as_deref(), Some("#id .class"));
    }

    #[test]
    fn extract_lang_none() {
        let (lang, rest) = extract_lang(None);
        assert!(lang.is_none());
        assert!(rest.is_none());
    }

    #[test]
    fn extract_lang_empty() {
        let (lang, rest) = extract_lang(Some(""));
        assert!(lang.is_none());
        assert!(rest.is_none());
    }

    #[test]
    fn extract_lang_whitespace_only() {
        let (lang, rest) = extract_lang(Some("   "));
        assert!(lang.is_none());
        assert!(rest.is_none());
    }

    #[test]
    fn extract_lang_key_value_skipped() {
        let (lang, rest) = extract_lang(Some("key=value rust"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(rest.as_deref(), Some("key=value"));
    }

    #[test]
    fn extract_lang_format_skipped() {
        let (lang, rest) = extract_lang(Some("=html rust"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(rest.as_deref(), Some("=html"));
    }

    #[test]
    fn extract_lang_directive_skipped() {
        let (lang, rest) = extract_lang(Some("@hint rust"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(rest.as_deref(), Some("@hint"));
    }

    // ── apply_brace_attrs ──

    #[test]
    fn brace_attrs_id() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "#my-id");
        assert_eq!(attrs.id, "my-id");
        assert!(!attrs.auto_id);
    }

    #[test]
    fn brace_attrs_class() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, ".wide");
        assert_eq!(attrs.classes, vec!["wide"]);
    }

    #[test]
    fn brace_attrs_key_value() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "width=\"80%\"");
        assert!(matches!(attrs.properties.get("width"), Some(Value::Text(s)) if s == "80%"));
    }

    #[test]
    fn brace_attrs_bare_boolean() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "nowrap");
        assert!(matches!(attrs.properties.get("nowrap"), Some(Value::Bool(true))));
    }

    #[test]
    fn brace_attrs_directive() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "@hint(figure, series=figs)");
        assert!(!attrs.hints.is_empty());
    }

    #[test]
    fn brace_attrs_multiple() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "#my-id .wide .dark nowrap");
        assert_eq!(attrs.id, "my-id");
        assert!(!attrs.auto_id);
        assert!(attrs.classes.contains(&"wide".to_string()));
        assert!(attrs.classes.contains(&"dark".to_string()));
        assert!(matches!(attrs.properties.get("nowrap"), Some(Value::Bool(true))));
    }

    #[test]
    fn brace_attrs_whitespace_only() {
        let mut attrs = Attrs::default();
        let orig_id = attrs.id.clone();
        apply_brace_attrs(&mut attrs, "   ");
        assert_eq!(attrs.id, orig_id);
    }

    // ── parse_directive_to_node ──

    #[test]
    fn directive_to_node_series() {
        let node = parse_directive_to_node("@series(fig, counts=figure)");
        assert!(node.is_some());
        let n = node.unwrap();
        assert!(!n.attrs.declarations.is_empty());
    }

    #[test]
    fn directive_to_node_hint() {
        let node = parse_directive_to_node("@hint(figure, series=figs)");
        assert!(node.is_some());
        let n = node.unwrap();
        assert!(!n.attrs.hints.is_empty());
    }

    #[test]
    fn directive_to_node_not_directive() {
        let node = parse_directive_to_node("not a directive");
        assert!(node.is_none());
    }

    #[test]
    fn directive_to_node_no_parens() {
        let node = parse_directive_to_node("@series");
        assert!(node.is_none());
    }

    #[test]
    fn directive_to_node_unclosed_paren() {
        let node = parse_directive_to_node("@series(fig, counts=figure");
        assert!(node.is_none());
    }

    // ── maybe_extract_trailing_attrs_from_children ──

    #[test]
    fn trailing_attrs_applied_to_previous_node() {
        let mut children = vec![
            Node::emphasis(vec![Node::text("em")]),
            Node::text(" {.highlight}"),
        ];
        maybe_extract_trailing_attrs_from_children(&mut children);
        // The {.highlight} should be applied to the emphasis node
        assert_eq!(children.len(), 1);
        assert!(children[0].attrs.classes.contains(&"highlight".to_string()));
    }

    #[test]
    fn trailing_attrs_no_brace() {
        let mut children = vec![Node::text("no braces")];
        maybe_extract_trailing_attrs_from_children(&mut children);
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn trailing_attrs_text_before_brace() {
        let mut children = vec![
            Node::emphasis(vec![Node::text("em")]),
            Node::text("text {.cls}"),
        ];
        maybe_extract_trailing_attrs_from_children(&mut children);
        // Should split: keep "text" node and apply .cls to emphasis
        assert_eq!(children.len(), 2);
        assert!(children[0].attrs.classes.contains(&"cls".to_string()));
    }

    #[test]
    fn trailing_attrs_single_child_pure_brace() {
        let mut children = vec![Node::text("{.cls}")];
        maybe_extract_trailing_attrs_from_children(&mut children);
        // Single child with only braces — pop it, but no previous node to apply to
        assert!(children.is_empty());
    }

    // ── parse_hint_value ──

    #[test]
    fn hint_value_true() {
        assert!(matches!(parse_hint_value("true"), Value::Bool(true)));
    }

    #[test]
    fn hint_value_false() {
        assert!(matches!(parse_hint_value("false"), Value::Bool(false)));
    }

    #[test]
    fn hint_value_number() {
        assert!(matches!(parse_hint_value("42"), Value::Number(n) if n == 42.0));
    }

    #[test]
    fn hint_value_float() {
        assert!(matches!(parse_hint_value("2.5"), Value::Number(n) if (n - 2.5).abs() < 0.01));
    }

    #[test]
    fn hint_value_text() {
        assert!(matches!(parse_hint_value("hello"), Value::Text(ref s) if s == "hello"));
    }

    #[test]
    fn hint_value_quoted() {
        assert!(matches!(parse_hint_value("\"hello\""), Value::Text(ref s) if s == "hello"));
    }

    // ── is_property_token ──

    #[test]
    fn property_token_key_value() {
        assert!(is_property_token("key=value"));
    }

    #[test]
    fn property_token_quoted_value() {
        assert!(is_property_token("key=\"value\""));
    }

    #[test]
    fn property_token_bare_word_not_property() {
        assert!(!is_property_token("figure"));
    }

    #[test]
    fn property_token_class_not_property() {
        assert!(!is_property_token(".wide"));
    }

    #[test]
    fn property_token_id_not_property() {
        assert!(!is_property_token("#my-id"));
    }

    #[test]
    fn property_token_bracket_not_property() {
        assert!(!is_property_token("[attr]"));
    }

    #[test]
    fn property_token_this_not_property() {
        assert!(!is_property_token("this"));
    }

    #[test]
    fn property_token_empty_not_property() {
        assert!(!is_property_token(""));
    }

    #[test]
    fn property_token_nested_eq() {
        // Equals inside brackets should not be considered top-level
        assert!(!is_property_token("[lang=\"rust\"]"));
    }

    // ── split_top_level_hint_commas ──

    #[test]
    fn split_commas_simple() {
        let parts = split_top_level_hint_commas("a, b, c");
        assert_eq!(parts, vec!["a", " b", " c"]);
    }

    #[test]
    fn split_commas_nested_brackets() {
        let parts = split_top_level_hint_commas("sel[a,b], key=val");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], "sel[a,b]");
    }

    #[test]
    fn split_commas_quoted() {
        let parts = split_top_level_hint_commas("key=\"a,b\", other=c");
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn split_commas_no_comma() {
        let parts = split_top_level_hint_commas("single");
        assert_eq!(parts, vec!["single"]);
    }

    // ── find_top_level_eq ──

    #[test]
    fn top_level_eq_simple() {
        assert_eq!(find_top_level_eq("key=val"), Some(3));
    }

    #[test]
    fn top_level_eq_in_quotes() {
        assert!(find_top_level_eq("\"a=b\"").is_none());
    }

    #[test]
    fn top_level_eq_in_brackets() {
        assert!(find_top_level_eq("[a=b]").is_none());
    }

    #[test]
    fn top_level_eq_in_parens() {
        assert!(find_top_level_eq("(a=b)").is_none());
    }

    #[test]
    fn top_level_eq_none() {
        assert!(find_top_level_eq("bareword").is_none());
    }

    // ── find_matching_open_brace ──

    #[test]
    fn matching_brace_simple() {
        assert_eq!(find_matching_open_brace("{content}"), Some(0));
    }

    #[test]
    fn matching_brace_with_prefix() {
        assert_eq!(find_matching_open_brace("text {content}"), Some(5));
    }

    #[test]
    fn matching_brace_nested() {
        assert_eq!(find_matching_open_brace("{outer {inner}}"), Some(0));
    }

    #[test]
    fn matching_brace_no_close() {
        assert!(find_matching_open_brace("no brace").is_none());
    }

    #[test]
    fn matching_brace_unbalanced() {
        // More } than {
        assert!(find_matching_open_brace("extra}").is_none());
    }

    #[test]
    fn matching_brace_empty() {
        assert!(find_matching_open_brace("").is_none());
    }

    // ── split_directive_args ──

    #[test]
    fn directive_args_simple() {
        let parts = split_directive_args("fig, counts=figure");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], ("fig".to_string(), "fig".to_string()));
        assert_eq!(parts[1].0, "counts");
        assert_eq!(parts[1].1, "figure");
    }

    #[test]
    fn directive_args_with_formats() {
        let parts = split_directive_args("sec, formats=[\"I\",\"1\"]");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].0, "formats");
    }

    #[test]
    fn directive_args_empty() {
        let parts = split_directive_args("");
        assert!(parts.is_empty());
    }

    // ── parse_format_list ──

    #[test]
    fn format_list_bracket_syntax() {
        let list = parse_format_list("[\"I\",\"1\",\"a\"]");
        assert_eq!(list, vec!["I", "1", "a"]);
    }

    #[test]
    fn format_list_single_token() {
        let list = parse_format_list("I");
        assert_eq!(list, vec!["I"]);
    }

    #[test]
    fn format_list_empty_brackets() {
        let list = parse_format_list("[]");
        assert!(list.is_empty());
    }

    // ── parse_hint_directive ──

    #[test]
    fn hint_directive_simple() {
        let hints = parse_hint_directive("figure, series=figs");
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "series");
    }

    #[test]
    fn hint_directive_multiple_props() {
        let hints = parse_hint_directive("figure, series=figs, label=\"Figure {n}\"");
        assert_eq!(hints.len(), 2);
    }

    #[test]
    fn hint_directive_no_props() {
        let hints = parse_hint_directive("figure");
        assert!(hints.is_empty());
    }

    #[test]
    fn hint_directive_only_selector_parts() {
        let hints = parse_hint_directive("figure.wide");
        assert!(hints.is_empty());
    }

    #[test]
    fn hint_directive_with_attr_selector() {
        let hints = parse_hint_directive("code-block[lang=\"rust\"], theme=dark");
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "theme");
    }

    // ── scan_delimited_token ──

    #[test]
    fn scan_delimited_stops_at_comma() {
        let chars: Vec<char> = "abc,def".chars().collect();
        assert_eq!(scan_delimited_token(&chars, 0), 3);
    }

    #[test]
    fn scan_delimited_skips_quoted_comma() {
        let chars: Vec<char> = "\"a,b\",c".chars().collect();
        assert_eq!(scan_delimited_token(&chars, 0), 5);
    }

    #[test]
    fn scan_delimited_skips_paren_comma() {
        let chars: Vec<char> = "(a,b),c".chars().collect();
        assert_eq!(scan_delimited_token(&chars, 0), 5);
    }

    #[test]
    fn scan_delimited_skips_bracket_comma() {
        let chars: Vec<char> = "[a,b],c".chars().collect();
        assert_eq!(scan_delimited_token(&chars, 0), 5);
    }

    // ── split_kv ──

    #[test]
    fn split_kv_with_eq() {
        let (k, v) = split_kv("key=val");
        assert_eq!(k, "key");
        assert_eq!(v, "val");
    }

    #[test]
    fn split_kv_no_eq() {
        let (k, v) = split_kv("bare");
        assert_eq!(k, "bare");
        assert_eq!(v, "bare");
    }

    #[test]
    fn split_kv_quoted() {
        let (k, v) = split_kv("key=\"val\"");
        assert_eq!(k, "key");
        assert_eq!(v, "val");
    }

    // ── parse_series_directive_full ──

    #[test]
    fn series_full_basic() {
        let result = parse_series_directive_full("fig");
        assert!(result.is_some());
        let (decl, hints) = result.unwrap();
        let Declaration::Series { id, config } = decl;
        assert_eq!(id, "fig");
        assert_eq!(config.formats, vec!["1"]);
        assert!(config.parent.is_none());
        assert!(!config.cycle);
        assert!(hints.is_empty());
    }

    #[test]
    fn series_full_with_counts() {
        let result = parse_series_directive_full("fig, counts=figure");
        assert!(result.is_some());
        let (_, hints) = result.unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "series");
    }

    #[test]
    fn series_full_with_all_params() {
        let result = parse_series_directive_full("fig, parent=sec, formats=[\"I\",\"1\"], cycle=true, counts=figure");
        assert!(result.is_some());
        let (decl, hints) = result.unwrap();
        let Declaration::Series { config, .. } = decl;
        assert_eq!(config.parent.as_deref(), Some("sec"));
        assert_eq!(config.formats, vec!["I", "1"]);
        assert!(config.cycle);
        assert_eq!(hints.len(), 1);
    }

    #[test]
    fn series_full_empty() {
        let result = parse_series_directive_full("");
        assert!(result.is_none());
    }

    // ── Unknown directive ──

    #[test]
    fn unknown_directive_ignored() {
        let mut attrs = Attrs::default();
        apply_directive(&mut attrs, "unknown_cmd", "arg1, arg2");
        assert!(attrs.declarations.is_empty());
        assert!(attrs.hints.is_empty());
    }



    // ── parse_attr_directive edge cases ──

    #[test]
    fn parse_attr_directive_no_parens() {
        let mut attrs = Attrs::default();
        let chars: Vec<char> = "@noparen rest".chars().collect();
        let consumed = parse_attr_directive(&chars, 0, &mut attrs);
        // Should skip the @noparen token
        assert!(consumed > 0);
    }



    // ── parse_hint_directive edge cases ──

    #[test]
    fn parse_hint_directive_empty_args() {
        let result = parse_hint_directive("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_hint_directive_no_properties() {
        // Just a selector with no property assignments
        let result = parse_hint_directive("figure");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_series_with_counts_and_formats() {
        let mut attrs = Attrs::default();
        apply_directive(&mut attrs, "series", "tbl, counts=table, formats=[\"I\",\"a\"]");
        assert!(!attrs.declarations.is_empty());
        assert!(!attrs.hints.is_empty());
        // Check that the series hint was generated for table selector
        let has_series_hint = attrs.hints.iter().any(|h| h.property == "series");
        assert!(has_series_hint, "should have series hint from counts=");
    }

    // ── is_property_token edge cases ──

    #[test]
    fn is_property_token_hyphenated_key() {
        assert!(is_property_token("my-key=value"));
    }
}
