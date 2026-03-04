use std::collections::HashMap;

use crate::ast::Document;

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
