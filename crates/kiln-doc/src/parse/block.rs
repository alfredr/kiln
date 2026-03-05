use super::attrs::extract_trailing_brace_attrs;

/// Intermediate block before inline parsing.
#[derive(Debug)]
pub(super) enum RawBlock {
    Heading {
        level: u8,
        text: String,
        attrs: Option<String>,
    },
    Paragraph(String),
    CodeBlock {
        source: String,
        attrs: Option<String>,
    },
    BlockQuote(Vec<RawBlock>),
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<Vec<RawBlock>>,
    },
    ThematicBreak,
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// A standalone directive line like `{@var(...)}`
    Directive(String),
}

pub(super) fn parse_blocks(lines: &[&str]) -> Vec<RawBlock> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // Skip blank lines
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // Thematic break: ---, ***, ___  (3+ of same char, optionally with spaces)
        if is_thematic_break(line) {
            blocks.push(RawBlock::ThematicBreak);
            i += 1;
            continue;
        }

        // ATX heading: # ... ######
        if let Some(heading) = try_parse_heading(line) {
            blocks.push(heading);
            i += 1;
            continue;
        }

        // Fenced code block: ``` or ~~~
        if let Some((block, consumed)) = try_parse_fenced_code(&lines[i..]) {
            blocks.push(block);
            i += consumed;
            continue;
        }

        // Block quote: > ...
        if line.trim_start().starts_with("> ") || line.trim_start() == ">" {
            let (bq, consumed) = parse_block_quote(&lines[i..]);
            blocks.push(bq);
            i += consumed;
            continue;
        }

        // List: - ... or * ... or + ... or 1. ... or 1) ...
        if let Some((list, consumed)) = try_parse_list(&lines[i..]) {
            blocks.push(list);
            i += consumed;
            continue;
        }

        // Table: | ... | ... |
        if let Some((table, consumed)) = try_parse_table(&lines[i..]) {
            blocks.push(table);
            i += consumed;
            continue;
        }

        // Standalone directive: {@ ... }
        let trimmed = line.trim();
        if trimmed.starts_with("{@") && trimmed.ends_with('}') {
            blocks.push(RawBlock::Directive(trimmed.to_string()));
            i += 1;
            continue;
        }

        // Paragraph: collect lines until blank line or block-level construct
        let (para, consumed) = parse_paragraph(&lines[i..]);
        blocks.push(para);
        i += consumed;
    }
    blocks
}

fn is_thematic_break(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    let ch = trimmed.as_bytes()[0];
    if ch != b'-' && ch != b'*' && ch != b'_' {
        return false;
    }
    trimmed.bytes().all(|b| b == ch || b == b' ')
}

/// Parse a GFM-style table. Requires:
/// 1. A header row with pipes: `| Col1 | Col2 |`
/// 2. A separator row with pipes and dashes: `|------|------|`
/// 3. Zero or more data rows with pipes
fn try_parse_table(lines: &[&str]) -> Option<(RawBlock, usize)> {
    if lines.len() < 2 {
        return None;
    }
    let header_line = lines[0].trim();
    let sep_line = lines[1].trim();

    // Both lines must contain pipes
    if !header_line.contains('|') || !sep_line.contains('|') {
        return None;
    }

    // Separator must be pipes, dashes, colons, and spaces only
    if !sep_line
        .bytes()
        .all(|b| b == b'|' || b == b'-' || b == b':' || b == b' ')
    {
        return None;
    }

    // Separator must have at least one segment of dashes
    let sep_cells = split_table_row(sep_line);
    if sep_cells.is_empty() {
        return None;
    }
    if !sep_cells.iter().all(|c| {
        let t = c.trim().trim_matches(':');
        !t.is_empty() && t.bytes().all(|b| b == b'-')
    }) {
        return None;
    }

    let header = split_table_row(header_line);
    if header.is_empty() {
        return None;
    }

    let mut rows = Vec::new();
    let mut i = 2;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.is_empty() || !line.contains('|') {
            break;
        }
        rows.push(split_table_row(line));
        i += 1;
    }

    Some((RawBlock::Table { header, rows }, i))
}

