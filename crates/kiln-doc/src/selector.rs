//! CSS-like selector parsing and matching for hint cascade.
//!
//! Selectors match nodes in the document tree. The syntax supports:
//! - Kind names: `figure`, `code-block`, `h` (bare or alias)
//! - Classes: `.wide`
//! - IDs: `#intro`
//! - Attributes: `[lang="rust"]`, `[preview]`
//! - Compounds: `figure.wide[level=2]`
//! - Combinators: `figure > caption`, `section ~> figure`
//! - Boolean: `&` (AND), `|` (OR), `,` (context-dependent)

use std::collections::HashMap;

use crate::ast::*;

// ── Kind registry ──

struct KindEntry {
    pascal: &'static str,
    kebab: &'static str,
    alias: &'static str,
}

const KIND_REGISTRY: &[KindEntry] = &[
    KindEntry {
        pascal: "Section",
        kebab: "section",
        alias: "sec",
    },
    KindEntry {
        pascal: "Heading",
        kebab: "heading",
        alias: "h",
    },
    KindEntry {
        pascal: "Paragraph",
        kebab: "paragraph",
        alias: "p",
    },
    KindEntry {
        pascal: "BlockQuote",
        kebab: "block-quote",
        alias: "bq",
    },
    KindEntry {
        pascal: "List",
        kebab: "list",
        alias: "list",
    },
    KindEntry {
        pascal: "ListItem",
        kebab: "list-item",
        alias: "li",
    },
    KindEntry {
        pascal: "Table",
        kebab: "table",
        alias: "table",
    },
    KindEntry {
        pascal: "TableRow",
        kebab: "table-row",
        alias: "tr",
    },
    KindEntry {
        pascal: "TableCell",
        kebab: "table-cell",
        alias: "td",
    },
    KindEntry {
        pascal: "ThematicBreak",
        kebab: "thematic-break",
        alias: "hr",
    },
    KindEntry {
        pascal: "Figure",
        kebab: "figure",
        alias: "fig",
    },
    KindEntry {
        pascal: "Caption",
        kebab: "caption",
        alias: "cap",
    },
    KindEntry {
        pascal: "CodeBlock",
        kebab: "code-block",
        alias: "code",
    },
    KindEntry {
        pascal: "RawBlock",
        kebab: "raw-block",
        alias: "raw",
    },
    KindEntry {
        pascal: "Math",
        kebab: "math",
        alias: "math",
    },
    KindEntry {
        pascal: "Text",
        kebab: "text",
        alias: "t",
    },
    KindEntry {
        pascal: "Code",
        kebab: "code-inline",
        alias: "ic",
    },
    KindEntry {
        pascal: "Emphasis",
        kebab: "emphasis",
        alias: "em",
    },
    KindEntry {
        pascal: "Strong",
        kebab: "strong",
        alias: "strong",
    },
    KindEntry {
        pascal: "Strikethrough",
        kebab: "strikethrough",
        alias: "del",
    },
    KindEntry {
        pascal: "Link",
        kebab: "link",
        alias: "a",
    },
    KindEntry {
        pascal: "Image",
        kebab: "image",
        alias: "img",
    },
    KindEntry {
        pascal: "Citation",
        kebab: "citation",
        alias: "cite",
    },
    KindEntry {
        pascal: "CrossRef",
        kebab: "cross-ref",
        alias: "ref",
    },
];

/// Resolve any form of kind name (PascalCase, kebab-case, or alias) to canonical PascalCase.
pub fn resolve_kind_name(input: &str) -> Option<&'static str> {
    KIND_REGISTRY
        .iter()
        .find(|e| e.pascal == input || e.kebab == input || e.alias == input)
        .map(|e| e.pascal)
}

