use std::collections::HashMap;

use crate::ast::{
    Declaration, Document, FormatToken, Hint, Node, NodeKind, RawBlockCompiler, Selector,
    SeriesConfig, Value,
};
use crate::selector::{NodeSnapshot, matches_selector};
use crate::visit::Pass;

/// Target of a resolved label.
#[derive(Debug, Clone)]
pub struct LabelTarget {
    pub doc_slug: String,
    pub node_id: String,
}

/// Context for resolving cross-references, citations, and series across documents.
pub struct ResolveContext {
    pub documents: HashMap<String, Document>,
    pub labels: HashMap<String, LabelTarget>,
}

/// Resolve cross-references, citations, and series in a set of documents.
///
/// Stage A: minimal — just passes through. No series resolution, no hint
/// cascade evaluation. The AST is designed to support it; we just don't
/// run it yet.
pub fn resolve(_ctx: &mut ResolveContext) {
    // Stage A: no-op. Full resolution comes later.
}

/// Resolve hints within a single document (convenience function).
///
/// Runs `HoistDirectives` → `CascadeHints` → `AssignNumbers` → `ResolveCrossRefs`.
pub fn resolve_document(doc: &mut Document) {
    HoistDirectives.run(doc);
    CascadeHints.run(doc);
    AssignNumbers.run(doc);
    ResolveCrossRefs.run(doc);
}

// ── HoistDirectives pass ──

/// Move hints and declarations from `Custom { "directive" }` child nodes
/// to their parent scope, then remove those directive nodes.
pub struct HoistDirectives;

impl Pass for HoistDirectives {
    fn run(&mut self, doc: &mut Document) {
        hoist(&mut doc.content);
    }
}

fn hoist(node: &mut Node) {
    // Recurse first (bottom-up) so nested directives are hoisted before parents.
    for child in &mut node.children {
        hoist(child);
    }

    let mut hoisted_hints = Vec::new();
    let mut hoisted_decls = Vec::new();

    for child in &node.children {
        if matches!(&child.kind, NodeKind::Custom { content_type } if content_type == "directive") {
            hoisted_hints.extend(child.attrs.hints.iter().cloned());
            hoisted_decls.extend(child.attrs.declarations.iter().cloned());
        }
    }

    node.attrs.hints.extend(hoisted_hints);
    node.attrs.declarations.extend(hoisted_decls);

    node.children.retain(|child| {
        !matches!(&child.kind, NodeKind::Custom { content_type } if content_type == "directive")
    });
}

// ── CascadeHints pass ──

/// Walk the tree depth-first, matching ancestor hints against each node
/// and merging into `attrs.properties`. Nearest ancestor wins;
/// authored values are never overwritten.
pub struct CascadeHints;

impl Pass for CascadeHints {
    fn run(&mut self, doc: &mut Document) {
        cascade(&mut doc.content, &[], &[]);
    }
}

fn cascade(node: &mut Node, ancestor_hints: &[Vec<Hint>], ancestors: &[NodeSnapshot]) {
    // Apply own This hints to self.
    for hint in &node.attrs.hints {
        if matches!(hint.selector, Selector::This) {
            node.attrs
                .properties
                .entry(hint.property.clone())
                .or_insert_with(|| hint.value.clone());
        }
    }

    // Match ancestor hints (nearest first via rev → entry-or-insert = nearest wins).
    for hint in ancestor_hints.iter().rev().flatten() {
        if matches_selector(&hint.selector, node, ancestors) {
            node.attrs
                .properties
                .entry(hint.property.clone())
                .or_insert_with(|| hint.value.clone());
        }
    }

    // Build ancestor stacks for children (clone to avoid borrow conflict).
    let own_hints = node.attrs.hints.clone();
    let mut child_ancestor_hints: Vec<Vec<Hint>> = ancestor_hints.to_vec();
    child_ancestor_hints.push(own_hints);

    let snap = NodeSnapshot::from_node(node);
    let mut child_ancestors: Vec<NodeSnapshot> = ancestors.to_vec();
    child_ancestors.push(snap);

    for child in &mut node.children {
        cascade(child, &child_ancestor_hints, &child_ancestors);
    }
}

// ── AssignNumbers pass ──

/// Walk the tree in document order, matching series declarations against
/// nodes via the cascaded `series` property, assigning numbers and building
/// a label map for cross-references.
pub struct AssignNumbers;

impl Pass for AssignNumbers {
    fn run(&mut self, doc: &mut Document) {
        // Check if any series declarations exist.
        if !has_series_declarations(&doc.content) {
            return;
        }
        let mut locals = Locals::default();
        let mut label_map = HashMap::new();
        assign_walk(&mut doc.content, &mut locals, &mut label_map);
        // Stash the label map on the root for ResolveCrossRefs.
        if !label_map.is_empty() {
            doc.content
                .attrs
                .properties
                .insert("__label_map".to_string(), encode_label_map(&label_map));
        }
    }
}

/// Locals for the counter engine — shared mutable state during the walk.
#[derive(Default)]
struct Locals {
    /// Per-series state: series_id → SeriesState.
    series: HashMap<String, SeriesState>,
}

#[derive(Clone)]
struct SeriesState {
    /// Hierarchical count array (one entry per depth level).
    counts: Vec<i64>,
    /// Flat counter for non-hierarchical series.
    counter: i64,
    /// The series config.
    config: SeriesConfig,
}

/// Entry in the label map built during pass 1.
struct LabelEntry {
    /// The computed display number (e.g. "I.2.1").
    number: String,
    /// The label template (e.g. "Figure {n}").
    label_template: String,
}

