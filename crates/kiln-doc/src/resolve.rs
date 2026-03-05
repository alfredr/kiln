use std::collections::HashMap;

use crate::ast::{
    Action, Declaration, Document, FormatToken, Init, Node, NodeKind, OnDirective,
    RawBlockCompiler, Selector, Value,
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
/// Stage A: minimal — just passes through. No series resolution, no init
/// cascade evaluation. The AST is designed to support it; we just don't
/// run it yet.
pub fn resolve(_ctx: &mut ResolveContext) {
    // Stage A: no-op. Full resolution comes later.
}

/// Resolve inits within a single document (convenience function).
///
/// Runs `HoistDirectives` → `CascadeInits` → `ExecuteProgram` → `ResolveCrossRefs`.
pub fn resolve_document(doc: &mut Document) {
    HoistDirectives.run(doc);
    CascadeInits.run(doc);
    ExecuteProgram.run(doc);
    ResolveCrossRefs.run(doc);
}

// ── HoistDirectives pass ──

/// Move inits and declarations from `Custom { "directive" }` child nodes
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

    let mut hoisted_inits = Vec::new();
    let mut hoisted_decls = Vec::new();

    for child in &node.children {
        if matches!(&child.kind, NodeKind::Custom { content_type } if content_type == "directive") {
            hoisted_inits.extend(child.attrs.inits.iter().cloned());
            hoisted_decls.extend(child.attrs.declarations.iter().cloned());
        }
    }

    node.attrs.inits.extend(hoisted_inits);
    node.attrs.declarations.extend(hoisted_decls);

    node.children.retain(|child| {
        !matches!(&child.kind, NodeKind::Custom { content_type } if content_type == "directive")
    });
}

// ── CascadeInits pass ──

/// Walk the tree depth-first, matching ancestor inits against each node
/// and merging into `attrs.properties`. Nearest ancestor wins;
/// authored values are never overwritten.
pub struct CascadeInits;

impl Pass for CascadeInits {
    fn run(&mut self, doc: &mut Document) {
        cascade(&mut doc.content, &[], &[]);
    }
}

fn cascade(node: &mut Node, ancestor_inits: &[Vec<Init>], ancestors: &[NodeSnapshot]) {
    // Apply own This inits to self.
    for init in &node.attrs.inits {
        if matches!(init.selector, Selector::This) {
            node.attrs
                .properties
                .entry(init.property.clone())
                .or_insert_with(|| init.value.clone());
        }
    }

    // Match ancestor inits (nearest first via rev → entry-or-insert = nearest wins).
    for init in ancestor_inits.iter().rev().flatten() {
        if matches_selector(&init.selector, node, ancestors) {
            node.attrs
                .properties
                .entry(init.property.clone())
                .or_insert_with(|| init.value.clone());
        }
    }

    // Build ancestor stacks for children (clone to avoid borrow conflict).
    let own_inits = node.attrs.inits.clone();
    let mut child_ancestor_inits: Vec<Vec<Init>> = ancestor_inits.to_vec();
    child_ancestor_inits.push(own_inits);

    let snap = NodeSnapshot::from_node(node);
    let mut child_ancestors: Vec<NodeSnapshot> = ancestors.to_vec();
    child_ancestors.push(snap);

    for child in &mut node.children {
        cascade(child, &child_ancestor_inits, &child_ancestors);
    }
}

// ── ExecuteProgram pass (generic reactive walk engine) ──

/// Running state carried through the walk — a map of Values.
type WalkState = HashMap<String, Value>;

/// Metadata about a declared variable (format info).
struct VarMeta {
    initial: Value,
    formats: Vec<FormatToken>,
    cycle: bool,
}

/// Entry in the label map built during the walk.
struct LabelEntry {
    number: String,
    label_template: String,
}

/// Walk the tree in document order, executing variable inits and firing
/// selector-triggered actions. Replaces the old `AssignNumbers` pass.
pub struct ExecuteProgram;