/// Split a table row like `| a | b | c |` into cells `["a", "b", "c"]`.
///
/// Handles escaped pipes (`\|`) and pipes inside backtick code spans —
/// neither is treated as a cell delimiter.
fn split_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    // Strip leading and trailing pipes
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);

    let mut cells = Vec::new();
    let mut current = String::new();
    let mut in_code = false;
    let mut prev_backslash = false;

    for ch in inner.chars() {
        if ch == '`' && !prev_backslash {
            in_code = !in_code;
            current.push(ch);
            prev_backslash = false;
        } else if ch == '|' && !in_code && !prev_backslash {
            cells.push(current.trim().to_string());
            current = String::new();
            prev_backslash = false;
        } else {
            prev_backslash = ch == '\\';
            current.push(ch);
        }
    }
    cells.push(current.trim().to_string());
    cells
}

fn try_parse_heading(line: &str) -> Option<RawBlock> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.bytes().take_while(|&b| b == b'#').count();
    if level > 6 || level == 0 {
        return None;
    }
    let rest = &trimmed[level..];
    // Must be followed by space or end of line
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim();
    // Strip trailing # sequences
    let text = text.trim_end_matches('#').trim_end();
    // Check for trailing {#id .class ...} attributes
    let (text, attrs) = extract_trailing_brace_attrs(text);
    Some(RawBlock::Heading {
        level: level as u8,
        text: text.to_string(),
        attrs,
    })
}

fn try_parse_fenced_code(lines: &[&str]) -> Option<(RawBlock, usize)> {
    let first = lines[0].trim_start();
    let indent = lines[0].len() - first.len();
    let (fence_char, fence_len) = if first.starts_with("```") {
        ('`', first.bytes().take_while(|&b| b == b'`').count())
    } else if first.starts_with("~~~") {
        ('~', first.bytes().take_while(|&b| b == b'~').count())
    } else {
        return None;
    };
    if fence_len < 3 {
        return None;
    }
    let info_string = first[fence_len..].trim();
    // Normalize info string into a single attrs value.
    // All forms collapse into attrs: `rust {#id}` → "rust #id",
    // `{=typst}` → "=typst", bare `rust` → "rust", empty → None.
    let attrs = if let Some(brace_start) = info_string.find('{') {
        let prefix = info_string[..brace_start].trim();
        let brace_end = info_string.rfind('}')?;
        let brace_content = &info_string[brace_start + 1..brace_end];
        Some(if prefix.is_empty() {
            brace_content.to_string()
        } else {
            format!("{prefix} {brace_content}")
        })
    } else if info_string.is_empty() {
        None
    } else {
        Some(info_string.to_string())
    };

    // Collect lines until closing fence
    let mut source = String::new();
    let mut i = 1;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let line_indent = line.len() - trimmed.len();
        // Closing fence: same char, at least as many, no other content
        if line_indent <= indent + 3
            && trimmed
                .bytes()
                .take_while(|&b| b == fence_char as u8)
                .count()
                >= fence_len
            && trimmed.trim_end().bytes().all(|b| b == fence_char as u8)
        {
            i += 1;
            break;
        }
        if !source.is_empty() {
            source.push('\n');
        }
        source.push_str(line);
        i += 1;
    }
    Some((RawBlock::CodeBlock { source, attrs }, i))
}

fn parse_block_quote(lines: &[&str]) -> (RawBlock, usize) {
    let mut inner_lines = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if let Some(stripped) = trimmed.strip_prefix("> ") {
            inner_lines.push(stripped);
        } else if trimmed == ">" {
            inner_lines.push("");
        } else if trimmed.is_empty() {
            break;
        } else {
            // Lazy continuation
            inner_lines.push(lines[i]);
        }
        i += 1;
    }
    let inner_strs: Vec<&str> = inner_lines.to_vec();
    let blocks = parse_blocks(&inner_strs);
    (RawBlock::BlockQuote(blocks), i)
}

