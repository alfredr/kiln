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
    // String-typed structural fields — hoist the clone.
    let string_field: Option<&String> = match (kind, name) {
        (NodeKind::CodeBlock { lang, .. }, "lang") => Some(lang),
        (NodeKind::RawBlock { format, .. }, "format") => Some(format),
        (NodeKind::Link { url } | NodeKind::Image { url, .. }, "url") => Some(url),
        (NodeKind::Image { alt, .. }, "alt") => Some(alt),
        _ => None,
    };
    if let Some(s) = string_field {
        return Some(s.clone());
    }
    // Non-string structural fields.
    let formatted = match (kind, name) {
        (NodeKind::Heading { level } | NodeKind::Section { level }, "level") => {
            Some(level.to_string())
        }
        (NodeKind::List { ordered, .. }, "ordered") => Some(ordered.to_string()),
        (NodeKind::Math { display, .. }, "display") => Some(display.to_string()),
        _ => None,
    };
    formatted.or_else(|| {
        properties.get(name).map(|v| match v {
            Value::Text(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            _ => format!("{v:?}"),
        })
    })
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

/// Generic precedence-level parser.
/// Splits `input` on `delim` using `split_fn`, wraps multiple parts with `wrap`,
/// and delegates single parts to `next`.
fn parse_prec_level<T>(
    input: &str,
    delim: char,
    wrap: fn(Vec<T>) -> T,
    split_fn: fn(&str, char) -> Vec<&str>,
    next: fn(&str) -> Option<T>,
) -> Option<T> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let parts = split_fn(input, delim);
    if parts.len() > 1 {
        let exprs: Option<Vec<_>> = parts.iter().map(|p| next(p.trim())).collect();
        return Some(wrap(exprs?));
    }
    next(input)
}

/// Parse a selector expression (handles `,` as OR at the top level).
fn parse_selector_expr(input: &str) -> Option<SelectorExpr> {
    parse_prec_level(
        input,
        ',',
        SelectorExpr::Or,
        split_top_level,
        parse_and_expr,
    )
}

/// Parse an expression at the `&` precedence level.
fn parse_and_expr(input: &str) -> Option<SelectorExpr> {
    parse_prec_level(
        input,
        '&',
        SelectorExpr::And,
        split_top_level,
        parse_or_expr,
    )
}