impl Pass for ExecuteProgram {
    fn run(&mut self, doc: &mut Document) {
        if !has_program_declarations(&doc.content) {
            return;
        }
        let mut state = WalkState::new();
        let mut var_meta: HashMap<String, VarMeta> = HashMap::new();
        let mut triggers: Vec<OnDirective> = Vec::new();
        let mut label_map: HashMap<String, LabelEntry> = HashMap::new();

        program_walk(
            &mut doc.content,
            &mut state,
            &mut var_meta,
            &mut triggers,
            &mut label_map,
            &[],
        );

        if !label_map.is_empty() {
            doc.content
                .attrs
                .properties
                .insert("__label_map".to_string(), encode_label_map(&label_map));
        }
    }
}

/// Check if any program declarations exist in the tree.
fn has_program_declarations(node: &Node) -> bool {
    if node.attrs.declarations.iter().any(|d| {
        matches!(
            d,
            Declaration::Series { .. }
                | Declaration::VarDecl(_)
                | Declaration::OnDirective(_)
        )
    }) {
        return true;
    }
    node.children.iter().any(has_program_declarations)
}

fn program_walk(
    node: &mut Node,
    state: &mut WalkState,
    var_meta: &mut HashMap<String, VarMeta>,
    triggers: &mut Vec<OnDirective>,
    label_map: &mut HashMap<String, LabelEntry>,
    ancestors: &[NodeSnapshot],
) {
    // 1. Initialize variables from declarations on this node.
    let decls = node.attrs.declarations.clone();
    for decl in &decls {
        match decl {
            Declaration::VarDecl(vd) => {
                state.entry(vd.name.clone()).or_insert_with(|| vd.value.clone());
                var_meta.entry(vd.name.clone()).or_insert_with(|| VarMeta {
                    initial: vd.value.clone(),
                    formats: vd.formats.iter().map(|f| FormatToken::parse(f)).collect(),
                    cycle: vd.cycle,
                });
            }
            Declaration::Series { id, config } => {
                // Series compat: initialize as Number(0) or List([]) based on format count
                let initial = if config.formats.len() > 1 {
                    Value::List(vec![])
                } else {
                    Value::Number(0.0)
                };
                state.entry(id.clone()).or_insert_with(|| initial.clone());
                var_meta.entry(id.clone()).or_insert_with(|| VarMeta {
                    initial,
                    formats: config
                        .formats
                        .iter()
                        .map(|f| FormatToken::parse(f))
                        .collect(),
                    cycle: config.cycle,
                });
            }
            Declaration::OnDirective(on) => {
                triggers.push(on.clone());
            }
        }
    }

    // 2. Fire matching triggers in declaration order.
    let snap = NodeSnapshot::from_node(node);
    let trigger_snapshot = triggers.clone();
    for trigger in &trigger_snapshot {
        if matches_selector(&trigger.selector, node, ancestors) {
            for action in &trigger.actions {
                execute_action(action, node, state, var_meta, label_map);
            }
        }
    }

    // 3. Recurse — children see (and modify) the running state.
    let mut child_ancestors: Vec<NodeSnapshot> = ancestors.to_vec();
    child_ancestors.push(snap);
    for child in &mut node.children {
        program_walk(child, state, var_meta, triggers, label_map, &child_ancestors);
    }
}