/// Map a `NodeKind` variant to its canonical PascalCase name.
pub fn node_kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Section { .. } => "Section",
        NodeKind::FileScope { .. } => "FileScope",
        NodeKind::Heading { .. } => "Heading",
        NodeKind::Paragraph => "Paragraph",
        NodeKind::BlockQuote => "BlockQuote",
        NodeKind::List { .. } => "List",
        NodeKind::ListItem => "ListItem",
        NodeKind::Table => "Table",
        NodeKind::TableRow => "TableRow",
        NodeKind::TableCell => "TableCell",
        NodeKind::ThematicBreak => "ThematicBreak",
        NodeKind::Figure => "Figure",
        NodeKind::Caption => "Caption",
        NodeKind::CodeBlock { .. } => "CodeBlock",
        NodeKind::RawBlock { .. } => "RawBlock",
        NodeKind::Math { .. } => "Math",
        NodeKind::Text(_) => "Text",
        NodeKind::Code(_) => "Code",
        NodeKind::Emphasis => "Emphasis",
        NodeKind::Strong => "Strong",
        NodeKind::Strikethrough => "Strikethrough",
        NodeKind::Link { .. } => "Link",
        NodeKind::Image { .. } => "Image",
        NodeKind::Citation { .. } => "Citation",
        NodeKind::CrossRef { .. } => "CrossRef",
        NodeKind::SoftBreak => "SoftBreak",
        NodeKind::HardBreak => "HardBreak",
        NodeKind::Space { .. } => "Space",
        NodeKind::Custom { .. } => "Custom",
    }
}

// ── NodeSnapshot ──

/// Lightweight snapshot of a node for ancestor matching (avoids borrow conflicts).
#[derive(Clone)]
pub struct NodeSnapshot {
    pub kind: NodeKind,
    pub id: String,
    pub auto_id: bool,
    pub classes: Vec<String>,
    pub properties: HashMap<String, Value>,
}

impl NodeSnapshot {
    pub fn from_node(node: &Node) -> Self {
        Self {
            kind: node.kind.clone(),
            id: node.attrs.id.clone(),
            auto_id: node.attrs.auto_id,
            classes: node.attrs.classes.clone(),
            properties: node.attrs.properties.clone(),
        }
    }
}

// ── Matching ──

/// Test whether a selector matches a node, given its ancestor chain.
/// `ancestors` is ordered root-first: `[root, ..., parent]`.
pub fn matches_selector(selector: &Selector, node: &Node, ancestors: &[NodeSnapshot]) -> bool {
    match selector {
        Selector::This => false, // handled separately at declaration site
        Selector::Expr(expr) => matches_expr(expr, node, ancestors),
    }
}

fn matches_expr(expr: &SelectorExpr, node: &Node, ancestors: &[NodeSnapshot]) -> bool {
    match expr {
        SelectorExpr::Simple(compound) => matches_compound(compound, node),
        SelectorExpr::Combinator { left, op, right } => {
            matches_expr(right, node, ancestors) && matches_combinator_left(left, op, ancestors)
        }
        SelectorExpr::Or(exprs) => exprs.iter().any(|e| matches_expr(e, node, ancestors)),
        SelectorExpr::And(exprs) => exprs.iter().all(|e| matches_expr(e, node, ancestors)),
    }
}

fn matches_combinator_left(
    left: &SelectorExpr,
    op: &CombinatorOp,
    ancestors: &[NodeSnapshot],
) -> bool {
    match op {
        CombinatorOp::Child => ancestors
            .last()
            .is_some_and(|parent| matches_expr_snapshot(left, parent)),
        CombinatorOp::Descendant => ancestors.iter().any(|a| matches_expr_snapshot(left, a)),
    }
}

/// Match an expression against a snapshot (for ancestor matching).
/// Combinators in ancestor context are not supported (would need deeper ancestor chain).
fn matches_expr_snapshot(expr: &SelectorExpr, snap: &NodeSnapshot) -> bool {
    match expr {
        SelectorExpr::Simple(compound) => matches_compound_snapshot(compound, snap),
        SelectorExpr::Or(exprs) => exprs.iter().any(|e| matches_expr_snapshot(e, snap)),
        SelectorExpr::And(exprs) => exprs.iter().all(|e| matches_expr_snapshot(e, snap)),
        SelectorExpr::Combinator { .. } => false, // not supported in ancestor context
    }
}