fn list_blank_line_continues(
    lines: &[&str],
    i: usize,
    base_indent: usize,
    content_indent: usize,
    ordered: bool,
) -> bool {
    let mut j = i + 1;
    while j < lines.len() && lines[j].trim().is_empty() {
        j += 1;
    }
    if j >= lines.len() {
        return false;
    }
    let next_trimmed = lines[j].trim_start();
    let next_indent = lines[j].len() - next_trimmed.len();
    if next_indent <= base_indent + 1
        && detect_list_marker_compatible(next_trimmed, ordered).is_some()
    {
        return true;
    }
    next_indent >= content_indent
}

fn try_parse_list(lines: &[&str]) -> Option<(RawBlock, usize)> {
    let first = lines[0];
    let first_trimmed = first.trim_start();
    let base_indent = first.len() - first_trimmed.len();

    let (ordered, start_num, marker_end) = detect_list_marker(first_trimmed)?;

    let mut items: Vec<Vec<&str>> = Vec::new();
    let mut current_item: Vec<&str> = vec![&first_trimmed[marker_end..]];
    let content_indent = base_indent + marker_end;
    let mut i = 1;

    while i < lines.len() {
        let line = lines[i];
        let is_blank = line.trim().is_empty();
        if is_blank && !list_blank_line_continues(lines, i, base_indent, content_indent, ordered) {
            break;
        }
        if is_blank {
            current_item.push("");
            i += 1;
            continue;
        }
        let line_trimmed = line.trim_start();
        let line_indent = line.len() - line_trimmed.len();
        // New item at same level?
        if line_indent <= base_indent + 1
            && let Some((_, _, me)) = detect_list_marker_compatible(line_trimmed, ordered)
        {
            items.push(std::mem::take(&mut current_item));
            current_item.push(&line_trimmed[me..]);
            i += 1;
            continue;
        }
        // Continuation line (indented)
        if line_indent >= content_indent {
            current_item.push(&line[content_indent..]);
        } else if line_indent > base_indent {
            current_item.push(line_trimmed);
        } else {
            // Not indented enough — end of list
            break;
        }
        i += 1;
    }
    items.push(current_item);

    // Parse each item's content as blocks
    let parsed_items: Vec<Vec<RawBlock>> = items
        .into_iter()
        .map(|item_lines| {
            let strs: Vec<&str> = item_lines.to_vec();
            parse_blocks(&strs)
        })
        .collect();

    Some((
        RawBlock::List {
            ordered,
            start: if ordered { Some(start_num) } else { None },
            items: parsed_items,
        },
        i,
    ))
}

