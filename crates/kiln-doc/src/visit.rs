use crate::ast::{Document, Node};

// Re-export node_kind_name for external callers.
pub use crate::selector::node_kind_name;

// ── Walk helpers ──

/// Walk a node tree, calling `f` on each node before its children (pre-order).
pub fn walk(node: &Node, f: &mut impl FnMut(&Node)) {
    f(node);
    for child in &node.children {
        walk(child, f);
    }
}

/// Mutable pre-order walk.
pub fn walk_mut(node: &mut Node, f: &mut impl FnMut(&mut Node)) {
    f(node);
    for child in &mut node.children {
        walk_mut(child, f);
    }
}

/// Mutable post-order walk (children visited before parent).
pub fn walk_mut_post(node: &mut Node, f: &mut impl FnMut(&mut Node)) {
    for child in &mut node.children {
        walk_mut_post(child, f);
    }
    f(node);
}

// ── Pass trait ──

/// A processing pass over a document.
///
/// Passes can use the walk helpers above for simple per-node logic,
/// or implement their own traversal for complex patterns (e.g. accumulator state).
pub trait Pass {
    fn run(&mut self, doc: &mut Document);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Attrs, Node, NodeKind, reset_id_counter};

    fn text(s: &str) -> Node {
        Node {
            kind: NodeKind::Text(s.to_string()),
            attrs: Attrs::default(),
            children: vec![],
        }
    }

    fn para(children: Vec<Node>) -> Node {
        Node {
            kind: NodeKind::Paragraph,
            attrs: Attrs::default(),
            children,
        }
    }

    #[test]
    fn walk_visits_all_nodes() {
        reset_id_counter();
        let tree = para(vec![text("a"), text("b")]);
        let mut count = 0;
        walk(&tree, &mut |_| count += 1);
        assert_eq!(count, 3); // para + 2 text
    }

    #[test]
    fn walk_mut_pre_order() {
        reset_id_counter();
        let mut tree = para(vec![text("a"), text("b")]);
        let mut order = Vec::new();
        walk_mut(&mut tree, &mut |node| {
            order.push(node_kind_name(&node.kind).to_string());
        });
        assert_eq!(order, vec!["Paragraph", "Text", "Text"]);
    }

    #[test]
    fn walk_mut_post_order() {
        reset_id_counter();
        let mut tree = para(vec![text("a"), text("b")]);
        let mut order = Vec::new();
        walk_mut_post(&mut tree, &mut |node| {
            order.push(node_kind_name(&node.kind).to_string());
        });
        assert_eq!(order, vec!["Text", "Text", "Paragraph"]);
    }
}