fn matches_compound(compound: &CompoundSelector, node: &Node) -> bool {
    if let Some(ref kind) = compound.kind
        && node_kind_name(&node.kind) != kind.as_str()
    {
        return false;
    }
    if let Some(ref id) = compound.id
        && (node.attrs.auto_id || node.attrs.id != *id)
    {
        return false;
    }
    if compound
        .classes
        .iter()
        .any(|c| !node.attrs.classes.contains(c))
    {
        return false;
    }
    if let Some(ref expr) = compound.attr_expr
        && !matches_attr_expr(expr, &node.kind, &node.attrs.properties)
    {
        return false;
    }
    true
}

fn matches_compound_snapshot(compound: &CompoundSelector, snap: &NodeSnapshot) -> bool {
    if let Some(ref kind) = compound.kind
        && node_kind_name(&snap.kind) != kind.as_str()
    {
        return false;
    }
    if let Some(ref id) = compound.id
        && (snap.auto_id || snap.id != *id)
    {
        return false;
    }
    if compound.classes.iter().any(|c| !snap.classes.contains(c)) {
        return false;
    }
    if let Some(ref expr) = compound.attr_expr
        && !matches_attr_expr(expr, &snap.kind, &snap.properties)
    {
        return false;
    }
    true
}

fn matches_attr_expr(
    expr: &AttrExpr,
    kind: &NodeKind,
    properties: &HashMap<String, Value>,
) -> bool {
    match expr {
        AttrExpr::Pred(pred) => matches_attr_pred(pred, kind, properties),
        AttrExpr::And(exprs) => exprs.iter().all(|e| matches_attr_expr(e, kind, properties)),
        AttrExpr::Or(exprs) => exprs.iter().any(|e| matches_attr_expr(e, kind, properties)),
    }
}

fn matches_attr_pred(
    pred: &AttrPred,
    kind: &NodeKind,
    properties: &HashMap<String, Value>,
) -> bool {
    let val = get_node_attr(&pred.name, kind, properties);
    match &pred.op {
        AttrOp::Exists => val.is_some(),
        AttrOp::Eq(expected) => val.as_deref() == Some(expected.as_str()),
        AttrOp::StartsWith(prefix) => val.is_some_and(|v| v.starts_with(prefix.as_str())),
    }
}

/// Get a node attribute by name, checking structural fields first, then user properties.
fn get_node_attr(
    name: &str,
    kind: &NodeKind,
    properties: &HashMap<String, Value>,
) -> Option<String> {
    // Structural attributes from NodeKind fields.
    match (kind, name) {
        (NodeKind::Heading { level } | NodeKind::Section { level }, "level") => {
            Some(level.to_string())
        }
        (NodeKind::CodeBlock { lang, .. }, "lang") => Some(lang.clone()),
        (NodeKind::RawBlock { format, .. }, "format") => Some(format.clone()),
        (NodeKind::List { ordered, .. }, "ordered") => Some(ordered.to_string()),
        (NodeKind::Math { display, .. }, "display") => Some(display.to_string()),
        (NodeKind::Link { url } | NodeKind::Image { url, .. }, "url") => Some(url.clone()),
        (NodeKind::Image { alt, .. }, "alt") => Some(alt.clone()),
        _ => {
            // Fall through to user properties.
            properties.get(name).map(|v| match v {
                Value::Text(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                _ => format!("{v:?}"),
            })
        }
    }
}

// ── Parsing ──

/// Parse a selector string into a `Selector`.
///
/// Returns `None` if the input is empty or invalid.
pub fn parse_selector(input: &str) -> Option<Selector> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if input == "this" {
        return Some(Selector::This);
    }
    parse_selector_expr(input).map(Selector::Expr)
}

