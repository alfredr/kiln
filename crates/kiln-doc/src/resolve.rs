use std::collections::HashMap;

use crate::ast::{Document, Hint, Node, NodeKind, RawBlockCompiler, Selector, Value};
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
/// Runs `HoistDirectives` then `CascadeHints`.
pub fn resolve_document(doc: &mut Document) {
    HoistDirectives.run(doc);
    CascadeHints.run(doc);
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
    use crate::ast::{CompoundSelector, SelectorExpr, Value, reset_id_counter};
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
}
