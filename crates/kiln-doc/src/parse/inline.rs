use crate::ast::Node;

/// Parse inline markdown text into a list of inline Nodes.
pub(super) fn parse_inline(text: &str) -> Vec<Node> {
    let mut nodes = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut pos = 0;
    let mut current_text = String::new();

    while pos < chars.len() {
        let ch = chars[pos];

        if let Some((escaped, end)) = try_scan_escape(&chars, pos) {
            current_text.push(escaped);
            pos = end;
            continue;
        }

        if let Some((node, end)) = try_scan_display_math(&chars, pos) {
            flush_text(&mut current_text, &mut nodes);
            nodes.push(node);
            pos = end;
            continue;
        }

        if let Some((node, end)) = try_scan_inline_math(&chars, pos) {
            flush_text(&mut current_text, &mut nodes);
            nodes.push(node);
            pos = end;
            continue;
        }

        if let Some((node, end)) = try_scan_code_span(&chars, pos) {
            flush_text(&mut current_text, &mut nodes);
            nodes.push(node);
            pos = end;
            continue;
        }

        if let Some((node, end)) = try_scan_image(&chars, pos) {
            flush_text(&mut current_text, &mut nodes);
            nodes.push(node);
            pos = end;
            continue;
        }

        if let Some((new_nodes, end)) = try_scan_bracket(&chars, pos) {
            flush_text(&mut current_text, &mut nodes);
            nodes.extend(new_nodes);
            pos = end;
            continue;
        }

        if let Some((node, end)) = try_scan_cross_ref_inline(&chars, pos) {
            flush_text(&mut current_text, &mut nodes);
            nodes.push(node);
            pos = end;
            continue;
        }

        if let Some((node, end)) = try_scan_strikethrough(&chars, pos) {
            flush_text(&mut current_text, &mut nodes);
            nodes.push(node);
            pos = end;
            continue;
        }

        if let Some((node, end)) = try_scan_emphasis_like(&chars, pos, ch) {
            flush_text(&mut current_text, &mut nodes);
            nodes.push(node);
            pos = end;
            continue;
        }

        if ch == '\n' {
            handle_newline_break(&mut current_text, &mut nodes);
            pos += 1;
            continue;
        }

        current_text.push(ch);
        pos += 1;
    }

    flush_text(&mut current_text, &mut nodes);
    nodes
}

fn try_scan_escape(chars: &[char], pos: usize) -> Option<(char, usize)> {
    if chars[pos] != '\\' || pos + 1 >= chars.len() {
        return None;
    }
    let next = chars[pos + 1];
    if is_escapable(next) {
        Some((next, pos + 2))
    } else {
        None
    }
}

fn try_scan_display_math(chars: &[char], pos: usize) -> Option<(Node, usize)> {
    if chars[pos] != '$' || pos + 1 >= chars.len() || chars[pos + 1] != '$' {
        return None;
    }
    let (math_src, end) = scan_delimited(chars, pos + 2, &['$', '$'])?;
    Some((Node::math(true, math_src), end))
}

fn try_scan_inline_math(chars: &[char], pos: usize) -> Option<(Node, usize)> {
    if chars[pos] != '$' {
        return None;
    }
    let (math_src, end) = scan_single_delimited(chars, pos + 1, '$')?;
    if math_src.is_empty() || math_src.starts_with(' ') && math_src.ends_with(' ') {
        return None;
    }
    Some((Node::math(false, math_src), end))
}

fn try_scan_code_span(chars: &[char], pos: usize) -> Option<(Node, usize)> {
    if chars[pos] != '`' {
        return None;
    }
    let backtick_count = chars[pos..].iter().take_while(|&&c| c == '`').count();
    let (code_text, end) = scan_backtick_span(chars, pos + backtick_count, backtick_count)?;
    Some((Node::code(code_text), end))
}

fn try_scan_image(chars: &[char], pos: usize) -> Option<(Node, usize)> {
    if chars[pos] != '!' || pos + 1 >= chars.len() || chars[pos + 1] != '[' {
        return None;
    }
    let (alt, url, end) = scan_image_or_link(chars, pos + 1)?;
    Some((Node::image(url, alt), end))
}