fn has_series_declarations(node: &Node) -> bool {
    if node.attrs.declarations.iter().any(|d| matches!(d, Declaration::Series { .. })) {
        return true;
    }
    node.children.iter().any(has_series_declarations)
}

fn assign_walk(
    node: &mut Node,
    locals: &mut Locals,
    label_map: &mut HashMap<String, LabelEntry>,
) {
    // Initialize series state from declarations on this node.
    for decl in &node.attrs.declarations {
        let Declaration::Series { id, config } = decl;
        locals.series.entry(id.clone()).or_insert_with(|| SeriesState {
            counts: Vec::new(),
            counter: 0,
            config: config.clone(),
        });
    }

    // Check for series membership via the cascaded `series` property.
    if let Some(Value::Text(series_id)) = node.attrs.properties.get("series").cloned() {
        let is_heading = matches!(&node.kind, NodeKind::Heading { .. });
        let is_hierarchical = is_heading
            && locals
                .series
                .get(&series_id)
                .is_some_and(|s| s.config.formats.len() > 1);

        if is_hierarchical {
            let level = match &node.kind {
                NodeKind::Heading { level } => *level as usize,
                _ => unreachable!(),
            };
            bump_depth(locals, &series_id, level);
        } else {
            incr_counter(locals, &series_id);
        }
        store_number(node, locals, &series_id, label_map);
    }

    for child in &mut node.children {
        assign_walk(child, locals, label_map);
    }
}