/// Execute a single action on a node.
fn execute_action(
    action: &Action,
    node: &mut Node,
    state: &mut WalkState,
    var_meta: &HashMap<String, VarMeta>,
    label_map: &mut HashMap<String, LabelEntry>,
) {
    match action {
        Action::Increment { var } => {
            if let Some(val) = state.get_mut(var) {
                match val {
                    Value::Number(n) => *n += 1.0,
                    Value::List(counts) => {
                        // Hierarchical: bump at heading depth, truncate deeper.
                        let level = match &node.kind {
                            NodeKind::Heading { level } => *level as usize,
                            _ => 1,
                        };
                        while counts.len() < level {
                            counts.push(Value::Number(0.0));
                        }
                        if let Some(Value::Number(n)) = counts.get_mut(level - 1) {
                            *n += 1.0;
                        }
                        counts.truncate(level);
                    }
                    _ => {}
                }
            }
        }
        Action::Reset { var, to } => {
            // Use the provided reset value, or fall back to var's initial value.
            let reset_val = if matches!(to, Value::Number(n) if *n == 0.0) {
                var_meta
                    .get(var)
                    .map(|m| m.initial.clone())
                    .unwrap_or_else(|| to.clone())
            } else {
                to.clone()
            };
            state.insert(var.clone(), reset_val);
        }
        Action::StoreFormatted {
            var,
            property,
            formats,
            cycle,
        } => {
            if let Some(val) = state.get(var) {
                // Use action-level formats if provided, otherwise fall back to var_meta
                let (effective_formats, effective_cycle) = if !formats.is_empty() {
                    (formats.as_slice(), *cycle)
                } else if let Some(meta) = var_meta.get(var) {
                    (meta.formats.as_slice(), meta.cycle)
                } else {
                    (&[] as &[FormatToken], false)
                };
                let number = format_value(val, effective_formats, effective_cycle);
                node.attrs
                    .properties
                    .insert(property.clone(), Value::Text(number.clone()));

                // Build label map entry for cross-refs.
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
        Action::SetProperty { property, value } => {
            node.attrs
                .properties
                .insert(property.clone(), value.clone());
        }
        Action::Prepend { template } => {
            let text = interpolate_template(template, &node.attrs.properties);
            node.children.insert(0, Node::text(text));
        }
        Action::Append { template } => {
            let text = interpolate_template(template, &node.attrs.properties);
            node.children.push(Node::text(text));
        }
    }
}

/// Format a Value using format tokens.
fn format_value(val: &Value, formats: &[FormatToken], cycle: bool) -> String {
    match val {
        Value::Number(n) => {
            let fmt = if formats.is_empty() {
                FormatToken::parse("1")
            } else {
                formats[0].clone()
            };
            fmt.render(*n as u32)
        }
        Value::List(counts) => {
            // Skip leading zeros — handles documents where top-level headings
            // (e.g. h1) are absent from content (provided by template instead).
            let start = counts
                .iter()
                .position(|v| !matches!(v, Value::Number(n) if *n == 0.0))
                .unwrap_or(0);
            counts[start..]
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let n = match v {
                        Value::Number(n) => *n as u32,
                        _ => 0,
                    };
                    let fmt = get_format_at(formats, start + i, cycle);
                    fmt.render(n)
                })
                .collect::<Vec<_>>()
                .join(".")
        }
        _ => String::new(),
    }
}

/// Get the format token for a given depth index.
fn get_format_at(formats: &[FormatToken], depth: usize, cycle: bool) -> FormatToken {
    if formats.is_empty() {
        return FormatToken::parse("1");
    }
    let idx = if cycle {
        depth % formats.len()
    } else {
        depth.min(formats.len() - 1)
    };
    formats[idx].clone()
}

/// Interpolate `{key}` placeholders in a template from node properties.
fn interpolate_template(template: &str, properties: &HashMap<String, Value>) -> String {
    let mut result = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut key = String::new();
            while let Some(&c) = chars.peek() {
                if c == '}' {
                    chars.next();
                    break;
                }
                key.push(c);
                chars.next();
            }
            if let Some(val) = properties.get(&key) {
                match val {
                    Value::Text(s) => result.push_str(s),
                    Value::Number(n) => result.push_str(&n.to_string()),
                    Value::Bool(b) => result.push_str(&b.to_string()),
                    _ => {}
                }
            }
            // Missing key → empty string (by design)
        } else {
            result.push(ch);
        }
    }
    result
}

// ── Legacy compat aliases ──