/// Detect a list marker. Returns (ordered, start_number, chars_consumed_including_space).
fn detect_list_marker(line: &str) -> Option<(bool, u64, usize)> {
    // Unordered: - or * or + followed by space
    if (line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ "))
        && !is_thematic_break(line)
    {
        return Some((false, 0, 2));
    }
    // Ordered: digits followed by . or ) then space
    let digit_count = line.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digit_count > 0 && digit_count <= 9 {
        let after = line.as_bytes().get(digit_count)?;
        if (*after == b'.' || *after == b')') && line.as_bytes().get(digit_count + 1) == Some(&b' ')
        {
            let num: u64 = line[..digit_count].parse().ok()?;
            return Some((true, num, digit_count + 2));
        }
    }
    None
}

fn detect_list_marker_compatible(line: &str, ordered: bool) -> Option<(bool, u64, usize)> {
    let result = detect_list_marker(line)?;
    if result.0 == ordered {
        Some(result)
    } else {
        None
    }
}

fn is_table_header_start(line: &str, next_line: Option<&str>) -> bool {
    if !line.contains('|') {
        return false;
    }
    let Some(next) = next_line else {
        return false;
    };
    let next = next.trim();
    next.contains('|')
        && next
            .bytes()
            .all(|b| b == b'|' || b == b'-' || b == b':' || b == b' ')
}

fn parse_paragraph(lines: &[&str]) -> (RawBlock, usize) {
    let mut text = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            break;
        }
        // Stop if next line is a block-level construct
        if is_thematic_break(line) && i > 0 {
            break;
        }
        if try_parse_heading(line).is_some() && i > 0 {
            break;
        }
        let trimmed = line.trim_start();
        if (trimmed.starts_with("```") || trimmed.starts_with("~~~")) && i > 0 {
            break;
        }
        if (trimmed.starts_with("> ") || trimmed == ">") && i > 0 {
            break;
        }
        if detect_list_marker(trimmed).is_some() && i > 0 {
            break;
        }
        // Standalone directive
        let t = line.trim();
        if t.starts_with("{@") && t.ends_with('}') && i > 0 {
            break;
        }
        // If this line looks like a table header, stop the paragraph
        if i > 0 && is_table_header_start(line, lines.get(i + 1).copied()) {
            break;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line);
        i += 1;
    }
    (RawBlock::Paragraph(text), i.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_thematic_break ──

    #[test]
    fn thematic_break_dashes() {
        assert!(is_thematic_break("---"));
        assert!(is_thematic_break("----"));
        assert!(is_thematic_break("- - -"));
        assert!(is_thematic_break("  ---  "));
    }

    #[test]
    fn thematic_break_stars() {
        assert!(is_thematic_break("***"));
        assert!(is_thematic_break("****"));
        assert!(is_thematic_break("* * *"));
    }

    #[test]
    fn thematic_break_underscores() {
        assert!(is_thematic_break("___"));
        assert!(is_thematic_break("____"));
        assert!(is_thematic_break("_ _ _"));
    }

    #[test]
    fn not_thematic_break_too_short() {
        assert!(!is_thematic_break("--"));
        assert!(!is_thematic_break("**"));
        assert!(!is_thematic_break("__"));
        assert!(!is_thematic_break("-"));
    }

    #[test]
    fn not_thematic_break_mixed() {
        assert!(!is_thematic_break("-*-"));
        assert!(!is_thematic_break("--_"));
    }

    // ── try_parse_heading ──

    #[test]
    fn heading_level_1() {
        let block = try_parse_heading("# Title").unwrap();
        assert!(matches!(block, RawBlock::Heading { level: 1, .. }));
    }

    #[test]
    fn heading_level_6() {
        let block = try_parse_heading("###### Deep").unwrap();
        assert!(matches!(block, RawBlock::Heading { level: 6, .. }));
    }

    #[test]
    fn heading_too_many_hashes() {
        assert!(try_parse_heading("####### Not a heading").is_none());
    }

    #[test]
    fn heading_no_space_after_hash() {
        assert!(try_parse_heading("#NoSpace").is_none());
    }

    #[test]
    fn heading_empty_after_hash() {
        let block = try_parse_heading("# ").unwrap();
        assert!(matches!(block, RawBlock::Heading { level: 1, .. }));
    }

    #[test]
    fn heading_with_trailing_hashes() {
        let block = try_parse_heading("## Title ##").unwrap();
        if let RawBlock::Heading { text, .. } = block {
            assert_eq!(text, "Title");
        }
    }

    #[test]
    fn heading_with_attrs() {
        let block = try_parse_heading("# Title {#my-id .intro}").unwrap();
        if let RawBlock::Heading { attrs, .. } = block {
            assert!(attrs.is_some());
            assert!(attrs.unwrap().contains("#my-id"));
        }
    }

    #[test]
    fn not_a_heading() {
        assert!(try_parse_heading("Not a heading").is_none());
    }

    // ── split_table_row ──

    #[test]
    fn table_row_basic() {
        let cells = split_table_row("| a | b | c |");
        assert_eq!(cells, vec!["a", "b", "c"]);
    }

    #[test]
    fn table_row_no_outer_pipes() {
        let cells = split_table_row("a | b | c");
        assert_eq!(cells, vec!["a", "b", "c"]);
    }

    #[test]
    fn table_row_escaped_pipe() {
        let cells = split_table_row("| a\\|b | c |");
        assert_eq!(cells.len(), 2);
    }

    #[test]
    fn table_row_code_span_pipe() {
        let cells = split_table_row("| `a|b` | c |");
        assert_eq!(cells.len(), 2);
    }

    #[test]
    fn table_row_single_cell() {
        let cells = split_table_row("| only |");
        assert_eq!(cells, vec!["only"]);
    }

    // ── try_parse_table ──

    #[test]
    fn table_basic_parse() {
        let lines = vec!["| A | B |", "|---|---|", "| 1 | 2 |"];
        let result = try_parse_table(&lines);
        assert!(result.is_some());
        let (block, consumed) = result.unwrap();
        assert!(matches!(block, RawBlock::Table { .. }));
        assert_eq!(consumed, 3);
    }

    #[test]
    fn table_alignment_separators() {
        let lines = vec!["| L | C | R |", "|:---|:---:|---:|", "| a | b | c |"];
        let result = try_parse_table(&lines);
        assert!(result.is_some());
    }

    #[test]
    fn table_no_body_rows() {
        let lines = vec!["| A | B |", "|---|---|"];
        let result = try_parse_table(&lines);
        assert!(result.is_some());
        if let Some((RawBlock::Table { rows, .. }, _)) = result {
            assert!(rows.is_empty());
        }
    }

    #[test]
    fn table_not_enough_lines() {
        let lines = vec!["| A | B |"];
        assert!(try_parse_table(&lines).is_none());
    }

    #[test]
    fn table_invalid_separator() {
        let lines = vec!["| A | B |", "| not | sep |"];
        assert!(try_parse_table(&lines).is_none());
    }

    #[test]
    fn table_stops_at_blank_line() {
        let lines = vec!["| A |", "|---|", "| 1 |", "", "| 2 |"];
        let result = try_parse_table(&lines).unwrap();
        assert_eq!(result.1, 3); // consumed 3 lines, stopped at blank
    }

    // ── try_parse_fenced_code ──

    #[test]
    fn fenced_code_backticks() {
        let lines = vec!["```rust", "fn main() {}", "```"];
        let result = try_parse_fenced_code(&lines);
        assert!(result.is_some());
        let (block, consumed) = result.unwrap();
        assert!(matches!(block, RawBlock::CodeBlock { .. }));
        assert_eq!(consumed, 3);
    }

    #[test]
    fn fenced_code_tildes() {
        let lines = vec!["~~~python", "print('hi')", "~~~"];
        let result = try_parse_fenced_code(&lines);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, 3);
    }

    #[test]
    fn fenced_code_unclosed() {
        let lines = vec!["```", "code without close"];
        let result = try_parse_fenced_code(&lines);
        assert!(result.is_some());
        let (block, consumed) = result.unwrap();
        assert_eq!(consumed, 2);
        if let RawBlock::CodeBlock { source, .. } = block {
            assert!(source.contains("code without close"));
        }
    }

    #[test]
    fn fenced_code_mismatched_chars() {
        let lines = vec!["```", "code", "~~~"];
        let result = try_parse_fenced_code(&lines);
        assert!(result.is_some());
        // ~~~ shouldn't close a ``` fence
        let (block, consumed) = result.unwrap();
        assert_eq!(consumed, 3); // extends to EOF
        if let RawBlock::CodeBlock { source, .. } = block {
            assert!(source.contains("~~~"));
        }
    }

    #[test]
    fn fenced_code_five_backticks() {
        let lines = vec!["`````", "code", "`````"];
        let result = try_parse_fenced_code(&lines);
        assert!(result.is_some());
    }

    #[test]
    fn fenced_code_with_brace_attrs() {
        let lines = vec!["```{=typst}", "#rect()", "```"];
        let result = try_parse_fenced_code(&lines).unwrap();
        if let (RawBlock::CodeBlock { attrs, .. }, _) = result {
            assert!(attrs.is_some());
            assert!(attrs.unwrap().contains("=typst"));
        }
    }

    #[test]
    fn fenced_code_empty() {
        let lines = vec!["```", "```"];
        let result = try_parse_fenced_code(&lines).unwrap();
        if let (RawBlock::CodeBlock { source, .. }, _) = result {
            assert!(source.is_empty());
        }
    }

    #[test]
    fn not_a_fence() {
        let lines = vec!["``not a fence``"];
        assert!(try_parse_fenced_code(&lines).is_none());
    }

    // ── detect_list_marker ──

    #[test]
    fn list_marker_dash() {
        let result = detect_list_marker("- item");
        assert_eq!(result, Some((false, 0, 2)));
    }

    #[test]
    fn list_marker_ordered_dot() {
        let result = detect_list_marker("1. item");
        assert_eq!(result, Some((true, 1, 3)));
    }

    #[test]
    fn list_marker_ordered_custom_start() {
        let result = detect_list_marker("5. item");
        assert_eq!(result, Some((true, 5, 3)));
    }

    #[test]
    fn list_marker_ordered_large_number() {
        let result = detect_list_marker("999. item");
        assert_eq!(result, Some((true, 999, 5)));
    }

    #[test]
    fn list_marker_too_many_digits() {
        // 10 digits — exceeds 9-digit limit
        let result = detect_list_marker("1234567890. item");
        assert!(result.is_none());
    }

    #[test]
    fn list_marker_thematic_break_not_list() {
        // "---" is a thematic break, not a list marker
        let result = detect_list_marker("---");
        assert!(result.is_none());
    }

    #[test]
    fn list_marker_no_space_after() {
        let result = detect_list_marker("-item");
        assert!(result.is_none());
    }

    #[test]
    fn list_marker_ordered_no_space() {
        let result = detect_list_marker("1.item");
        assert!(result.is_none());
    }

    // ── parse_block_quote ──

    #[test]
    fn blockquote_simple() {
        let lines = vec!["> hello", "> world"];
        let (block, consumed) = parse_block_quote(&lines);
        assert!(matches!(block, RawBlock::BlockQuote(_)));
        assert_eq!(consumed, 2);
    }

    #[test]
    fn blockquote_empty_marker() {
        let lines = vec!["> text", ">", "> more"];
        let (block, consumed) = parse_block_quote(&lines);
        assert!(matches!(block, RawBlock::BlockQuote(_)));
        assert_eq!(consumed, 3);
    }

    #[test]
    fn blockquote_stops_at_blank() {
        let lines = vec!["> text", "", "not quote"];
        let (_, consumed) = parse_block_quote(&lines);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn blockquote_lazy_continuation() {
        let lines = vec!["> first", "lazy line"];
        let (_, consumed) = parse_block_quote(&lines);
        assert_eq!(consumed, 2); // lazy continuation should be included
    }

    // ── parse_blocks (integration) ──

    #[test]
    fn blocks_paragraph() {
        let blocks = parse_blocks(&["Hello world."]);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], RawBlock::Paragraph(_)));
    }

    #[test]
    fn blocks_thematic_break() {
        let blocks = parse_blocks(&["above", "", "---", "", "below"]);
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[1], RawBlock::ThematicBreak));
    }

    // ── is_table_header_start ──

    #[test]
    fn table_header_start_valid() {
        assert!(is_table_header_start("| A | B |", Some("|---|---|")));
    }

    #[test]
    fn table_header_start_no_pipe() {
        assert!(!is_table_header_start("no pipes", Some("|---|---|")));
    }

    #[test]
    fn table_header_start_no_next_line() {
        assert!(!is_table_header_start("| A |", None));
    }

    #[test]
    fn table_header_start_invalid_sep() {
        assert!(!is_table_header_start("| A |", Some("| not sep |")));
    }

    // ── parse_paragraph ──

    #[test]
    fn paragraph_single_line() {
        let (block, consumed) = parse_paragraph(&["Hello."]);
        assert!(matches!(block, RawBlock::Paragraph(_)));
        assert_eq!(consumed, 1);
    }

    #[test]
    fn paragraph_multi_line() {
        let (block, consumed) = parse_paragraph(&["Line 1", "Line 2"]);
        assert!(matches!(block, RawBlock::Paragraph(ref s) if s.contains("Line 1") && s.contains("Line 2")));
        assert_eq!(consumed, 2);
    }

    #[test]
    fn paragraph_stops_at_blank() {
        let (_, consumed) = parse_paragraph(&["Line 1", "", "Line 2"]);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn paragraph_stops_at_heading() {
        let (_, consumed) = parse_paragraph(&["Line 1", "# Heading"]);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn paragraph_stops_at_fence() {
        let (_, consumed) = parse_paragraph(&["Line 1", "```code```"]);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn paragraph_stops_at_blockquote() {
        let (_, consumed) = parse_paragraph(&["Line 1", "> quote"]);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn paragraph_stops_at_list() {
        let (_, consumed) = parse_paragraph(&["Line 1", "- item"]);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn paragraph_stops_at_directive() {
        let (_, consumed) = parse_paragraph(&["Line 1", "{@hint(...)}"]);
        assert_eq!(consumed, 1);
    }

    // ── list_blank_line_continues ──

    #[test]
    fn list_blank_continues_same_indent() {
        let lines = vec!["- item 1", "", "  continued"];
        assert!(list_blank_line_continues(&lines, 1, 0, 2, false));
    }

    #[test]
    fn list_blank_continues_new_item() {
        let lines = vec!["- item 1", "", "- item 2"];
        assert!(list_blank_line_continues(&lines, 1, 0, 2, false));
    }

    #[test]
    fn list_blank_does_not_continue_at_eof() {
        let lines = vec!["- item 1", ""];
        assert!(!list_blank_line_continues(&lines, 1, 0, 2, false));
    }

    // ── parse_blocks comprehensive ──

    #[test]
    fn blocks_all_types() {
        // Exercises every branch of the parse_blocks dispatcher
        let lines = vec![
            "",                  // blank line
            "# Heading",        // heading
            "",
            "```",              // fenced code
            "code here",
            "```",
            "",
            "> quote",          // block quote
            "",
            "- item",           // list
            "",
            "| A | B |",        // table
            "|---|---|",
            "| 1 | 2 |",
            "",
            "{@var(fig, 0)}",   // directive
            "",
            "paragraph text",   // paragraph
        ];
        let blocks = parse_blocks(&lines);
        assert!(matches!(blocks[0], RawBlock::Heading { .. }));
        assert!(matches!(blocks[1], RawBlock::CodeBlock { .. }));
        assert!(matches!(blocks[2], RawBlock::BlockQuote(_)));
        assert!(matches!(blocks[3], RawBlock::List { .. }));
        assert!(matches!(blocks[4], RawBlock::Table { .. }));
        assert!(matches!(blocks[5], RawBlock::Directive(_)));
        assert!(matches!(blocks[6], RawBlock::Paragraph(_)));
        assert_eq!(blocks.len(), 7);
    }

    #[test]
    fn blocks_code_fence_consumes_correctly() {
        let lines = vec!["```rust", "fn main() {}", "```", "after"];
        let blocks = parse_blocks(&lines);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], RawBlock::CodeBlock { .. }));
        assert!(matches!(blocks[1], RawBlock::Paragraph(_)));
    }

    #[test]
    fn blocks_list_with_continuation() {
        let lines = vec!["- first", "  continued", "- second"];
        let blocks = parse_blocks(&lines);
        assert_eq!(blocks.len(), 1);
        if let RawBlock::List { items, .. } = &blocks[0] {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn blocks_ordered_list() {
        let lines = vec!["1. first", "2. second"];
        let blocks = parse_blocks(&lines);
        assert_eq!(blocks.len(), 1);
        if let RawBlock::List { ordered, start, items, .. } = &blocks[0] {
            assert!(ordered);
            assert_eq!(*start, Some(1));
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected ordered list");
        }
    }

    #[test]
    fn blocks_blockquote_consumes() {
        let lines = vec!["> line 1", "> line 2", "", "after"];
        let blocks = parse_blocks(&lines);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], RawBlock::BlockQuote(_)));
        assert!(matches!(blocks[1], RawBlock::Paragraph(_)));
    }

    #[test]
    fn blocks_table_consumes() {
        let lines = vec!["| H |", "|---|", "| d |", "", "after"];
        let blocks = parse_blocks(&lines);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], RawBlock::Table { .. }));
        assert!(matches!(blocks[1], RawBlock::Paragraph(_)));
    }

    // ── try_parse_table edge cases ──

    #[test]
    fn try_parse_table_with_alignment() {
        let lines = vec!["|L|C|R|", "|:---|:---:|---:|", "|a|b|c|"];
        let result = try_parse_table(&lines).unwrap();
        let (block, consumed) = result;
        assert_eq!(consumed, 3);
        if let RawBlock::Table { header, rows } = block {
            assert_eq!(header.len(), 3);
            assert_eq!(rows.len(), 1);
        }
    }

    #[test]
    fn try_parse_table_multiple_body_rows() {
        let lines = vec!["|H|", "|---|", "|a|", "|b|", "|c|"];
        let result = try_parse_table(&lines).unwrap();
        if let (RawBlock::Table { rows, .. }, consumed) = result {
            assert_eq!(rows.len(), 3);
            assert_eq!(consumed, 5);
        }
    }

    #[test]
    fn split_table_row_no_outer_pipes() {
        let cells = split_table_row("a | b | c");
        assert_eq!(cells, vec!["a", "b", "c"]);
    }

    // ── try_parse_fenced_code edge cases ──

    #[test]
    fn fenced_code_with_lang_and_brace_attrs() {
        let lines = vec!["```rust {#my-code .highlight}", "fn main() {}", "```"];
        let result = try_parse_fenced_code(&lines).unwrap();
        if let (RawBlock::CodeBlock { attrs, source }, _) = result {
            let a = attrs.unwrap();
            assert!(a.contains("rust"));
            assert!(a.contains("#my-code"));
            assert_eq!(source, "fn main() {}");
        }
    }

    // ── try_parse_list edge cases ──

    #[test]
    fn try_parse_list_ordered_custom_start() {
        let lines = vec!["5. fifth", "6. sixth"];
        let result = try_parse_list(&lines).unwrap();
        if let (RawBlock::List { ordered, start, items }, consumed) = result {
            assert!(ordered);
            assert_eq!(start, Some(5));
            assert_eq!(items.len(), 2);
            assert_eq!(consumed, 2);
        }
    }

    #[test]
    fn try_parse_list_blank_between_items() {
        let lines = vec!["- item 1", "", "- item 2"];
        let result = try_parse_list(&lines).unwrap();
        if let (RawBlock::List { items, .. }, _) = result {
            assert_eq!(items.len(), 2);
        }
    }

    #[test]
    fn try_parse_list_partial_indent_continuation() {
        // Line indented more than base but less than content_indent
        let lines = vec!["- item", " partial"];
        let result = try_parse_list(&lines).unwrap();
        if let (RawBlock::List { items, .. }, consumed) = result {
            assert_eq!(items.len(), 1);
            assert_eq!(consumed, 2);
        }
    }

    #[test]
    fn try_parse_list_stops_at_unindented() {
        let lines = vec!["- item", "not indented"];
        let result = try_parse_list(&lines).unwrap();
        if let (RawBlock::List { items, .. }, consumed) = result {
            assert_eq!(items.len(), 1);
            assert_eq!(consumed, 1);
        }
    }

    // ── parse_paragraph edge cases ──

    #[test]
    fn paragraph_stops_at_table() {
        let (_, consumed) = parse_paragraph(&["Line 1", "| A |", "|---|"]);
        assert_eq!(consumed, 1);
    }

}