/// Parse a selector expression (handles `,` as OR at the top level).
fn parse_selector_expr(input: &str) -> Option<SelectorExpr> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    // Split on top-level commas (OR outside brackets).
    let comma_parts = split_top_level(input, ',');
    if comma_parts.len() > 1 {
        let exprs: Option<Vec<_>> = comma_parts
            .iter()
            .map(|p| parse_and_expr(p.trim()))
            .collect();
        return Some(SelectorExpr::Or(exprs?));
    }

    parse_and_expr(input)
}

/// Parse an expression at the `&` precedence level.
fn parse_and_expr(input: &str) -> Option<SelectorExpr> {
    let input = input.trim();
    let parts = split_top_level(input, '&');
    if parts.len() > 1 {
        let exprs: Option<Vec<_>> = parts.iter().map(|p| parse_or_expr(p.trim())).collect();
        return Some(SelectorExpr::And(exprs?));
    }
    parse_or_expr(input)
}

/// Parse an expression at the `|` precedence level.
fn parse_or_expr(input: &str) -> Option<SelectorExpr> {
    let input = input.trim();
    let parts = split_top_level(input, '|');
    if parts.len() > 1 {
        let exprs: Option<Vec<_>> = parts
            .iter()
            .map(|p| parse_combinator_expr(p.trim()))
            .collect();
        return Some(SelectorExpr::Or(exprs?));
    }
    parse_combinator_expr(input)
}

/// Parse an expression at the combinator precedence level (`>`, `~>`).
fn parse_combinator_expr(input: &str) -> Option<SelectorExpr> {
    let input = input.trim();

    // Scan for combinator tokens, respecting brackets.
    let parts = split_on_combinators(input);
    if parts.len() == 1 {
        return parse_compound_str(parts[0].0.trim());
    }

    // Build right-associative combinator chain.
    // parts: [(segment, Some(op)), (segment, Some(op)), ..., (segment, None)]
    let mut iter = parts.into_iter().rev();
    let (last_seg, _) = iter.next()?;
    let mut right = parse_compound_str(last_seg.trim())?;

    for (seg, op) in iter {
        let left = parse_compound_str(seg.trim())?;
        right = SelectorExpr::Combinator {
            left: Box::new(left),
            op: op?,
            right: Box::new(right),
        };
    }
    Some(right)
}

/// Parse a single compound selector string like `figure.wide[lang="rust"]`.
fn parse_compound_str(input: &str) -> Option<SelectorExpr> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    let mut compound = CompoundSelector::default();
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0;
    let mut attr_exprs: Vec<AttrExpr> = Vec::new();

    while pos < chars.len() {
        match chars[pos] {
            '#' => {
                pos += 1;
                let (token, new_pos) = scan_ident(&chars, pos);
                compound.id = Some(token);
                pos = new_pos;
            }
            '.' => {
                pos += 1;
                let (token, new_pos) = scan_ident(&chars, pos);
                compound.classes.push(token);
                pos = new_pos;
            }
            '[' => {
                let (expr, new_pos) = parse_bracket_expr(&chars, pos)?;
                attr_exprs.push(expr);
                pos = new_pos;
            }
            _ => {
                let (token, new_pos) = scan_bare_kind(&chars, pos);
                parse_bare_kind_token(&token, &mut compound)?;
                pos = new_pos;
            }
        }
    }

    // Merge attribute expressions: multiple brackets are ANDed.
    compound.attr_expr = match attr_exprs.len() {
        0 => None,
        1 => Some(attr_exprs.remove(0)),
        _ => Some(AttrExpr::And(attr_exprs)),
    };

    Some(SelectorExpr::Simple(compound))
}