fn try_scan_bracket(chars: &[char], pos: usize) -> Option<(Vec<Node>, usize)> {
    if chars[pos] != '[' {
        return None;
    }
    // Citation: [@key] or [@key1; @key2]
    if pos + 1 < chars.len()
        && chars[pos + 1] == '@'
        && let Some((keys, end)) = scan_citation(chars, pos)
    {
        let nodes = keys.into_iter().map(Node::citation).collect();
        return Some((nodes, end));
    }
    // Footnote reference: [^label]
    if let Some((label, end)) = scan_footnote_ref(chars, pos) {
        return Some((vec![Node::footnote_ref(label)], end));
    }
    // Link: [text](url)
    let (link_text, url, end) = scan_image_or_link(chars, pos)?;
    let children = parse_inline(&link_text);
    Some((vec![Node::link(url, children)], end))
}

/// Scan a footnote reference, `[^label]`, with no whitespace in the label.
fn scan_footnote_ref(chars: &[char], pos: usize) -> Option<(String, usize)> {
    if chars.get(pos + 1) != Some(&'^') {
        return None;
    }
    let len = chars[pos + 2..].iter().position(|&c| c == ']' || c.is_whitespace())?;
    if len == 0 || chars[pos + 2 + len] != ']' {
        return None;
    }
    let label = chars[pos + 2..pos + 2 + len].iter().collect();
    Some((label, pos + 3 + len))
}

fn try_scan_cross_ref_inline(chars: &[char], pos: usize) -> Option<(Node, usize)> {
    if chars[pos] != '@' {
        return None;
    }
    let (target, end) = scan_cross_ref(chars, pos)?;
    Some((Node::cross_ref(target), end))
}

fn try_scan_strikethrough(chars: &[char], pos: usize) -> Option<(Node, usize)> {
    if chars[pos] != '~' || pos + 1 >= chars.len() || chars[pos + 1] != '~' {
        return None;
    }
    let (inner, end) = scan_delimited(chars, pos + 2, &['~', '~'])?;
    let children = parse_inline(&inner);
    Some((Node::strikethrough(children), end))
}

fn try_scan_emphasis_like(chars: &[char], pos: usize, ch: char) -> Option<(Node, usize)> {
    use crate::ast::NodeKind;

    if ch != '*' && ch != '_' {
        return None;
    }
    // Strong + emphasis: ***...*** or ___...___
    if pos + 2 < chars.len()
        && chars[pos + 1] == ch
        && chars[pos + 2] == ch
        && let Some((inner, end)) = scan_delimited(chars, pos + 3, &[ch, ch, ch])
    {
        let children = parse_inline(&inner);
        let node = Node::strong(vec![Node::new(NodeKind::Emphasis).with_children(children)]);
        return Some((node, end));
    }
    // Strong: **...**  or __...__
    if pos + 1 < chars.len()
        && chars[pos + 1] == ch
        && let Some((inner, end)) = scan_delimited(chars, pos + 2, &[ch, ch])
    {
        let children = parse_inline(&inner);
        return Some((Node::strong(children), end));
    }
    // Emphasis: *...* or _..._
    let (inner, end) = scan_single_delimited(chars, pos + 1, ch)?;
    if inner.is_empty() {
        return None;
    }
    let children = parse_inline(&inner);
    Some((Node::emphasis(children), end))
}

fn handle_newline_break(current_text: &mut String, nodes: &mut Vec<Node>) {
    if current_text.ends_with("  ") {
        current_text.truncate(current_text.trim_end().len());
        flush_text(current_text, nodes);
        nodes.push(Node::hard_break());
    } else if current_text.ends_with('\\') {
        current_text.pop();
        flush_text(current_text, nodes);
        nodes.push(Node::hard_break());
    } else {
        flush_text(current_text, nodes);
        nodes.push(Node::soft_break());
    }
}

fn is_escapable(ch: char) -> bool {
    matches!(
        ch,
        '\\' | '`'
            | '*'
            | '_'
            | '{'
            | '}'
            | '['
            | ']'
            | '('
            | ')'
            | '#'
            | '+'
            | '-'
            | '.'
            | '!'
            | '|'
            | '~'
            | '@'
            | '$'
    )
}

fn flush_text(buf: &mut String, nodes: &mut Vec<Node>) {
    if !buf.is_empty() {
        nodes.push(Node::text(std::mem::take(buf)));
    }
}