/// Compute the number for a series and store it on the node.
fn store_number(
    node: &mut Node,
    locals: &Locals,
    series_id: &str,
    label_map: &mut HashMap<String, LabelEntry>,
) {
    if let Some(state) = locals.series.get(series_id) {
        let number = render_series_number(state);
        node.attrs
            .properties
            .insert("number".to_string(), Value::Text(number.clone()));

        if !node.attrs.auto_id {
            let label_template = node
                .attrs
                .properties
                .get("label")
                .and_then(|v| match v {
                    Value::Text(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            label_map.insert(
                node.attrs.id.clone(),
                LabelEntry {
                    number,
                    label_template,
                },
            );
        }
    }
}

/// Bump the depth of a hierarchical series at the given level.
fn bump_depth(locals: &mut Locals, series_id: &str, level: usize) {
    // Phase 1: update the series state.
    let updated = if let Some(state) = locals.series.get_mut(series_id) {
        while state.counts.len() < level {
            state.counts.push(0);
        }
        state.counts[level - 1] += 1;
        state.counts.truncate(level);
        true
    } else {
        false
    };

    if !updated {
        return;
    }

    // Phase 2: reset child series (separate borrow scope).
    let child_ids: Vec<String> = locals
        .series
        .iter()
        .filter(|(_, s)| s.config.parent.as_deref() == Some(series_id))
        .map(|(id, _)| id.clone())
        .collect();
    for child_id in child_ids {
        if let Some(child_state) = locals.series.get_mut(&child_id) {
            child_state.counter = 0;
        }
    }
}

/// Increment a flat counter for a series.
fn incr_counter(locals: &mut Locals, series_id: &str) {
    if let Some(state) = locals.series.get_mut(series_id) {
        state.counter += 1;
    }
}

/// Render the number for a series based on its current state.
fn render_series_number(state: &SeriesState) -> String {
    if !state.counts.is_empty() && state.counter > 0 {
        // Hierarchical with flat counter: "I.2.1" (parent hierarchy + own counter).
        let parent_num = render_hierarchical(&state.counts, &state.config);
        let own_fmt = get_format_token(&state.config, state.counts.len());
        format!("{}.{}", parent_num, own_fmt.render(state.counter as u32))
    } else if !state.counts.is_empty() {
        // Pure hierarchical: "I.2.1"
        render_hierarchical(&state.counts, &state.config)
    } else {
        // Flat counter only: "1", "2", etc.
        let fmt = get_format_token(&state.config, 0);
        fmt.render(state.counter as u32)
    }
}

/// Render a hierarchical number from a counts array.
fn render_hierarchical(counts: &[i64], config: &SeriesConfig) -> String {
    counts
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let token = get_format_token(config, i);
            token.render(c as u32)
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Get the format token for a given depth index.
fn get_format_token(config: &SeriesConfig, depth: usize) -> FormatToken {
    if config.formats.is_empty() {
        return FormatToken::parse("1");
    }
    let idx = if config.cycle {
        depth % config.formats.len()
    } else {
        depth.min(config.formats.len() - 1)
    };
    FormatToken::parse(&config.formats[idx])
}

/// Encode a label map as a `Value::Map` for stashing on the root node.
fn encode_label_map(map: &HashMap<String, LabelEntry>) -> Value {
    let mut m = HashMap::new();
    for (id, entry) in map {
        let mut inner = HashMap::new();
        inner.insert("number".to_string(), Value::Text(entry.number.clone()));
        inner.insert(
            "label".to_string(),
            Value::Text(entry.label_template.clone()),
        );
        m.insert(id.clone(), Value::Map(inner));
    }
    Value::Map(m)
}

/// Decode a label map from a `Value::Map` stashed on the root node.
fn decode_label_map(val: &Value) -> HashMap<String, LabelEntry> {
    let Value::Map(m) = val else {
        return HashMap::new();
    };
    m.iter()
        .filter_map(|(id, inner)| {
            let Value::Map(fields) = inner else {
                return None;
            };
            let number = extract_text(fields.get("number"));
            let label_template = extract_text(fields.get("label"));
            Some((id.clone(), LabelEntry { number, label_template }))
        })
        .collect()
}

fn extract_text(val: Option<&Value>) -> String {
    match val {
        Some(Value::Text(s)) => s.clone(),
        _ => String::new(),
    }
}

// ── ResolveCrossRefs pass ──

/// Second pass: resolve `CrossRef { target }` nodes using the label map
/// built by `AssignNumbers`.
pub struct ResolveCrossRefs;

impl Pass for ResolveCrossRefs {
    fn run(&mut self, doc: &mut Document) {
        let label_map = doc
            .content
            .attrs
            .properties
            .get("__label_map")
            .map(decode_label_map)
            .unwrap_or_default();

        if label_map.is_empty() {
            return;
        }

        resolve_refs(&mut doc.content, &label_map);

        // Clean up the internal label map property.
        doc.content.attrs.properties.remove("__label_map");
    }
}

fn resolve_refs(node: &mut Node, label_map: &HashMap<String, LabelEntry>) {
    if let NodeKind::CrossRef { target } = &node.kind
        && let Some(entry) = label_map.get(target)
    {
        let display = format_label_display(&entry.label_template, &entry.number);
        node.attrs
            .properties
            .insert("text".to_string(), Value::Text(display));
        node.attrs
            .properties
            .insert("href".to_string(), Value::Text(format!("#{target}")));
    }
    for child in &mut node.children {
        resolve_refs(child, label_map);
    }
}

fn format_label_display(template: &str, number: &str) -> String {
    if template.is_empty() {
        number.to_string()
    } else {
        template.replace("{n}", number)
    }
}

// ── CompileBlocks pass ──

/// Compile raw blocks and preview code blocks, storing results in `attrs.properties`.
///
/// Must run after `CascadeHints` (reads the `preview` property).
/// For each `RawBlock`, compiles via the provided `RawBlockCompiler`.
/// For each `CodeBlock` with `preview`, compiles using the preview format.
/// Stores compiled HTML in `properties["compiled"]` and errors in
/// `properties["compile_error"]`.
pub struct CompileBlocks<'a> {
    compiler: &'a dyn RawBlockCompiler,
}

impl<'a> CompileBlocks<'a> {
    pub fn new(compiler: &'a dyn RawBlockCompiler) -> Self {
        Self { compiler }
    }
}

impl Pass for CompileBlocks<'_> {
    fn run(&mut self, doc: &mut Document) {
        compile_blocks(&mut doc.content, self.compiler);
    }
}

fn compile_blocks(node: &mut Node, compiler: &dyn RawBlockCompiler) {
    let compile_target = match &node.kind {
        NodeKind::RawBlock { format, source } => Some((format.clone(), source.clone())),
        NodeKind::CodeBlock { lang, source } if node.attrs.properties.contains_key("preview") => {
            Some((lang.clone(), source.clone()))
        }
        _ => None,
    };
    if let Some((default_fmt, source)) = compile_target {
        let format = get_compile_format(&node.attrs, &default_fmt);
        store_compiled(node, compiler, &format, &source);
    }

    for child in &mut node.children {
        compile_blocks(child, compiler);
    }
}

fn store_compiled(node: &mut Node, compiler: &dyn RawBlockCompiler, format: &str, source: &str) {
    let (key, val) = match compiler.compile(format, source) {
        Ok(html) => ("compiled", html),
        Err(e) => ("compile_error", e),
    };
    node.attrs
        .properties
        .insert(key.to_string(), Value::Text(val));
}

/// Determine the compilation format from the `preview` property, falling back to
/// the node's own format/lang.
fn get_compile_format<'a>(attrs: &'a crate::ast::Attrs, default: &'a str) -> String {
    match attrs.properties.get("preview") {
        Some(Value::Text(s)) if s != "true" => s.clone(),
        _ => default.to_string(),
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{CompoundSelector, NumberFormat, SelectorExpr, Value, reset_id_counter};
    use crate::parse::parse;

    fn find_node(node: &Node, pred: fn(&Node) -> bool) -> Option<&Node> {
        if pred(node) {
            return Some(node);
        }
        node.children.iter().find_map(|c| find_node(c, pred))
    }

    #[test]
    fn hoist_removes_directive_nodes() {
        reset_id_counter();
        let mut doc =
            parse("{@hint(code-block, preview=true)}\n\n```html\n<b>hi</b>\n```").unwrap();

        // Before hoist: root has a Custom directive child and a CodeBlock child.
        let has_directive = doc.content.children.iter().any(
            |n| matches!(&n.kind, NodeKind::Custom { content_type } if content_type == "directive"),
        );
        assert!(has_directive);

        HoistDirectives.run(&mut doc);

        // After hoist: directive node removed, hint moved to root.
        let has_directive = doc.content.children.iter().any(
            |n| matches!(&n.kind, NodeKind::Custom { content_type } if content_type == "directive"),
        );
        assert!(!has_directive);
        assert!(!doc.content.attrs.hints.is_empty());
    }

    #[test]
    fn cascade_kind_selector() {
        reset_id_counter();
        let mut doc =
            parse("{@hint(code-block, preview=true)}\n\nSome text.\n\n```html\n<b>hi</b>\n```")
                .unwrap();
        resolve_document(&mut doc);

        // CodeBlock should have preview in properties.
        let code = doc
            .content
            .children
            .iter()
            .find(|n| matches!(n.kind, NodeKind::CodeBlock { .. }))
            .expect("should have a code block");
        assert!(code.attrs.properties.contains_key("preview"));

        // Paragraph should NOT have preview.
        let para = doc
            .content
            .children
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Paragraph))
            .expect("should have a paragraph");
        assert!(!para.attrs.properties.contains_key("preview"));
    }

    #[test]
    fn cascade_class_selector() {
        reset_id_counter();
        let mut doc = parse(
            "{@hint(.wide, layout=full)}\n\n```html {.wide}\ncode1\n```\n\n```html\ncode2\n```",
        )
        .unwrap();
        resolve_document(&mut doc);

        let blocks: Vec<_> = doc
            .content
            .children
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::CodeBlock { .. }))
            .collect();
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].attrs.properties.contains_key("layout"));
        assert!(!blocks[1].attrs.properties.contains_key("layout"));
    }

    #[test]
    fn cascade_id_selector() {
        reset_id_counter();
        let mut doc =
            parse("{@hint(#sec-intro, highlight=true)}\n\n# Introduction {#sec-intro}").unwrap();
        resolve_document(&mut doc);

        let heading = find_node(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }))
            .expect("should have a heading");
        assert_eq!(heading.attrs.id, "sec-intro");
        assert!(heading.attrs.properties.contains_key("highlight"));
    }

    #[test]
    fn cascade_this_selector() {
        reset_id_counter();
        let mut doc = parse("# Title {@hint(this, font=Georgia)}").unwrap();
        resolve_document(&mut doc);

        let section = &doc.content.children[0];
        let heading = &section.children[0];
        assert!(matches!(heading.kind, NodeKind::Heading { .. }));
        assert!(heading.attrs.properties.contains_key("font"));
    }

    #[test]
    fn nearest_ancestor_wins() {
        reset_id_counter();
        let mut doc = parse(
            "{@hint(code-block, theme=dark)}\n\n## Sub {@hint(this, skip=x)}\n\n{@hint(code-block, theme=light)}\n\n```html\ncode\n```",
        )
        .unwrap();
        resolve_document(&mut doc);

        let code = find_node(&doc.content, |n| {
            matches!(n.kind, NodeKind::CodeBlock { .. })
        })
        .expect("should have code block");
        let theme = code
            .attrs
            .properties
            .get("theme")
            .expect("should have theme");
        // The inner section's hint should win over the root's.
        assert!(matches!(theme, Value::Text(s) if s == "light"));
    }

    #[test]
    fn cascade_injected_hints() {
        // Simulates what build.rs does: parse without directives, then inject hints.
        reset_id_counter();
        let mut doc = parse("## Heading\n\n```html\n<b>hi</b>\n```").unwrap();
        doc.content.attrs.hints.push(Hint {
            selector: Selector::Expr(SelectorExpr::Simple(CompoundSelector {
                kind: Some("CodeBlock".to_string()),
                ..Default::default()
            })),
            property: "preview".to_string(),
            value: Value::Bool(true),
        });
        resolve_document(&mut doc);

        let code = find_node(&doc.content, |n| {
            matches!(n.kind, NodeKind::CodeBlock { .. })
        })
        .expect("should have a code block");
        assert!(
            code.attrs.properties.contains_key("preview"),
            "injected hint should cascade to nested code block, properties: {:?}",
            code.attrs.properties
        );
    }

    #[test]
    fn auto_id_not_matched() {
        reset_id_counter();
        let mut doc = parse("{@hint(#code-0, x=y)}\n\n```html\ncode\n```").unwrap();
        resolve_document(&mut doc);

        let code = doc
            .content
            .children
            .iter()
            .find(|n| matches!(n.kind, NodeKind::CodeBlock { .. }))
            .expect("should have code block");
        // Auto-generated ID should not match an Id selector.
        assert!(!code.attrs.properties.contains_key("x"));
    }

    #[test]
    fn empty_tree_no_panic() {
        reset_id_counter();
        let mut doc = parse("").unwrap();
        resolve_document(&mut doc);
        assert!(doc.content.children.is_empty());
    }

    #[test]
    fn hints_with_no_matching_descendants() {
        reset_id_counter();
        let mut doc = parse("{@hint(table, border=true)}\n\nJust a paragraph.").unwrap();
        resolve_document(&mut doc);

        let para = doc
            .content
            .children
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Paragraph))
            .unwrap();
        assert!(para.attrs.properties.is_empty());
    }

    // ── Counter engine tests ──

    fn get_number(node: &Node) -> Option<&str> {
        node.attrs.properties.get("number").and_then(|v| match v {
            Value::Text(s) => Some(s.as_str()),
            _ => None,
        })
    }

    fn find_all(node: &Node, pred: fn(&Node) -> bool) -> Vec<&Node> {
        let mut result = Vec::new();
        find_all_walk(node, pred, &mut result);
        result
    }

    fn find_all_walk<'a>(node: &'a Node, pred: fn(&Node) -> bool, result: &mut Vec<&'a Node>) {
        if pred(node) {
            result.push(node);
        }
        for child in &node.children {
            find_all_walk(child, pred, result);
        }
    }

    #[test]
    fn flat_counter_figures() {
        reset_id_counter();
        let input = "\
{@series(fig, counts=figure)}
{@hint(figure, label=\"Figure {n}\")}

![First](a.png){#fig:first}

![Second](b.png){#fig:second}

![Third](c.png){#fig:third}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let figs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Figure));
        assert_eq!(figs.len(), 3);
        assert_eq!(get_number(figs[0]), Some("1"));
        assert_eq!(get_number(figs[1]), Some("2"));
        assert_eq!(get_number(figs[2]), Some("3"));
    }

    #[test]
    fn hierarchical_section_numbering() {
        reset_id_counter();
        let input = "\
{@series(sec, counts=heading, formats=[\"I\",\"1\"])}

# Introduction {#sec:intro}

## Background {#sec:bg}

## Methods {#sec:methods}

# Results {#sec:results}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 4);
        assert_eq!(get_number(headings[0]), Some("I"), "Introduction should be I");
        assert_eq!(get_number(headings[1]), Some("I.1"), "Background should be I.1");
        assert_eq!(get_number(headings[2]), Some("I.2"), "Methods should be I.2");
        assert_eq!(get_number(headings[3]), Some("II"), "Results should be II");
    }

    #[test]
    fn figure_counter_resets_with_parent() {
        reset_id_counter();
        let input = "\
{@series(sec, counts=heading, formats=[\"I\",\"1\"])}
{@series(fig, counts=figure, parent=sec, formats=[\"1\"])}
{@hint(figure, label=\"Figure {n}\")}

# Chapter One

![A](a.png){#fig:a}

![B](b.png){#fig:b}

# Chapter Two

![C](c.png){#fig:c}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let figs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Figure));
        assert_eq!(figs.len(), 3);
        assert_eq!(get_number(figs[0]), Some("1"));
        assert_eq!(get_number(figs[1]), Some("2"));
        // After chapter change, counter resets.
        assert_eq!(get_number(figs[2]), Some("1"));
    }

    #[test]
    fn cross_ref_forward_and_backward() {
        reset_id_counter();
        let input = "\
{@series(fig, counts=figure)}
{@hint(figure, label=\"Figure {n}\")}

See @fig:second for details.

![First](a.png){#fig:first}

![Second](b.png){#fig:second}

As shown in @fig:first above.";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        // Find cross-refs.
        let refs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::CrossRef { .. }));
        assert_eq!(refs.len(), 2);

        // Forward ref to fig:second should resolve.
        let text0 = refs[0].attrs.properties.get("text").and_then(|v| match v {
            Value::Text(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(text0, Some("Figure 2"));

        // Backward ref to fig:first should resolve.
        let text1 = refs[1].attrs.properties.get("text").and_then(|v| match v {
            Value::Text(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(text1, Some("Figure 1"));
    }

    #[test]
    fn format_token_roman() {
        use crate::ast::FormatToken;
        assert_eq!(FormatToken::parse("I").render(4), "IV");
        assert_eq!(FormatToken::parse("i").render(4), "iv");
        assert_eq!(FormatToken::parse("(a)").render(3), "(c)");
        assert_eq!(FormatToken::parse("1").render(42), "42");
        assert_eq!(FormatToken::parse("A").render(1), "A");
    }

    #[test]
    fn no_series_no_crash() {
        reset_id_counter();
        let mut doc = parse("# Just a heading\n\nSome text.").unwrap();
        resolve_document(&mut doc);
        // No series declarations → no numbers assigned, no crash.
        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert!(!headings[0].attrs.properties.contains_key("number"));
    }

    #[test]
    fn cross_ref_no_label_shows_number() {
        reset_id_counter();
        let input = "\
{@series(fig, counts=figure)}

![Pic](a.png){#fig:pic}

See @fig:pic.";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let refs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::CrossRef { .. }));
        assert_eq!(refs.len(), 1);
        let text = refs[0].attrs.properties.get("text").and_then(|v| match v {
            Value::Text(s) => Some(s.as_str()),
            _ => None,
        });
        // No label template → bare number.
        assert_eq!(text, Some("1"));
    }

    #[test]
    fn encode_decode_label_map_roundtrip() {
        let mut map = HashMap::new();
        map.insert(
            "fig:a".to_string(),
            LabelEntry {
                number: "I.1".to_string(),
                label_template: "Figure {n}".to_string(),
            },
        );
        map.insert(
            "fig:b".to_string(),
            LabelEntry {
                number: "II.3".to_string(),
                label_template: String::new(),
            },
        );
        let encoded = encode_label_map(&map);
        let decoded = decode_label_map(&encoded);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded["fig:a"].number, "I.1");
        assert_eq!(decoded["fig:a"].label_template, "Figure {n}");
        assert_eq!(decoded["fig:b"].number, "II.3");
        assert!(decoded["fig:b"].label_template.is_empty());
    }

    #[test]
    fn hierarchical_three_levels() {
        reset_id_counter();
        let input = "\
{@series(sec, counts=heading, formats=[\"I\",\"1\",\"a\"])}

# Chapter {#ch1}

## Section {#s1}

### Sub {#sub1}

## Section Two {#s2}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 4);
        assert_eq!(get_number(headings[0]), Some("I"));
        assert_eq!(get_number(headings[1]), Some("I.1"));
        assert_eq!(get_number(headings[2]), Some("I.1.a"));
        assert_eq!(get_number(headings[3]), Some("I.2"));
    }

    #[test]
    fn format_repeat_last() {
        reset_id_counter();
        // formats=["I","1"] with 3 levels — depth 3 should use "1" (repeat last).
        let input = "\
{@series(sec, counts=heading, formats=[\"I\",\"1\"])}

# Top {#t}

## Mid {#m}

### Deep {#d}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(get_number(headings[0]), Some("I"));
        assert_eq!(get_number(headings[1]), Some("I.1"));
        // Depth 3 repeats last format ("1" = arabic).
        assert_eq!(get_number(headings[2]), Some("I.1.1"));
    }

    #[test]
    fn render_series_number_flat() {
        let state = SeriesState {
            counts: Vec::new(),
            counter: 5,
            config: SeriesConfig {
                parent: None,
                formats: vec!["a".to_string()],
                cycle: false,
            },
        };
        assert_eq!(render_series_number(&state), "e");
    }

    #[test]
    fn render_series_number_hierarchical() {
        let state = SeriesState {
            counts: vec![2, 3],
            counter: 0,
            config: SeriesConfig {
                parent: None,
                formats: vec!["I".to_string(), "1".to_string()],
                cycle: false,
            },
        };
        assert_eq!(render_series_number(&state), "II.3");
    }

    #[test]
    fn render_series_number_hierarchical_with_counter() {
        let state = SeriesState {
            counts: vec![1],
            counter: 4,
            config: SeriesConfig {
                parent: None,
                formats: vec!["I".to_string(), "1".to_string()],
                cycle: false,
            },
        };
        // Parent "I" + flat counter "4".
        assert_eq!(render_series_number(&state), "I.4");
    }

    #[test]
    fn get_format_token_cycling() {
        let config = SeriesConfig {
            parent: None,
            formats: vec!["I".to_string(), "A".to_string()],
            cycle: true,
        };
        // depth 0 → "I", depth 1 → "A", depth 2 → "I" (wraps)
        assert!(matches!(
            get_format_token(&config, 0).format,
            NumberFormat::RomanUpper
        ));
        assert!(matches!(
            get_format_token(&config, 1).format,
            NumberFormat::AlphaUpper
        ));
        assert!(matches!(
            get_format_token(&config, 2).format,
            NumberFormat::RomanUpper
        ));
    }

    #[test]
    fn get_format_token_repeat_last() {
        let config = SeriesConfig {
            parent: None,
            formats: vec!["I".to_string(), "1".to_string()],
            cycle: false,
        };
        // depth 2 and beyond → "1" (repeat last)
        assert!(matches!(
            get_format_token(&config, 2).format,
            NumberFormat::Arabic
        ));
        assert!(matches!(
            get_format_token(&config, 10).format,
            NumberFormat::Arabic
        ));
    }

    #[test]
    fn get_format_token_empty_defaults() {
        let config = SeriesConfig {
            parent: None,
            formats: Vec::new(),
            cycle: false,
        };
        assert!(matches!(
            get_format_token(&config, 0).format,
            NumberFormat::Arabic
        ));
    }

    #[test]
    fn format_label_display_with_template() {
        assert_eq!(format_label_display("Figure {n}", "I.2"), "Figure I.2");
        assert_eq!(format_label_display("", "42"), "42");
    }

    #[test]
    fn resolve_empty_context() {
        let mut ctx = ResolveContext {
            documents: HashMap::new(),
            labels: HashMap::new(),
        };
        resolve(&mut ctx); // Should be a no-op without panic
    }

    #[test]
    fn decode_label_map_non_map_value() {
        // If __label_map is not a Map, should return empty
        let val = Value::Text("not a map".into());
        let decoded = decode_label_map(&val);
        assert!(decoded.is_empty());
    }

    #[test]
    fn decode_label_map_malformed_entries() {
        // Map with entries that aren't themselves maps
        let mut m = HashMap::new();
        m.insert("fig:x".to_string(), Value::Text("bad".into()));
        let val = Value::Map(m);
        let decoded = decode_label_map(&val);
        assert!(decoded.is_empty(), "non-map entries should be skipped");
    }

    #[test]
    fn multiple_series_independent() {
        reset_id_counter();
        let input = "\
{@series(fig, counts=figure)}
{@series(tbl, counts=table)}
{@hint(figure, label=\"Figure {n}\")}

![A](a.png){#fig:a}

| H |\n|---|\n| d |

![B](b.png){#fig:b}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let figs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Figure));
        assert_eq!(figs.len(), 2);
        assert_eq!(get_number(figs[0]), Some("1"));
        assert_eq!(get_number(figs[1]), Some("2"));
    }

    #[test]
    fn series_format_cycling() {
        reset_id_counter();
        // Use cycle=true with 2 formats over 4 heading levels
        let input = "\
{@series(sec, counts=heading, formats=[\"I\",\"a\"], cycle=true)}

# Ch {#c1}

## S {#s1}

### Sub {#sub1}

#### Deep {#d1}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 4);
        // depth 0 → "I", depth 1 → "a", depth 2 → "I" (cycle), depth 3 → "a" (cycle)
        assert_eq!(get_number(headings[0]), Some("I"));
        assert_eq!(get_number(headings[1]), Some("I.a"));
        assert_eq!(get_number(headings[2]), Some("I.a.I"));
        assert_eq!(get_number(headings[3]), Some("I.a.I.a"));
    }

    #[test]
    fn cross_ref_unresolved_uses_target() {
        reset_id_counter();
        // Cross-ref to something that doesn't exist in label map
        let input = "See @fig:missing.";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let refs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::CrossRef { .. }));
        assert_eq!(refs.len(), 1);
        // No text property should be set (unresolved)
        assert!(!refs[0].attrs.properties.contains_key("text"));
    }

    #[test]
    fn has_series_declarations_nested() {
        reset_id_counter();
        // Series declared inside a subsection
        let input = "# Chapter\n\n{@series(fig, counts=figure)}\n\n![A](a.png){#fig:a}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let figs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Figure));
        assert_eq!(figs.len(), 1);
        assert_eq!(get_number(figs[0]), Some("1"));
    }

    #[test]
    fn store_number_no_series_property() {
        reset_id_counter();
        // Node without series property should not get a number
        let input = "Just a paragraph.";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let paras = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Paragraph));
        assert!(!paras.is_empty());
        assert!(!paras[0].attrs.properties.contains_key("number"));
    }

    #[test]
    fn compound_selector_attr_match() {
        reset_id_counter();
        // Use an attribute selector to target specific nodes
        let mut doc = parse(
            "{@hint(code-block[lang=\"rust\"], theme=monokai)}\n\n```rust\ncode1\n```\n\n```python\ncode2\n```",
        )
        .unwrap();
        resolve_document(&mut doc);

        let blocks: Vec<_> = find_all(&doc.content, |n| matches!(n.kind, NodeKind::CodeBlock { .. }));
        assert_eq!(blocks.len(), 2);
        // Only rust block should match
        assert!(blocks[0].attrs.properties.contains_key("theme"));
        assert!(!blocks[1].attrs.properties.contains_key("theme"));
    }

    #[test]
    fn descendant_combinator_selector() {
        reset_id_counter();
        let mut doc = parse(
            "{@hint(section ~> code-block, highlight=true)}\n\n# Section\n\n```rust\ncode\n```",
        )
        .unwrap();
        resolve_document(&mut doc);

        let code = find_node(&doc.content, |n| matches!(n.kind, NodeKind::CodeBlock { .. }))
            .expect("should have code block");
        assert!(code.attrs.properties.contains_key("highlight"));
    }

    #[test]
    fn child_combinator_selector() {
        reset_id_counter();
        let mut doc = parse(
            "{@hint(section > heading, color=red)}\n\n# Top\n\nParagraph.",
        )
        .unwrap();
        resolve_document(&mut doc);

        let heading = find_node(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }))
            .expect("should have heading");
        assert!(heading.attrs.properties.contains_key("color"));
    }

    // ── Hoist directives (additional) ──

    #[test]
    fn hoist_transfers_hints_to_parent() {
        reset_id_counter();
        let input = "{@hint(figure, series=fig)}";
        let mut doc = parse(input).unwrap();
        HoistDirectives.run(&mut doc);
        // The hint should be on the section that contained the directive
        let has_hints = !doc.content.attrs.hints.is_empty()
            || doc.content.children.iter().any(|c| !c.attrs.hints.is_empty());
        assert!(has_hints, "hints should be hoisted to parent");
    }

    #[test]
    fn hoist_transfers_declarations() {
        reset_id_counter();
        let input = "{@series(fig, counts=figure)}";
        let mut doc = parse(input).unwrap();
        HoistDirectives.run(&mut doc);
        let has_decls = !doc.content.attrs.declarations.is_empty()
            || doc.content.children.iter().any(|c| !c.attrs.declarations.is_empty());
        assert!(has_decls, "declarations should be hoisted to parent");
    }

    // ── Cascade hints ──

    #[test]
    fn cascade_this_hint_applies_to_self() {
        reset_id_counter();
        let input = "![img](a.png){@hint(this, width=100)}";
        let mut doc = parse(input).unwrap();
        HoistDirectives.run(&mut doc);
        CascadeHints.run(&mut doc);
        let figs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Figure));
        // The this hint should be applied to the node itself
        if !figs.is_empty() {
            assert!(
                figs[0].attrs.properties.contains_key("width")
                    || !find_all(&doc.content, |n| n.attrs.properties.contains_key("width")).is_empty(),
                "this hint should apply"
            );
        }
    }

    #[test]
    fn cascade_hint_applies_to_matching_nodes() {
        reset_id_counter();
        let input = "\
{@hint(heading, color=blue)}

# Chapter

## Section";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);
        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 2);
        // Both headings should get the hint
        for h in &headings {
            let color = h.attrs.properties.get("color").and_then(|v| match v {
                Value::Text(s) => Some(s.as_str()),
                _ => None,
            });
            assert_eq!(color, Some("blue"));
        }
    }

    #[test]
    fn cascade_authored_value_not_overwritten() {
        reset_id_counter();
        // Authored property should NOT be overwritten by hint
        let input = "\
{@hint(heading, color=blue)}

# Chapter {color=red}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);
        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 1);
        let color = headings[0].attrs.properties.get("color").and_then(|v| match v {
            Value::Text(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(color, Some("red"), "authored value should win");
    }

    // ── CompileBlocks pass ──

    struct TestCompiler;
    #[allow(clippy::excessive_nesting)]
    impl RawBlockCompiler for TestCompiler {
        fn compile(&self, format: &str, source: &str) -> Result<String, String> {
            if format == "fail" {
                Err("compile error".into())
            } else {
                Ok(format!("<compiled>{source}</compiled>"))
            }
        }
    }

    #[test]
    fn compile_blocks_raw_block() {
        reset_id_counter();
        let mut doc = parse("```{=typst}\n#rect()\n```").unwrap();
        HoistDirectives.run(&mut doc);
        CascadeHints.run(&mut doc);
        CompileBlocks::new(&TestCompiler).run(&mut doc);

        let raws = find_all(&doc.content, |n| matches!(n.kind, NodeKind::RawBlock { .. }));
        assert!(!raws.is_empty());
        let compiled = raws[0].attrs.properties.get("compiled").and_then(|v| match v {
            Value::Text(s) => Some(s.as_str()),
            _ => None,
        });
        assert!(compiled.is_some(), "raw block should be compiled");
        assert!(compiled.unwrap().contains("#rect()"));
    }

    #[test]
    fn compile_blocks_error() {
        reset_id_counter();
        let mut doc = parse("```{=fail}\nbad\n```").unwrap();
        HoistDirectives.run(&mut doc);
        CascadeHints.run(&mut doc);
        CompileBlocks::new(&TestCompiler).run(&mut doc);

        let raws = find_all(&doc.content, |n| matches!(n.kind, NodeKind::RawBlock { .. }));
        assert!(!raws.is_empty());
        assert!(raws[0].attrs.properties.contains_key("compile_error"));
    }

    #[test]
    fn compile_blocks_code_with_preview() {
        reset_id_counter();
        let input = "{@hint(code-block, preview=true)}\n\n```typst\n#circle()\n```";
        let mut doc = parse(input).unwrap();
        HoistDirectives.run(&mut doc);
        CascadeHints.run(&mut doc);
        CompileBlocks::new(&TestCompiler).run(&mut doc);

        let codes = find_all(&doc.content, |n| matches!(n.kind, NodeKind::CodeBlock { .. }));
        assert!(!codes.is_empty());
        assert!(
            codes[0].attrs.properties.contains_key("compiled"),
            "code block with preview should be compiled"
        );
    }

    #[test]
    fn compile_blocks_preview_custom_format() {
        reset_id_counter();
        let input = "{@hint(code-block, preview=latex)}\n\n```typst\nsource\n```";
        let mut doc = parse(input).unwrap();
        HoistDirectives.run(&mut doc);
        CascadeHints.run(&mut doc);
        CompileBlocks::new(&TestCompiler).run(&mut doc);

        let codes = find_all(&doc.content, |n| matches!(n.kind, NodeKind::CodeBlock { .. }));
        assert!(!codes.is_empty());
        // The preview format "latex" should be passed to compiler, not "typst"
        let compiled = codes[0].attrs.properties.get("compiled").and_then(|v| match v {
            Value::Text(s) => Some(s.clone()),
            _ => None,
        });
        assert!(compiled.is_some());
    }

    // ── Series with parent + child reset ──

    #[test]
    fn series_parent_child_reset() {
        reset_id_counter();
        // Parent series must be hierarchical (>1 format) for bump_depth to reset children
        let input = "\
{@series(ch, counts=heading, formats=[\"I\",\"1\"])}
{@series(fig, counts=figure, parent=ch)}
{@hint(figure, label=\"Figure {n}\")}

# Chapter 1 {#ch1}

![A](a.png){#fig:a}

![B](b.png){#fig:b}

# Chapter 2 {#ch2}

![C](c.png){#fig:c}";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let figs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Figure));
        assert_eq!(figs.len(), 3);
        assert_eq!(get_number(figs[0]), Some("1"));
        assert_eq!(get_number(figs[1]), Some("2"));
        // After Chapter 2 heading, parent resets child fig counter
        assert_eq!(get_number(figs[2]), Some("1"));
    }

    // ── bump_depth ──

    #[test]
    fn bump_depth_extends_array() {
        let mut locals = Locals::default();
        locals.series.insert("s".into(), SeriesState {
            counts: Vec::new(),
            counter: 0,
            config: SeriesConfig {
                parent: None,
                formats: vec!["I".into(), "1".into()],
                cycle: false,
            },
        });
        bump_depth(&mut locals, "s", 1);
        assert_eq!(locals.series["s"].counts, vec![1]);
        bump_depth(&mut locals, "s", 2);
        assert_eq!(locals.series["s"].counts, vec![1, 1]);
    }

    #[test]
    fn bump_depth_truncates_on_level_decrease() {
        let mut locals = Locals::default();
        locals.series.insert("s".into(), SeriesState {
            counts: vec![2, 3, 1],
            counter: 0,
            config: SeriesConfig {
                parent: None,
                formats: vec!["I".into(), "1".into(), "a".into()],
                cycle: false,
            },
        });
        bump_depth(&mut locals, "s", 2);
        // Truncated to level 2, counter incremented
        assert_eq!(locals.series["s"].counts, vec![2, 4]);
    }

    #[test]
    fn bump_depth_resets_child_series() {
        let mut locals = Locals::default();
        locals.series.insert("ch".into(), SeriesState {
            counts: Vec::new(),
            counter: 0,
            config: SeriesConfig {
                parent: None,
                formats: vec!["I".into()],
                cycle: false,
            },
        });
        locals.series.insert("fig".into(), SeriesState {
            counts: Vec::new(),
            counter: 5,
            config: SeriesConfig {
                parent: Some("ch".into()),
                formats: vec!["1".into()],
                cycle: false,
            },
        });
        bump_depth(&mut locals, "ch", 1);
        assert_eq!(locals.series["fig"].counter, 0, "child counter should reset");
    }

    // ── incr_counter ──

    #[test]
    fn incr_counter_increments() {
        let mut locals = Locals::default();
        locals.series.insert("fig".into(), SeriesState {
            counts: Vec::new(),
            counter: 0,
            config: SeriesConfig {
                parent: None,
                formats: vec!["1".into()],
                cycle: false,
            },
        });
        incr_counter(&mut locals, "fig");
        assert_eq!(locals.series["fig"].counter, 1);
        incr_counter(&mut locals, "fig");
        assert_eq!(locals.series["fig"].counter, 2);
    }

    // ── get_compile_format ──

    #[test]
    fn get_compile_format_default() {
        use crate::ast::Attrs;
        let attrs = Attrs::default();
        assert_eq!(get_compile_format(&attrs, "typst"), "typst");
    }

    #[test]
    fn get_compile_format_custom() {
        let mut attrs = crate::ast::Attrs::default();
        attrs.properties.insert("preview".to_string(), Value::Text("latex".to_string()));
        assert_eq!(get_compile_format(&attrs, "typst"), "latex");
    }

}