/// Parse a bare kind token like `figure` or `fig:arch` into the compound selector.
fn parse_bare_kind_token(token: &str, compound: &mut CompoundSelector) -> Option<()> {
    if let Some(colon_pos) = token.find(':') {
        let kind_part = &token[..colon_pos];
        let id_part = &token[colon_pos + 1..];
        compound.kind = Some(resolve_kind_name(kind_part)?.to_string());
        if id_part != "*" {
            compound.id = Some(token.to_string());
        }
    } else {
        compound.kind = Some(resolve_kind_name(token)?.to_string());
    }
    Some(())
}

/// Parse the contents of a `[...]` bracket into an `AttrExpr`.
/// Returns the expression and the position after the closing `]`.
fn parse_bracket_expr(chars: &[char], pos: usize) -> Option<(AttrExpr, usize)> {
    debug_assert_eq!(chars[pos], '[');
    let close = find_bracket_close(chars, pos)?;
    let inner: String = chars[pos + 1..close].iter().collect();
    let expr = parse_attr_expr_str(&inner)?;
    Some((expr, close + 1))
}

/// Parse an attribute expression string (contents of `[...]`).
/// Precedence: `&` > `|` > `,` (AND inside brackets).
fn parse_attr_expr_str(input: &str) -> Option<AttrExpr> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    // Split on comma (AND, loosest precedence inside brackets).
    let comma_parts = split_top_level_attr(input, ',');
    if comma_parts.len() > 1 {
        let exprs: Option<Vec<_>> = comma_parts
            .iter()
            .map(|p| parse_attr_or_expr(p.trim()))
            .collect();
        return Some(AttrExpr::And(exprs?));
    }

    parse_attr_or_expr(input)
}

/// Parse at `|` precedence inside brackets.
fn parse_attr_or_expr(input: &str) -> Option<AttrExpr> {
    let parts = split_top_level_attr(input, '|');
    if parts.len() > 1 {
        let exprs: Option<Vec<_>> = parts
            .iter()
            .map(|p| parse_attr_and_expr(p.trim()))
            .collect();
        return Some(AttrExpr::Or(exprs?));
    }
    parse_attr_and_expr(input)
}

/// Parse at `&` precedence inside brackets (tightest).
fn parse_attr_and_expr(input: &str) -> Option<AttrExpr> {
    let parts = split_top_level_attr(input, '&');
    if parts.len() > 1 {
        let exprs: Option<Vec<_>> = parts
            .iter()
            .map(|p| parse_single_attr_pred(p.trim()))
            .collect();
        return Some(AttrExpr::And(exprs?));
    }
    parse_single_attr_pred(input)
}

/// Parse a single attribute predicate: `lang="rust"`, `preview`, `lang=("rust", "python")`.
fn parse_single_attr_pred(input: &str) -> Option<AttrExpr> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    let Some(eq_pos) = input.find('=') else {
        // Bare name → exists check.
        return Some(AttrExpr::Pred(AttrPred {
            name: input.to_string(),
            op: AttrOp::Exists,
        }));
    };

    let name = input[..eq_pos].trim().to_string();
    let value_str = input[eq_pos + 1..].trim();

    if value_str.starts_with('(') && value_str.ends_with(')') {
        return Some(parse_tuple_attr(&name, value_str));
    }

    if let Some(rest) = value_str.strip_prefix('^') {
        let val = rest.trim().trim_matches('"').to_string();
        return Some(AttrExpr::Pred(AttrPred {
            name,
            op: AttrOp::StartsWith(val),
        }));
    }

    let val = value_str.trim_matches('"').to_string();
    Some(AttrExpr::Pred(AttrPred {
        name,
        op: AttrOp::Eq(val),
    }))
}

