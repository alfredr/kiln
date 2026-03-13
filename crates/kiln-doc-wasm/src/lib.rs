use std::cell::RefCell;
use wasm_bindgen::prelude::*;

use kiln_doc::ast::{Document, Event, Node, Selector};
use kiln_doc::parse;
use kiln_doc::resolve::{
    self, CascadeInits, HoistDirectives, ResolveCrossRefs, Runtime, Trace, TraceEvent,
};
use kiln_doc::selector::{self, NodeSnapshot};
use kiln_doc::visit::Pass;

thread_local! {
    static LAST_TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
    static LIVE: RefCell<Option<(Document, Runtime)>> = const { RefCell::new(None) };
}

/// Parse kiln markup and return a full execution trace as JSON.
///
/// Returns `{ "input": "...", "initial_ast": {...}, "events": [...] }`
#[wasm_bindgen]
pub fn trace(input: &str) -> String {
    let doc = match parse::parse(input) {
        Ok(doc) => doc,
        Err(e) => return serde_json::json!({ "error": format!("{e}") }).to_string(),
    };
    let t = resolve::resolve_document_traced(&doc, input);
    let json = serde_json::to_string(&t).unwrap_or_else(|e| {
        serde_json::json!({ "error": format!("{e}") }).to_string()
    });
    LAST_TRACE.with(|cell| *cell.borrow_mut() = Some(t));
    json
}

/// Given the current event index and a selector string, return JSON array of
/// matching node IDs in the AST at that point.
#[wasm_bindgen]
pub fn match_selector(event_idx: usize, selector_str: &str) -> String {
    LAST_TRACE.with(|cell| {
        let borrow = cell.borrow();
        let trace = match borrow.as_ref() {
            Some(t) => t,
            None => return "[]".to_string(),
        };
        let sel = match selector::parse_selector(selector_str) {
            Some(s) => s,
            None => return "[]".to_string(),
        };
        let ast = find_ast_at_event(trace, event_idx);
        let mut matches = Vec::new();
        collect_matches(ast, &sel, &[], &mut matches);
        serde_json::to_string(&matches).unwrap_or_else(|_| "[]".to_string())
    })
}

fn find_ast_at_event(trace: &Trace, idx: usize) -> &Node {
    let mut last_ast = &trace.initial_ast.content;
    for ev in trace.events.iter().take(idx + 1) {
        if let TraceEvent::PassEnd { ast, .. } = ev {
            last_ast = &ast.content;
        }
    }
    last_ast
}

fn collect_matches(
    node: &Node,
    sel: &Selector,
    ancestors: &[NodeSnapshot],
    out: &mut Vec<String>,
) {
    if selector::matches_selector(sel, node, ancestors) {
        out.push(node.attrs.id.clone());
    }
    let snap = NodeSnapshot::from_node(node);
    let mut child_ancestors = ancestors.to_vec();
    child_ancestors.push(snap);
    for child in &node.children {
        collect_matches(child, sel, &child_ancestors, out);
    }
}

// ── Live interactive runtime ──

/// Initialize a live runtime from kiln markup.
///
/// Runs parse → hoist → cascade → execute(init) → crossrefs, then stores
/// the document and runtime for subsequent `dispatch_event()` calls.
/// Returns the resolved AST as JSON.
#[wasm_bindgen]
pub fn init_runtime(input: &str) -> String {
    let mut doc = match parse::parse(input) {
        Ok(doc) => doc,
        Err(e) => return serde_json::json!({ "error": format!("{e}") }).to_string(),
    };
    HoistDirectives.run(&mut doc);
    CascadeInits.run(&mut doc);
    let rt = Runtime::init(&mut doc);
    ResolveCrossRefs.run(&mut doc);

    let json = serde_json::to_string(&doc).unwrap_or_else(|e| {
        serde_json::json!({ "error": format!("{e}") }).to_string()
    });
    LIVE.with(|cell| *cell.borrow_mut() = Some((doc, rt)));
    json
}

/// Dispatch a user event to a node in the live runtime.
///
/// Returns JSON: `{ "changed": ["node-id", ...], "ast": {...} }`
#[wasm_bindgen]
pub fn dispatch_event(event: &str, node_id: &str) -> String {
    LIVE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some((doc, rt)) = borrow.as_mut() else {
            return serde_json::json!({ "error": "no live runtime" }).to_string();
        };
        let event = match event {
            "click" => Event::Click,
            "enter" => Event::Enter,
            "exit" => Event::Exit,
            _ => return serde_json::json!({ "error": format!("unknown event: {event}") }).to_string(),
        };
        let changed = rt.dispatch(doc, event, node_id);
        serde_json::json!({
            "changed": changed,
            "ast": doc,
        })
        .to_string()
    })
}