/// Parse an expression at the `|` precedence level.
fn parse_or_expr(input: &str) -> Option<SelectorExpr> {
    parse_prec_level(
        input,
        '|',
        SelectorExpr::Or,
        split_top_level,
        parse_combinator_expr,
    )
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
            sigil @ ('#' | '.') => {
                let (token, end) = scan_token(&chars, pos + 1, false);
                match sigil {
                    '#' => compound.id = Some(token),
                    _ => compound.classes.push(token),
                }
                pos = end;
            }
            '[' => {
                let (expr, end) = parse_bracket_expr(&chars, pos)?;
                attr_exprs.push(expr);
                pos = end;
            }
            _ => {
                let (token, end) = scan_token(&chars, pos, true);
                parse_bare_kind_token(&token, &mut compound)?;
                pos = end;
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
    parse_prec_level(
        input,
        ',',
        AttrExpr::And,
        split_top_level_attr,
        parse_attr_or_expr,
    )
}

/// Parse at `|` precedence inside brackets.
fn parse_attr_or_expr(input: &str) -> Option<AttrExpr> {
    parse_prec_level(
        input,
        '|',
        AttrExpr::Or,
        split_top_level_attr,
        parse_attr_and_expr,
    )
}

/// Parse at `&` precedence inside brackets (tightest).
fn parse_attr_and_expr(input: &str) -> Option<AttrExpr> {
    parse_prec_level(
        input,
        '&',
        AttrExpr::And,
        split_top_level_attr,
        parse_single_attr_pred,
    )
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

/// Scan a token: alphanumeric, hyphens, underscores, colons.
/// When `allow_star` is true, also accepts `*` (for kind wildcard like `fig:*`).
fn scan_token(chars: &[char], pos: usize, allow_star: bool) -> (String, usize) {
    let start = pos;
    let mut p = pos;
    while p < chars.len() {
        let ch = chars[p];
        if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == ':' || (allow_star && ch == '*')
        {
            p += 1;
        } else {
            break;
        }
    }
    (chars[start..p].iter().collect(), p)
}

/// Split a string on a delimiter, respecting quotes and nested delimiters.
/// `track_brackets`: count `[`/`]` as nesting. `track_parens`: count `(`/`)` as nesting.
fn split_on_delim(input: &str, delim: char, track_brackets: bool, track_parens: bool) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut in_quotes = false;

    for (i, ch) in input.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '[' if !in_quotes && track_brackets => depth += 1,
            ']' if !in_quotes && track_brackets => depth -= 1,
            '(' if !in_quotes && track_parens => depth += 1,
            ')' if !in_quotes && track_parens => depth -= 1,
            c if c == delim && !in_quotes && depth == 0 => {
                parts.push(&input[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

/// Split on delimiter at the top level (respects both brackets and parens).
fn split_top_level(input: &str, delim: char) -> Vec<&str> {
    split_on_delim(input, delim, true, true)
}

/// Split on delimiter inside attribute brackets (respects parens only).
fn split_top_level_attr(input: &str, delim: char) -> Vec<&str> {
    split_on_delim(input, delim, false, true)
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
        // Check for combinator tokens first.
        let combinator = if bytes[i] == b'~'
            && !in_quotes
            && depth == 0
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'>'
        {
            Some(CombinatorOp::Descendant)
        } else if bytes[i] == b'>' && !in_quotes && depth == 0 {
            Some(CombinatorOp::Child)
        } else {
            None
        };
        if let Some(op) = combinator {
            parts.push((&input[start..i], Some(op)));
            i += op.token_len();
            start = i;
            continue;
        }
        // Handle nesting and quoting.
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b'[' if !in_quotes => depth += 1,
            b']' if !in_quotes => depth -= 1,
            _ => {}
        }
        i += 1;
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

    // ── Matching tests ──

    fn make_node(kind: NodeKind) -> Node {
        Node {
            kind,
            attrs: Attrs::default(),
            children: vec![],
        }
    }

    fn make_snapshot(kind: NodeKind) -> NodeSnapshot {
        NodeSnapshot {
            kind,
            id: String::new(),
            auto_id: true,
            classes: vec![],
            properties: HashMap::new(),
        }
    }

    #[test]
    fn matches_selector_this_returns_false() {
        let node = make_node(NodeKind::Paragraph);
        assert!(!matches_selector(&Selector::This, &node, &[]));
    }

    #[test]
    fn matches_kind() {
        let mut node = make_node(NodeKind::Paragraph);
        node.attrs.auto_id = false;
        let sel = parse_selector("paragraph").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        let sel2 = parse_selector("heading").unwrap();
        assert!(!matches_selector(&sel2, &node, &[]));
    }

    #[test]
    fn matches_id_explicit_only() {
        let mut node = make_node(NodeKind::Paragraph);
        node.attrs.id = "intro".to_string();
        node.attrs.auto_id = false;

        let sel = parse_selector("#intro").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        // Auto-generated ID should not match
        node.attrs.auto_id = true;
        assert!(!matches_selector(&sel, &node, &[]));
    }

    #[test]
    fn matches_class() {
        let mut node = make_node(NodeKind::Paragraph);
        node.attrs.classes.push("wide".to_string());

        let sel = parse_selector(".wide").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        let sel2 = parse_selector(".narrow").unwrap();
        assert!(!matches_selector(&sel2, &node, &[]));
    }

    #[test]
    fn matches_attr_exists() {
        let mut node = make_node(NodeKind::Paragraph);
        node.attrs
            .properties
            .insert("preview".to_string(), Value::Bool(true));

        let sel = parse_selector("[preview]").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        let sel2 = parse_selector("[missing]").unwrap();
        assert!(!matches_selector(&sel2, &node, &[]));
    }

    #[test]
    fn matches_attr_eq() {
        let node = make_node(NodeKind::CodeBlock {
            lang: "rust".to_string(),
            source: String::new(),
        });

        let sel = parse_selector("[lang=\"rust\"]").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        let sel2 = parse_selector("[lang=\"python\"]").unwrap();
        assert!(!matches_selector(&sel2, &node, &[]));
    }

    #[test]
    fn matches_attr_starts_with() {
        let node = make_node(NodeKind::CodeBlock {
            lang: "rust-nightly".to_string(),
            source: String::new(),
        });

        let sel = parse_selector("[lang=^\"rust\"]").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        let sel2 = parse_selector("[lang=^\"python\"]").unwrap();
        assert!(!matches_selector(&sel2, &node, &[]));
    }

    #[test]
    fn matches_structural_attrs() {
        // Heading level
        let heading = make_node(NodeKind::Heading { level: 2 });
        let sel = parse_selector("[level=2]").unwrap();
        assert!(matches_selector(&sel, &heading, &[]));

        // Section level
        let section = make_node(NodeKind::Section { level: 3 });
        let sel = parse_selector("[level=3]").unwrap();
        assert!(matches_selector(&sel, &section, &[]));

        // RawBlock format
        let raw = make_node(NodeKind::RawBlock {
            format: "html".to_string(),
            source: String::new(),
        });
        let sel = parse_selector("[format=\"html\"]").unwrap();
        assert!(matches_selector(&sel, &raw, &[]));

        // List ordered
        let list = make_node(NodeKind::List {
            ordered: true,
            start: None,
        });
        let sel = parse_selector("[ordered=true]").unwrap();
        assert!(matches_selector(&sel, &list, &[]));

        // Math display
        let math = make_node(NodeKind::Math {
            display: true,
            source: String::new(),
        });
        let sel = parse_selector("[display=true]").unwrap();
        assert!(matches_selector(&sel, &math, &[]));

        // Link url
        let link = make_node(NodeKind::Link {
            url: "https://example.com".to_string(),
        });
        let sel = parse_selector("[url=\"https://example.com\"]").unwrap();
        assert!(matches_selector(&sel, &link, &[]));

        // Image url and alt
        let img = make_node(NodeKind::Image {
            url: "img.png".to_string(),
            alt: "photo".to_string(),
        });
        let sel_url = parse_selector("[url=\"img.png\"]").unwrap();
        assert!(matches_selector(&sel_url, &img, &[]));
        let sel_alt = parse_selector("[alt=\"photo\"]").unwrap();
        assert!(matches_selector(&sel_alt, &img, &[]));
    }

    #[test]
    fn matches_property_fallback_types() {
        let mut node = make_node(NodeKind::Paragraph);
        node.attrs
            .properties
            .insert("flag".to_string(), Value::Bool(true));
        node.attrs
            .properties
            .insert("count".to_string(), Value::Number(42.0));
        node.attrs
            .properties
            .insert("label".to_string(), Value::Text("hello".to_string()));

        let sel = parse_selector("[flag=true]").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        let sel = parse_selector("[count=42]").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        let sel = parse_selector("[label=\"hello\"]").unwrap();
        assert!(matches_selector(&sel, &node, &[]));
    }

    #[test]
    fn matches_or_expr() {
        let node = make_node(NodeKind::Paragraph);
        let sel = parse_selector("paragraph, heading").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        let node2 = make_node(NodeKind::CodeBlock {
            lang: String::new(),
            source: String::new(),
        });
        assert!(!matches_selector(&sel, &node2, &[]));
    }

    #[test]
    fn matches_and_expr() {
        let mut node = make_node(NodeKind::Paragraph);
        node.attrs.classes.push("wide".to_string());

        let sel = parse_selector("paragraph & .wide").unwrap();
        assert!(matches_selector(&sel, &node, &[]));

        let sel2 = parse_selector("paragraph & .narrow").unwrap();
        assert!(!matches_selector(&sel2, &node, &[]));
    }

    #[test]
    fn matches_child_combinator() {
        let child = make_node(NodeKind::Caption);
        let parent = make_snapshot(NodeKind::Figure);
        let ancestors = vec![parent];

        let sel = parse_selector("figure > caption").unwrap();
        assert!(matches_selector(&sel, &child, &ancestors));

        // Wrong parent
        let wrong_parent = make_snapshot(NodeKind::Paragraph);
        assert!(!matches_selector(&sel, &child, &[wrong_parent]));

        // No parent
        assert!(!matches_selector(&sel, &child, &[]));
    }

    #[test]
    fn matches_descendant_combinator() {
        let code = make_node(NodeKind::CodeBlock {
            lang: "rust".to_string(),
            source: String::new(),
        });
        let section = make_snapshot(NodeKind::Section { level: 1 });
        let ancestors = vec![section];

        let sel = parse_selector("section ~> code-block").unwrap();
        assert!(matches_selector(&sel, &code, &ancestors));

        // Nested deeper
        let root = make_snapshot(NodeKind::Section { level: 1 });
        let inner = make_snapshot(NodeKind::Paragraph);
        assert!(matches_selector(&sel, &code, &[root, inner]));

        // No matching ancestor
        let para_only = make_snapshot(NodeKind::Paragraph);
        assert!(!matches_selector(&sel, &code, &[para_only]));
    }

    #[test]
    fn matches_snapshot_or_and() {
        // Test matches_expr_snapshot through combinator matching
        let child = make_node(NodeKind::Caption);
        let parent = NodeSnapshot {
            kind: NodeKind::Figure,
            id: String::new(),
            auto_id: true,
            classes: vec!["wide".to_string()],
            properties: HashMap::new(),
        };

        // Parent must match Or expression
        let sel = parse_selector("figure > caption").unwrap();
        assert!(matches_selector(&sel, &child, &[parent.clone()]));

        // Combinator in ancestor context returns false
        let sel2 = parse_selector("section ~> figure > caption").unwrap();
        // This tests multi-level combinator (nested combinator in snapshot = false)
        assert!(!matches_selector(&sel2, &child, &[parent]));
    }

    #[test]
    fn matches_compound_snapshot() {
        let child = make_node(NodeKind::Caption);
        let parent = NodeSnapshot {
            kind: NodeKind::Figure,
            id: "fig1".to_string(),
            auto_id: false,
            classes: vec!["wide".to_string()],
            properties: HashMap::new(),
        };

        // Kind + class
        let sel = parse_selector("figure.wide > caption").unwrap();
        assert!(matches_selector(&sel, &child, &[parent.clone()]));

        // Kind + wrong class
        let sel2 = parse_selector("figure.narrow > caption").unwrap();
        assert!(!matches_selector(&sel2, &child, &[parent.clone()]));

        // Kind + id
        let sel3 = parse_selector("figure#fig1 > caption").unwrap();
        assert!(matches_selector(&sel3, &child, &[parent.clone()]));

        // Kind + wrong id
        let sel4 = parse_selector("figure#fig2 > caption").unwrap();
        assert!(!matches_selector(&sel4, &child, &[parent.clone()]));

        // Auto-id should not match
        let auto_parent = NodeSnapshot {
            auto_id: true,
            ..parent
        };
        assert!(!matches_selector(&sel3, &child, &[auto_parent]));
    }

    // ── Parser edge cases ──

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse_selector("").is_none());
        assert!(parse_selector("   ").is_none());
    }

    #[test]
    fn parse_invalid_kind_returns_none() {
        assert!(parse_selector("nonexistent").is_none());
    }

    #[test]
    fn parse_starts_with_attr() {
        let sel = parse_selector("[lang=^\"fig\"]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        let Some(AttrExpr::Pred(pred)) = &c.attr_expr else {
            panic!("expected attr")
        };
        assert!(matches!(&pred.op, AttrOp::StartsWith(v) if v == "fig"));
    }

    #[test]
    fn parse_single_element_tuple() {
        let sel = parse_selector("[lang=(\"rust\")]").unwrap();
        let Selector::Expr(SelectorExpr::Simple(c)) = sel else {
            panic!("expected Simple")
        };
        // Single-element tuple should collapse to a simple Pred, not Or
        let Some(AttrExpr::Pred(pred)) = &c.attr_expr else {
            panic!("expected Pred, got {:?}", c.attr_expr)
        };
        assert!(matches!(&pred.op, AttrOp::Eq(v) if v == "rust"));
    }

    #[test]
    fn parse_unclosed_bracket_returns_none() {
        assert!(parse_selector("[lang=\"rust\"").is_none());
    }

    #[test]
    fn parse_nested_brackets() {
        // Not typical but should not panic
        let result = parse_selector("[a=\"[b]\"]");
        assert!(result.is_some());
    }

    #[test]
    fn node_kind_name_covers_all() {
        // Test a sampling of node kinds for coverage
        assert_eq!(node_kind_name(&NodeKind::Paragraph), "Paragraph");
        assert_eq!(node_kind_name(&NodeKind::BlockQuote), "BlockQuote");
        assert_eq!(
            node_kind_name(&NodeKind::List {
                ordered: false,
                start: None
            }),
            "List"
        );
        assert_eq!(node_kind_name(&NodeKind::ListItem), "ListItem");
        assert_eq!(node_kind_name(&NodeKind::Table), "Table");
        assert_eq!(node_kind_name(&NodeKind::TableRow), "TableRow");
        assert_eq!(node_kind_name(&NodeKind::TableCell), "TableCell");
        assert_eq!(node_kind_name(&NodeKind::ThematicBreak), "ThematicBreak");
        assert_eq!(node_kind_name(&NodeKind::Figure), "Figure");
        assert_eq!(node_kind_name(&NodeKind::Caption), "Caption");
        assert_eq!(
            node_kind_name(&NodeKind::Math {
                display: false,
                source: String::new()
            }),
            "Math"
        );
        assert_eq!(node_kind_name(&NodeKind::Text("x".into())), "Text");
        assert_eq!(node_kind_name(&NodeKind::Code("x".into())), "Code");
        assert_eq!(node_kind_name(&NodeKind::Emphasis), "Emphasis");
        assert_eq!(node_kind_name(&NodeKind::Strong), "Strong");
        assert_eq!(node_kind_name(&NodeKind::Strikethrough), "Strikethrough");
        assert_eq!(
            node_kind_name(&NodeKind::Link { url: String::new() }),
            "Link"
        );
        assert_eq!(
            node_kind_name(&NodeKind::Image {
                url: String::new(),
                alt: String::new()
            }),
            "Image"
        );
        assert_eq!(
            node_kind_name(&NodeKind::Citation { key: String::new() }),
            "Citation"
        );
        assert_eq!(
            node_kind_name(&NodeKind::CrossRef {
                target: String::new()
            }),
            "CrossRef"
        );
        assert_eq!(node_kind_name(&NodeKind::SoftBreak), "SoftBreak");
        assert_eq!(node_kind_name(&NodeKind::HardBreak), "HardBreak");
        assert_eq!(
            node_kind_name(&NodeKind::Space {
                spacing: crate::ast::Spacing::Auto
            }),
            "Space"
        );
        assert_eq!(
            node_kind_name(&NodeKind::Custom {
                content_type: String::new()
            }),
            "Custom"
        );
        assert_eq!(
            node_kind_name(&NodeKind::FileScope {
                source: String::new()
            }),
            "FileScope"
        );
    }

    #[test]
    fn combinator_op_token_len() {
        assert_eq!(CombinatorOp::Child.token_len(), 1);
        assert_eq!(CombinatorOp::Descendant.token_len(), 2);
    }
}