/// Parse a tuple attribute value `("rust", "python")` into an OR expression.
fn parse_tuple_attr(name: &str, value_str: &str) -> AttrExpr {
    let inner = &value_str[1..value_str.len() - 1];
    let preds: Vec<AttrExpr> = inner
        .split(',')
        .map(|v| {
            AttrExpr::Pred(AttrPred {
                name: name.to_string(),
                op: AttrOp::Eq(v.trim().trim_matches('"').to_string()),
            })
        })
        .collect();
    if preds.len() == 1 {
        preds.into_iter().next().unwrap()
    } else {
        AttrExpr::Or(preds)
    }
}

// ── Tokenization helpers ──

/// Scan an identifier (alphanumeric, hyphens, underscores, colons, dots for IDs).
fn scan_ident(chars: &[char], pos: usize) -> (String, usize) {
    let start = pos;
    let mut p = pos;
    while p < chars.len()
        && (chars[p].is_alphanumeric() || chars[p] == '-' || chars[p] == '_' || chars[p] == ':')
    {
        p += 1;
    }
    (chars[start..p].iter().collect(), p)
}

/// Scan a bare kind token (alphanumeric, hyphens, colons for kind:id shorthand).
fn scan_bare_kind(chars: &[char], pos: usize) -> (String, usize) {
    let start = pos;
    let mut p = pos;
    while p < chars.len()
        && (chars[p].is_alphanumeric()
            || chars[p] == '-'
            || chars[p] == '_'
            || chars[p] == ':'
            || chars[p] == '*')
    {
        p += 1;
    }
    (chars[start..p].iter().collect(), p)
}