/// Legacy alias for `ExecuteProgram`.
pub type AssignNumbers = ExecuteProgram;

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
/// Must run after `CascadeInits` (reads the `preview` property).
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
        assert!(!doc.content.attrs.inits.is_empty());
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
        doc.content.attrs.inits.push(Init {
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
    fn format_value_flat() {
        let val = Value::Number(5.0);
        let fmts = vec![FormatToken::parse("a")];
        assert_eq!(format_value(&val, &fmts, false), "e");
    }

    #[test]
    fn format_value_hierarchical() {
        let val = Value::List(vec![Value::Number(2.0), Value::Number(3.0)]);
        let fmts = vec![FormatToken::parse("I"), FormatToken::parse("1")];
        assert_eq!(format_value(&val, &fmts, false), "II.3");
    }

    #[test]
    fn get_format_at_cycling() {
        let fmts = vec![FormatToken::parse("I"), FormatToken::parse("A")];
        // depth 0 → "I", depth 1 → "A", depth 2 → "I" (wraps)
        assert!(matches!(
            get_format_at(&fmts, 0, true).format,
            NumberFormat::RomanUpper
        ));
        assert!(matches!(
            get_format_at(&fmts, 1, true).format,
            NumberFormat::AlphaUpper
        ));
        assert!(matches!(
            get_format_at(&fmts, 2, true).format,
            NumberFormat::RomanUpper
        ));
    }

    #[test]
    fn get_format_at_repeat_last() {
        let fmts = vec![FormatToken::parse("I"), FormatToken::parse("1")];
        // depth 2 and beyond → "1" (repeat last)
        assert!(matches!(
            get_format_at(&fmts, 2, false).format,
            NumberFormat::Arabic
        ));
        assert!(matches!(
            get_format_at(&fmts, 10, false).format,
            NumberFormat::Arabic
        ));
    }

    #[test]
    fn get_format_at_empty_defaults() {
        let fmts: Vec<FormatToken> = Vec::new();
        assert!(matches!(
            get_format_at(&fmts, 0, false).format,
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
        let has_hints = !doc.content.attrs.inits.is_empty()
            || doc.content.children.iter().any(|c| !c.attrs.inits.is_empty());
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
        CascadeInits.run(&mut doc);
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
        CascadeInits.run(&mut doc);
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
        CascadeInits.run(&mut doc);
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
        CascadeInits.run(&mut doc);
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

    // ── interpolate_template ──

    #[test]
    fn interpolate_template_basic() {
        let mut props = HashMap::new();
        props.insert("number".to_string(), Value::Text("I.1".to_string()));
        assert_eq!(interpolate_template("{number}. ", &props), "I.1. ");
    }

    #[test]
    fn interpolate_template_missing_key() {
        let props = HashMap::new();
        assert_eq!(interpolate_template("{missing} text", &props), " text");
    }

    // ── get_compile_format ──

    #[test]
    fn get_compile_format_custom() {
        let mut attrs = crate::ast::Attrs::default();
        attrs.properties.insert("preview".to_string(), Value::Text("latex".to_string()));
        assert_eq!(get_compile_format(&attrs, "typst"), "latex");
    }

    // ── Generic engine: new primitive tests ──

    #[test]
    fn var_and_on_flat_count() {
        reset_id_counter();
        let input = "\
{@var(fig, 0, format=\"1\")}
{@init(figure, series=\"fig\")}
{@on([series=\"fig\"], @inc(fig), @store(number, fig))}

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
    fn var_and_on_hierarchical() {
        reset_id_counter();
        let input = "\
{@var(sec, [], format=[\"I\",\"1\"])}
{@init(heading, series=\"sec\")}
{@on([series=\"sec\"], @inc(sec), @store(number, sec))}

# Introduction

## Background

## Methods

# Results";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 4);
        assert_eq!(get_number(headings[0]), Some("I"));
        assert_eq!(get_number(headings[1]), Some("I.1"));
        assert_eq!(get_number(headings[2]), Some("I.2"));
        assert_eq!(get_number(headings[3]), Some("II"));
    }

    #[test]
    fn hierarchical_skips_leading_zeros_when_h1_absent() {
        reset_id_counter();
        let input = "\
{@var(sec, [], format=[\"I\",\"1\"])}
{@init(heading, series=\"sec\")}
{@on([series=\"sec\"], @inc(sec), @store(number, sec))}

## Background

## Methods

### Detail";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 3);
        // Leading zero at depth 0 (no h1 seen) is skipped
        assert_eq!(get_number(headings[0]), Some("1"));
        assert_eq!(get_number(headings[1]), Some("2"));
        assert_eq!(get_number(headings[2]), Some("2.1"));
    }

    #[test]
    fn on_reset() {
        reset_id_counter();
        let input = "\
{@series(sec, counts=heading, formats=[\"I\",\"1\"])}
{@var(fig, 0, format=\"1\")}
{@init(figure, series=\"fig\")}
{@on([series=\"fig\"], @inc(fig), @store(number, fig))}
{@on([series=\"sec\"], @reset(fig))}

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
        assert_eq!(get_number(figs[2]), Some("1"), "should reset after heading");
    }

    #[test]
    fn on_set_property() {
        reset_id_counter();
        let input = "\
{@on(figure, @set(highlighted, true))}

![Pic](a.png)";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let figs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Figure));
        assert_eq!(figs.len(), 1);
        assert!(
            figs[0].attrs.properties.contains_key("highlighted"),
            "figure should have highlighted property"
        );
    }

    #[test]
    fn on_prepend() {
        reset_id_counter();
        let input = "\
{@series(sec, counts=heading, formats=[\"I\",\"1\"])}
{@on(heading, @prepend(@text(\"{number}. \")))}

# Introduction";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 1);
        // First child should be the prepended text
        let first_child = &headings[0].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            assert_eq!(t, "I. ");
        } else {
            panic!("expected Text node, got {:?}", first_child.kind);
        }
    }

    #[test]
    fn on_append() {
        reset_id_counter();
        let input = "\
{@series(sec, counts=heading, formats=[\"I\",\"1\"])}
{@on(heading, @append(@text(\" §\")))}

# Introduction";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 1);
        // Last child should be the appended text
        let last_child = headings[0].children.last().unwrap();
        if let NodeKind::Text(ref t) = last_child.kind {
            assert_eq!(t, " §");
        } else {
            panic!("expected Text node, got {:?}", last_child.kind);
        }
    }

    #[test]
    fn content_sugar() {
        reset_id_counter();
        let input = "\
{@series(sec, counts=heading, formats=[\"I\",\"1\"])}
{@content(heading, \"{number}. \")}

# Introduction

## Background";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 2);
        // Check that number prefix was prepended
        let first_child = &headings[0].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            assert_eq!(t, "I. ");
        } else {
            panic!("expected Text node");
        }
        let first_child = &headings[1].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            assert_eq!(t, "I.1. ");
        } else {
            panic!("expected Text node");
        }
    }

    #[test]
    fn prepend_missing_property() {
        reset_id_counter();
        let input = "\
{@on(heading, @prepend(@text(\"{missing}prefix\")))}

# Title";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        let first_child = &headings[0].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            assert_eq!(t, "prefix", "missing property should interpolate as empty");
        } else {
            panic!("expected Text node");
        }
    }

    #[test]
    fn series_expands_to_primitives() {
        // @series(fig, counts=figure) should produce the same result
        // as manual @var + @init + @on
        reset_id_counter();
        let input_series = "\
{@series(fig, counts=figure)}

![A](a.png){#fig:a}

![B](b.png){#fig:b}";
        let mut doc_series = parse(input_series).unwrap();
        resolve_document(&mut doc_series);

        let figs_series = find_all(&doc_series.content, |n| matches!(n.kind, NodeKind::Figure));
        assert_eq!(get_number(figs_series[0]), Some("1"));
        assert_eq!(get_number(figs_series[1]), Some("2"));

        reset_id_counter();
        let input_manual = "\
{@var(fig, 0, format=\"1\")}
{@init(figure, series=\"fig\")}
{@on([series=\"fig\"], @inc(fig), @store(number, fig))}

![A](a.png){#fig:a}

![B](b.png){#fig:b}";
        let mut doc_manual = parse(input_manual).unwrap();
        resolve_document(&mut doc_manual);

        let figs_manual = find_all(&doc_manual.content, |n| matches!(n.kind, NodeKind::Figure));
        assert_eq!(get_number(figs_manual[0]), Some("1"));
        assert_eq!(get_number(figs_manual[1]), Some("2"));
    }

}