/// Scan for content between matching delimiter sequences.
/// Returns (content_string, position_after_closing_delimiter).
fn scan_delimited(chars: &[char], start: usize, closing: &[char]) -> Option<(String, usize)> {
    let clen = closing.len();
    let mut pos = start;
    while pos + clen <= chars.len() {
        if &chars[pos..pos + clen] == closing {
            let content: String = chars[start..pos].iter().collect();
            return Some((content, pos + clen));
        }
        pos += 1;
    }
    None
}

/// Scan for a single closing delimiter char. Returns (content, pos_after).
fn scan_single_delimited(chars: &[char], start: usize, closing: char) -> Option<(String, usize)> {
    let mut pos = start;
    while pos < chars.len() {
        if chars[pos] == closing {
            let content: String = chars[start..pos].iter().collect();
            return Some((content, pos + 1));
        }
        // Don't cross newlines for inline delimiters
        if chars[pos] == '\n' && closing != '$' {
            return None;
        }
        pos += 1;
    }
    None
}

/// Scan backtick spans: find N backticks that close the span.
fn strip_code_spaces(s: String) -> String {
    if s.starts_with(' ') && s.ends_with(' ') && s.len() > 1 {
        s[1..s.len() - 1].to_string()
    } else {
        s
    }
}

fn scan_backtick_span(chars: &[char], start: usize, count: usize) -> Option<(String, usize)> {
    let mut pos = start;
    while pos < chars.len() {
        if chars[pos] != '`' {
            pos += 1;
            continue;
        }
        let bt_count = chars[pos..].iter().take_while(|&&c| c == '`').count();
        if bt_count != count {
            pos += bt_count;
            continue;
        }
        let content: String = chars[start..pos].iter().collect();
        return Some((strip_code_spaces(content), pos + count));
    }
    None
}

/// Scan [text](url) or ![alt](url). Returns (text/alt, url, pos_after).
pub(super) fn find_matching_delim(
    chars: &[char],
    start: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_quotes = false;
    for (i, &ch) in chars[start..].iter().enumerate() {
        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if in_quotes {
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
        }
        if depth == 0 {
            return Some(start + i);
        }
    }
    None
}

fn scan_image_or_link(chars: &[char], bracket_pos: usize) -> Option<(String, String, usize)> {
    if chars[bracket_pos] != '[' {
        return None;
    }
    let close_bracket = find_matching_delim(chars, bracket_pos, '[', ']')?;
    if close_bracket + 1 >= chars.len() || chars[close_bracket + 1] != '(' {
        return None;
    }
    let url_start = close_bracket + 2;
    let close_paren = find_matching_delim(chars, close_bracket + 1, '(', ')')?;
    let text: String = chars[bracket_pos + 1..close_bracket].iter().collect();
    let url: String = chars[url_start..close_paren].iter().collect();
    let url = url.trim().to_string();
    Some((text, url, close_paren + 1))
}

/// Scan a citation: [@key] or [@key1; @key2; ...]
fn scan_citation(chars: &[char], start: usize) -> Option<(Vec<String>, usize)> {
    // start points to '['
    if chars[start] != '[' || start + 1 >= chars.len() || chars[start + 1] != '@' {
        return None;
    }
    // Find closing ]
    let mut pos = start + 1;
    while pos < chars.len() && chars[pos] != ']' {
        if chars[pos] == '\n' {
            return None;
        }
        pos += 1;
    }
    if pos >= chars.len() {
        return None;
    }
    let inner: String = chars[start + 1..pos].iter().collect();
    let keys: Vec<String> = inner
        .split(';')
        .map(|s| s.trim().trim_start_matches('@').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if keys.is_empty() {
        return None;
    }
    Some((keys, pos + 1))
}

/// Scan a cross-reference: @kind:label
/// Ends at first non-alphanumeric, non-hyphen, non-underscore, non-colon character.
fn scan_cross_ref(chars: &[char], start: usize) -> Option<(String, usize)> {
    // start points to '@'
    if start + 1 >= chars.len() {
        return None;
    }
    let mut pos = start + 1;
    let mut has_colon = false;
    while pos < chars.len() {
        let ch = chars[pos];
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            pos += 1;
        } else if ch == ':' {
            has_colon = true;
            pos += 1;
        } else {
            break;
        }
    }
    // Must have at least one colon and some content after @
    if !has_colon || pos == start + 1 {
        return None;
    }
    let target: String = chars[start + 1..pos].iter().collect();
    Some((target, pos))
}