/// Split a string on a delimiter at the top level (not inside `[...]` or `"`).
fn split_top_level(input: &str, delim: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut in_quotes = false;

    for (i, ch) in input.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '[' if !in_quotes => depth += 1,
            ']' if !in_quotes => depth -= 1,
            '(' if !in_quotes => depth += 1,
            ')' if !in_quotes => depth -= 1,
            c if c == delim && !in_quotes && depth == 0 => {
                // For '|', also check it's not part of '~>'... actually '~>' doesn't contain '|'.
                parts.push(&input[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

/// Split on delimiter inside attribute brackets (simpler — no bracket nesting).
fn split_top_level_attr(input: &str, delim: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut paren_depth = 0;

    for (i, ch) in input.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '(' if !in_quotes => paren_depth += 1,
            ')' if !in_quotes => paren_depth -= 1,
            c if c == delim && !in_quotes && paren_depth == 0 => {
                parts.push(&input[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

/// Split input on combinators (`>`, `~>`), returning segments with their trailing combinator.
fn split_on_combinators(input: &str) -> Vec<(&str, Option<CombinatorOp>)> {
    let mut parts = Vec::new();
    let bytes = input.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let mut depth = 0;
    let mut in_quotes = false;

    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                in_quotes = !in_quotes;
                i += 1;
            }
            b'[' if !in_quotes => {
                depth += 1;
                i += 1;
            }
            b']' if !in_quotes => {
                depth -= 1;
                i += 1;
            }
            b'~' if !in_quotes && depth == 0 && i + 1 < bytes.len() && bytes[i + 1] == b'>' => {
                parts.push((&input[start..i], Some(CombinatorOp::Descendant)));
                i += 2; // skip ~>
                start = i;
            }
            b'>' if !in_quotes && depth == 0 => {
                parts.push((&input[start..i], Some(CombinatorOp::Child)));
                i += 1;
                start = i;
            }
            _ => {
                i += 1;
            }
        }
    }
    parts.push((&input[start..], None));
    parts
}

/// Find the matching `]` for a `[` at position `pos`.
fn find_bracket_close(chars: &[char], pos: usize) -> Option<usize> {
    let mut depth = 1i32; // start inside the opening bracket
    let mut in_quotes = false;
    for (i, &ch) in chars.iter().enumerate().skip(pos + 1) {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if !in_quotes && ch == '[' {
            depth += 1;
        } else if !in_quotes && ch == ']' {
            depth -= 1;
        }
        if depth == 0 {
            return Some(i);
        }
    }
    None
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_kind_names() {
        assert_eq!(resolve_kind_name("figure"), Some("Figure"));
        assert_eq!(resolve_kind_name("fig"), Some("Figure"));
        assert_eq!(resolve_kind_name("Figure"), Some("Figure"));
        assert_eq!(resolve_kind_name("code-block"), Some("CodeBlock"));
        assert_eq!(resolve_kind_name("code"), Some("CodeBlock"));
        assert_eq!(resolve_kind_name("heading"), Some("Heading"));
        assert_eq!(resolve_kind_name("h"), Some("Heading"));
        assert_eq!(resolve_kind_name("block-quote"), Some("BlockQuote"));
        assert_eq!(resolve_kind_name("bq"), Some("BlockQuote"));
        assert_eq!(resolve_kind_name("nonsense"), None);
    }

    #[test]
    fn parse_this() {
        let sel = parse_selector("this").unwrap();
        assert!(matches!(sel, Selector::This));
    }

    #[test]
    fn parse_bare_kind() {
        let sel = parse_selector("figure").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        assert_eq!(c.kind.as_deref(), Some("Figure"));
    }

    #[test]
    fn parse_kind_alias() {
        let sel = parse_selector("fig").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        assert_eq!(c.kind.as_deref(), Some("Figure"));
    }

    #[test]
    fn parse_kebab_kind() {
        let sel = parse_selector("code-block").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        assert_eq!(c.kind.as_deref(), Some("CodeBlock"));
    }

    #[test]
    fn parse_class() {
        let sel = parse_selector(".wide").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        assert_eq!(c.classes, vec!["wide"]);
    }

    #[test]
    fn parse_id() {
        let sel = parse_selector("#intro").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        assert_eq!(c.id.as_deref(), Some("intro"));
    }

    #[test]
    fn parse_compound_kind_class() {
        let sel = parse_selector("figure.wide").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        assert_eq!(c.kind.as_deref(), Some("Figure"));
        assert_eq!(c.classes, vec!["wide"]);
    }

    #[test]
    fn parse_attr_eq() {
        let sel = parse_selector("[lang=\"rust\"]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        let Some(AttrExpr::Pred(pred)) = &c.attr_expr else {
            panic!("expected attr")
        };
        assert_eq!(pred.name, "lang");
        assert!(matches!(&pred.op, AttrOp::Eq(v) if v == "rust"));
    }

    #[test]
    fn parse_attr_exists() {
        let sel = parse_selector("[preview]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        let Some(AttrExpr::Pred(pred)) = &c.attr_expr else {
            panic!("expected attr")
        };
        assert_eq!(pred.name, "preview");
        assert!(matches!(pred.op, AttrOp::Exists));
    }

    #[test]
    fn parse_compound_kind_attr() {
        let sel = parse_selector("heading[level=2]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        assert_eq!(c.kind.as_deref(), Some("Heading"));
        let Some(AttrExpr::Pred(pred)) = &c.attr_expr else {
            panic!("expected attr")
        };
        assert_eq!(pred.name, "level");
        assert!(matches!(&pred.op, AttrOp::Eq(v) if v == "2"));
    }

    #[test]
    fn parse_kind_id_shorthand() {
        let sel = parse_selector("fig:arch").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        assert_eq!(c.kind.as_deref(), Some("Figure"));
        assert_eq!(c.id.as_deref(), Some("fig:arch"));
    }

    #[test]
    fn parse_kind_wildcard() {
        let sel = parse_selector("fig:*").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        assert_eq!(c.kind.as_deref(), Some("Figure"));
        assert!(c.id.is_none());
    }

    #[test]
    fn parse_child_combinator() {
        let sel = parse_selector("figure > caption").unwrap();
        let Selector::Expr(SelectorExpr::Combinator { left, op, right }) = sel else {
            panic!("expected Combinator")
        };
        assert_eq!(op, CombinatorOp::Child);
        let SelectorExpr::Simple(l) = *left else {
            panic!("expected Simple left")
        };
        assert_eq!(l.kind.as_deref(), Some("Figure"));
        let SelectorExpr::Simple(r) = *right else {
            panic!("expected Simple right")
        };
        assert_eq!(r.kind.as_deref(), Some("Caption"));
    }

    #[test]
    fn parse_descendant_combinator() {
        let sel = parse_selector("section ~> figure").unwrap();
        let Selector::Expr(SelectorExpr::Combinator { left, op, right }) = sel else {
            panic!("expected Combinator")
        };
        assert_eq!(op, CombinatorOp::Descendant);
        let SelectorExpr::Simple(l) = *left else {
            panic!("expected Simple left")
        };
        assert_eq!(l.kind.as_deref(), Some("Section"));
        let SelectorExpr::Simple(r) = *right else {
            panic!("expected Simple right")
        };
        assert_eq!(r.kind.as_deref(), Some("Figure"));
    }

    #[test]
    fn parse_or_comma() {
        let sel = parse_selector("figure, code-block").unwrap();
        let Selector::Expr(SelectorExpr::Or(exprs)) = sel else {
            panic!("expected Or")
        };
        assert_eq!(exprs.len(), 2);
    }

    #[test]
    fn parse_explicit_and() {
        let sel = parse_selector("figure & .wide").unwrap();
        let Selector::Expr(SelectorExpr::And(exprs)) = sel else {
            panic!("expected And")
        };
        assert_eq!(exprs.len(), 2);
    }

    #[test]
    fn parse_explicit_or() {
        let sel = parse_selector("figure | code-block").unwrap();
        let Selector::Expr(SelectorExpr::Or(exprs)) = sel else {
            panic!("expected Or")
        };
        assert_eq!(exprs.len(), 2);
    }

    #[test]
    fn parse_attr_and_inside_brackets() {
        let sel = parse_selector("[lang=\"rust\", preview]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        let Some(AttrExpr::And(exprs)) = &c.attr_expr else {
            panic!("expected And")
        };
        assert_eq!(exprs.len(), 2);
    }

    #[test]
    fn parse_attr_or_inside_brackets() {
        let sel = parse_selector("[lang=\"rust\" | level=2]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        let Some(AttrExpr::Or(exprs)) = &c.attr_expr else {
            panic!("expected Or")
        };
        assert_eq!(exprs.len(), 2);
    }

    #[test]
    fn parse_tuple_distribution() {
        let sel = parse_selector("[lang=(\"rust\", \"python\")]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        let Some(AttrExpr::Or(exprs)) = &c.attr_expr else {
            panic!("expected Or")
        };
        assert_eq!(exprs.len(), 2);
    }

    #[test]
    fn parse_chained_brackets() {
        let sel = parse_selector("[lang=\"rust\"][preview]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        let Some(AttrExpr::And(exprs)) = &c.attr_expr else {
            panic!("expected And")
        };
        assert_eq!(exprs.len(), 2);
    }

    #[test]
    fn parse_precedence_comma_and_or() {
        // A & B, C → (A & B) | C (& tighter than ,)
        let sel = parse_selector("figure & .wide, code-block").unwrap();
        let Selector::Expr(SelectorExpr::Or(exprs)) = sel else {
            panic!("expected Or")
        };
        assert_eq!(exprs.len(), 2);
        assert!(matches!(&exprs[0], SelectorExpr::And(_)));
    }

    #[test]
    fn parse_attr_precedence_comma_or() {
        // [a, b | c] → a AND (b OR c) — | tighter than , inside brackets
        let sel = parse_selector("[a, b | c]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        let Some(AttrExpr::And(exprs)) = &c.attr_expr else {
            panic!("expected And")
        };
        assert_eq!(exprs.len(), 2);
        assert!(matches!(&exprs[0], AttrExpr::Pred(_)));
        assert!(matches!(&exprs[1], AttrExpr::Or(_)));
    }
}
