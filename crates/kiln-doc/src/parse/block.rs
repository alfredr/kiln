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
        lang: String,
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
    /// A standalone directive line like `{@series(...)}`
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
    // Parse lang and optional brace attrs from info string
    let (lang, attrs) = if let Some(brace_start) = info_string.find('{') {
        let lang = info_string[..brace_start].trim();
        let brace_end = info_string.rfind('}')?;
        let attrs = &info_string[brace_start + 1..brace_end];
        (lang.to_string(), Some(attrs.to_string()))
    } else {
        (info_string.to_string(), None)
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
    Some((
        RawBlock::CodeBlock {
            lang,
            source,
            attrs,
        },
        i,
    ))
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
