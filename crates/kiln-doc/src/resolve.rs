use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::ast::{
    Action, Bind, Declaration, Document, Event, FormatToken, Init, Node, NodeKind, OnDirective,
    Placement, PushValue, RawBlockCompiler, Selector, Value,
};
use crate::selector::{NodeSnapshot, matches_selector, node_kind_name};
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
/// Runs `HoistDirectives` → `CascadeInits` → `ExecuteProgram` → `ResolveCrossRefs`
/// → `NumberFootnotes`.
pub fn resolve_document(doc: &mut Document) {
    HoistDirectives.run(doc);
    CascadeInits.run(doc);
    ExecuteProgram.run(doc);
    ResolveCrossRefs.run(doc);
    NumberFootnotes.run(doc);
}

/// Resolve a document step-by-step, returning the AST snapshot after each pass.
///
/// Returns `[("Parsed", ...), ("Hoist", ...), ("Cascade", ...), ("Execute", ...), ("CrossRefs", ...)]`.
pub fn resolve_document_stepped(doc: &Document) -> Vec<(&'static str, Document)> {
    let mut doc = doc.clone();
    let mut stages = vec![("Parsed", doc.clone())];

    HoistDirectives.run(&mut doc);
    stages.push(("Hoist", doc.clone()));

    CascadeInits.run(&mut doc);
    stages.push(("Cascade", doc.clone()));

    ExecuteProgram.run(&mut doc);
    stages.push(("Execute", doc.clone()));

    ResolveCrossRefs.run(&mut doc);
    stages.push(("CrossRefs", doc.clone()));

    stages
}

// ── Trace types ──

/// A full execution trace: initial AST + all events from all passes.
#[derive(Serialize)]
pub struct Trace {
    pub input: String,
    pub initial_ast: Document,
    pub events: Vec<TraceEvent>,
}

/// A single event in the execution trace.
#[derive(Serialize)]
pub enum TraceEvent {
    PassStart { name: String },
    PassEnd { name: String, ast: Box<Document> },

    // Generic node enter/exit for all passes
    NodeEnter { pass: String, node_id: String, kind: String, node_properties: HashMap<String, Value> },
    NodeExit { pass: String, node_id: String, node_properties: HashMap<String, Value> },

    // HoistDirectives
    Hoist { node_id: String, decls: usize, inits: usize, node_properties: HashMap<String, Value> },

    // CascadeInits
    InitApplied { node_id: String, property: String, value: Value, source: String },
    InitSkipped { node_id: String, property: String },

    // ExecuteProgram
    DeclareVar { name: String, value: Value },
    RegisterTrigger { selector: String, event: String, actions: Vec<String> },
    DefRegister { name: String, params: Vec<String> },
    UseExpand { name: String, bindings: HashMap<String, String> },
    TriggerMatch { node_id: String, selector: String, event: String },
    ActionExec {
        node_id: String,
        action: String,
        state_before: HashMap<String, Value>,
        state_after: HashMap<String, Value>,
        node_properties: HashMap<String, Value>,
    },
    PrependText { node_id: String, text: String },
    AppendText { node_id: String, text: String },

    // ResolveCrossRefs
    ResolveRef { node_id: String, target: String, resolved_text: String, node_properties: HashMap<String, Value> },
}

/// Resolve a document with full tracing — returns the initial AST and all events.
pub fn resolve_document_traced(doc: &Document, input: &str) -> Trace {
    let initial_ast = doc.clone();
    let mut doc = doc.clone();
    let mut events = Vec::new();

    events.push(TraceEvent::PassStart { name: "Hoist".into() });
    hoist_traced(&mut doc.content, &mut events);
    events.push(TraceEvent::PassEnd { name: "Hoist".into(), ast: Box::new(doc.clone()) });

    events.push(TraceEvent::PassStart { name: "Cascade".into() });
    cascade_traced(&mut doc.content, &[], &[], &mut events);
    events.push(TraceEvent::PassEnd { name: "Cascade".into(), ast: Box::new(doc.clone()) });

    events.push(TraceEvent::PassStart { name: "Execute".into() });
    execute_program_traced(&mut doc, &mut events);
    events.push(TraceEvent::PassEnd { name: "Execute".into(), ast: Box::new(doc.clone()) });

    events.push(TraceEvent::PassStart { name: "CrossRefs".into() });
    resolve_refs_traced(&mut doc, &mut events);
    events.push(TraceEvent::PassEnd { name: "CrossRefs".into(), ast: Box::new(doc.clone()) });

    Trace {
        input: input.to_string(),
        initial_ast,
        events,
    }
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

fn hoist_traced(node: &mut Node, events: &mut Vec<TraceEvent>) {
    events.push(TraceEvent::NodeEnter {
        pass: "Hoist".into(),
        node_id: node.attrs.id.clone(),
        kind: node_kind_name(&node.kind).to_string(),
        node_properties: node.attrs.properties.clone(),
    });

    for child in &mut node.children {
        hoist_traced(child, events);
    }

    let mut hoisted_inits = Vec::new();
    let mut hoisted_decls = Vec::new();

    for child in &node.children {
        if matches!(&child.kind, NodeKind::Custom { content_type } if content_type == "directive") {
            hoisted_inits.extend(child.attrs.inits.iter().cloned());
            hoisted_decls.extend(child.attrs.declarations.iter().cloned());
        }
    }

    node.attrs.inits.extend(hoisted_inits.clone());
    node.attrs.declarations.extend(hoisted_decls.clone());
    node.children.retain(|child| {
        !matches!(&child.kind, NodeKind::Custom { content_type } if content_type == "directive")
    });

    if !hoisted_decls.is_empty() || !hoisted_inits.is_empty() {
        events.push(TraceEvent::Hoist {
            node_id: node.attrs.id.clone(),
            decls: hoisted_decls.len(),
            inits: hoisted_inits.len(),
            node_properties: node.attrs.properties.clone(),
        });
    }

    events.push(TraceEvent::NodeExit {
        pass: "Hoist".into(),
        node_id: node.attrs.id.clone(),
        node_properties: node.attrs.properties.clone(),
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
                .or_insert_with(|| init.value.val().clone());
        }
    }

    // Match ancestor inits (nearest first via rev → entry-or-insert = nearest wins).
    for init in ancestor_inits.iter().rev().flatten() {
        if matches_selector(&init.selector, node, ancestors) {
            node.attrs
                .properties
                .entry(init.property.clone())
                .or_insert_with(|| init.value.val().clone());
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

fn apply_init_traced(node: &mut Node, init: &Init, source: &str, events: &mut Vec<TraceEvent>) {
    let prop = init.property.clone();
    if node.attrs.properties.contains_key(&prop) {
        events.push(TraceEvent::InitSkipped {
            node_id: node.attrs.id.clone(),
            property: prop,
        });
    } else {
        let val = init.value.val().clone();
        events.push(TraceEvent::InitApplied {
            node_id: node.attrs.id.clone(),
            property: prop.clone(),
            value: val.clone(),
            source: source.into(),
        });
        node.attrs.properties.insert(prop, val);
    }
}

fn cascade_traced(
    node: &mut Node,
    ancestor_inits: &[Vec<Init>],
    ancestors: &[NodeSnapshot],
    events: &mut Vec<TraceEvent>,
) {
    let kind = node_kind_name(&node.kind).to_string();
    events.push(TraceEvent::NodeEnter {
        pass: "Cascade".into(),
        node_id: node.attrs.id.clone(),
        kind,
        node_properties: node.attrs.properties.clone(),
    });

    // Apply own This inits to self (clone to release borrow on node).
    let this_inits: Vec<_> = node.attrs.inits.iter()
        .filter(|i| matches!(i.selector, Selector::This))
        .cloned()
        .collect();
    for init in &this_inits {
        apply_init_traced(node, init, "self", events);
    }

    // Match ancestor inits (nearest first via rev).
    for init in ancestor_inits.iter().rev().flatten() {
        if matches_selector(&init.selector, node, ancestors) {
            apply_init_traced(node, init, "ancestor", events);
        }
    }

    let own_inits = node.attrs.inits.clone();
    let mut child_ancestor_inits: Vec<Vec<Init>> = ancestor_inits.to_vec();
    child_ancestor_inits.push(own_inits);

    let snap = NodeSnapshot::from_node(node);
    let mut child_ancestors: Vec<NodeSnapshot> = ancestors.to_vec();
    child_ancestors.push(snap);

    for child in &mut node.children {
        cascade_traced(child, &child_ancestor_inits, &child_ancestors, events);
    }

    events.push(TraceEvent::NodeExit {
        pass: "Cascade".into(),
        node_id: node.attrs.id.clone(),
        node_properties: node.attrs.properties.clone(),
    });
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

/// Named definition registry carried through the walk.
type Defs = HashMap<String, (Vec<(String, String)>, Vec<Declaration>, Vec<Init>)>;

/// Persistent runtime state for the reactive walk engine.
///
/// In static mode, created during [`ExecuteProgram`] and consumed immediately.
/// In interactive mode (browser/WASM), kept alive to dispatch user events.
pub struct Runtime {
    state: WalkState,
    var_meta: HashMap<String, VarMeta>,
    triggers: Vec<OnDirective>,
    label_map: HashMap<String, LabelEntry>,
    defs: Defs,
}

impl Runtime {
    /// Create an empty runtime.
    fn new() -> Self {
        Self {
            state: WalkState::new(),
            var_meta: HashMap::new(),
            triggers: Vec::new(),
            label_map: HashMap::new(),
            defs: HashMap::new(),
        }
    }

    /// Phase 1: Walk the tree in document order, registering variables and
    /// triggers, and firing Enter/Exit events. Returns `Self` with accumulated
    /// state for further dispatching or finalization.
    pub fn init(doc: &mut Document) -> Self {
        if !has_program_declarations(&doc.content) {
            return Self::new();
        }
        let mut rt = Self::new();
        program_walk(&mut doc.content, &mut rt, &[]);
        rt
    }

    /// Finalize: encode the label map into the document root and consume self.
    /// Used by static precompilation where no further dispatching is needed.
    pub fn finish(self, doc: &mut Document) {
        if !self.label_map.is_empty() {
            doc.content
                .attrs
                .properties
                .insert("__label_map".to_string(), encode_label_map(&self.label_map));
        }
    }

    /// Static mode convenience: init + finish in one step.
    pub fn run_static(doc: &mut Document) {
        let rt = Self::init(doc);
        rt.finish(doc);
    }

    /// Dispatch a user event to a specific node, executing matching triggers.
    ///
    /// Returns the IDs of nodes whose triggers were fired (and thus may have
    /// changed properties or state).
    pub fn dispatch(
        &mut self,
        doc: &mut Document,
        event: Event,
        node_id: &str,
    ) -> Vec<String> {
        let ancestors = match build_ancestor_chain(&doc.content, node_id) {
            Some(a) => a,
            None => return vec![],
        };
        let Some(node) = find_node_mut(&mut doc.content, node_id) else {
            return vec![];
        };
        let triggers = self.triggers.clone();
        let fired = dispatch_triggers(event, &triggers, node, &ancestors, self);
        // Re-evaluate content templates (Prepend/Append) so visible text updates
        reapply_content(&triggers, node, &ancestors);
        if fired { vec![node_id.to_string()] } else { vec![] }
    }
}

/// Find a mutable reference to a node by ID.
fn find_node_mut<'a>(node: &'a mut Node, id: &str) -> Option<&'a mut Node> {
    if node.attrs.id == id {
        return Some(node);
    }
    for child in &mut node.children {
        if let Some(found) = find_node_mut(child, id) {
            return Some(found);
        }
    }
    None
}

/// Build the ancestor snapshot chain for a node (root-first), excluding the node itself.
fn build_ancestor_chain(root: &Node, target_id: &str) -> Option<Vec<NodeSnapshot>> {
    let mut path = Vec::new();
    if ancestor_walk(root, target_id, &mut path) {
        Some(path)
    } else {
        None
    }
}

fn ancestor_walk(node: &Node, target_id: &str, path: &mut Vec<NodeSnapshot>) -> bool {
    if node.attrs.id == target_id {
        return true;
    }
    path.push(NodeSnapshot::from_node(node));
    for child in &node.children {
        if ancestor_walk(child, target_id, path) {
            return true;
        }
    }
    path.pop();
    false
}

/// Walk the tree in document order, executing variable inits and firing
/// selector-triggered actions. Replaces the old `AssignNumbers` pass.
pub struct ExecuteProgram;

impl Pass for ExecuteProgram {
    fn run(&mut self, doc: &mut Document) {
        Runtime::run_static(doc);
    }
}

/// Check if any program declarations exist in the tree.
fn has_program_declarations(node: &Node) -> bool {
    if node.attrs.declarations.iter().any(|d| {
        matches!(
            d,
            Declaration::VarDecl(_)
                | Declaration::OnDirective(_)
                | Declaration::Def { .. }
                | Declaration::Use { .. }
        )
    }) {
        return true;
    }
    node.children.iter().any(has_program_declarations)
}

fn program_walk(node: &mut Node, ps: &mut Runtime, ancestors: &[NodeSnapshot]) {
    // 1. Process declarations on this node.
    let decls = node.attrs.declarations.clone();
    process_declarations(&decls, ps, &mut node.attrs.inits);

    // 2. Fire matching Enter triggers in declaration order.
    let snap = NodeSnapshot::from_node(node);
    let trigger_snapshot = ps.triggers.clone();
    fire_triggers(Event::Enter, &trigger_snapshot, node, ancestors, ps);

    // 3. Recurse — children see (and modify) the running state.
    let mut child_ancestors: Vec<NodeSnapshot> = ancestors.to_vec();
    child_ancestors.push(snap);
    for child in &mut node.children {
        program_walk(child, ps, &child_ancestors);
    }

    // 4. Fire matching Exit triggers after children.
    let trigger_snapshot = ps.triggers.clone();
    fire_triggers(Event::Exit, &trigger_snapshot, node, ancestors, ps);
}

fn fire_triggers(
    event: Event,
    triggers: &[OnDirective],
    node: &mut Node,
    ancestors: &[NodeSnapshot],
    rt: &mut Runtime,
) {
    dispatch_triggers(event, triggers, node, ancestors, rt);
}

/// Fire matching triggers and return whether any fired.
fn dispatch_triggers(
    event: Event,
    triggers: &[OnDirective],
    node: &mut Node,
    ancestors: &[NodeSnapshot],
    rt: &mut Runtime,
) -> bool {
    let mut fired = false;
    for trigger in triggers {
        if trigger.event != event || !matches_selector(&trigger.selector, node, ancestors) {
            continue;
        }
        fired = true;
        for action in &trigger.actions {
            execute_action(action, node, &mut rt.state, &rt.var_meta, &mut rt.label_map);
        }
    }
    fired
}

/// Re-evaluate only Prepend/Append actions from matching triggers.
///
/// Called after dispatch to update content text nodes without re-firing
/// side-effecting actions like @inc or @store.
fn reapply_content(triggers: &[OnDirective], node: &mut Node, ancestors: &[NodeSnapshot]) {
    for trigger in triggers {
        if !matches_selector(&trigger.selector, node, ancestors) {
            continue;
        }
        for action in &trigger.actions {
            match action {
                Action::Prepend { template } => {
                    let text = interpolate_template(template.val(), &node.attrs.properties);
                    if let Some(first) = node.children.first_mut() {
                        if first.attrs.properties.contains_key("__content") {
                            first.kind = NodeKind::Text(text);
                            continue;
                        }
                    }
                    let mut tn = Node::text(text);
                    tn.attrs.properties
                        .insert("__content".to_string(), Value::Bool(true));
                    node.children.insert(0, tn);
                }
                Action::Append { template } => {
                    let text = interpolate_template(template.val(), &node.attrs.properties);
                    if let Some(last) = node.children.last_mut() {
                        if last.attrs.properties.contains_key("__content_append") {
                            last.kind = NodeKind::Text(text);
                            continue;
                        }
                    }
                    let mut tn = Node::text(text);
                    tn.attrs.properties
                        .insert("__content_append".to_string(), Value::Bool(true));
                    node.children.push(tn);
                }
                _ => {}
            }
        }
    }
}

/// Process a list of declarations: register vars, triggers, defs, and expand uses.
fn process_declarations(decls: &[Declaration], ps: &mut Runtime, inits: &mut Vec<Init>) {
    for decl in decls {
        process_one_declaration(decl, ps, inits);
    }
}

fn process_one_declaration(decl: &Declaration, ps: &mut Runtime, inits: &mut Vec<Init>) {
    match decl {
        Declaration::VarDecl(vd) => {
            ps.state.entry(vd.name.clone()).or_insert_with(|| vd.value.val().clone());
            ps.var_meta.entry(vd.name.clone()).or_insert_with(|| VarMeta {
                initial: vd.value.val().clone(),
                formats: vd.formats.val().iter().map(|f| FormatToken::parse(f)).collect(),
                cycle: vd.cycle,
            });
        }
        Declaration::OnDirective(on) => {
            ps.triggers.push(on.clone());
        }
        Declaration::Def { name, params, body, inits: def_inits } => {
            ps.defs.insert(name.clone(), (params.clone(), body.clone(), def_inits.clone()));
        }
        Declaration::Use { name, overrides } => {
            let Some((params, body, def_inits)) = ps.defs.get(name).cloned() else {
                return;
            };
            if overrides.is_empty() && params.is_empty() {
                inits.extend(def_inits);
                process_declarations(&body, ps, inits);
            } else {
                let bindings = build_bindings(&params, overrides);
                let expanded = resolve_decls(&body, &bindings);
                let expanded_inits = resolve_inits(&def_inits, &bindings);
                inits.extend(expanded_inits);
                process_declarations(&expanded, ps, inits);
            }
        }
    }
}

// ── Traced ExecuteProgram ──

fn execute_program_traced(doc: &mut Document, events: &mut Vec<TraceEvent>) {
    if !has_program_declarations(&doc.content) {
        return;
    }
    let mut rt = Runtime::new();
    program_walk_traced(&mut doc.content, &mut rt, &[], events);
    rt.finish(doc);
}

fn program_walk_traced(
    node: &mut Node,
    ps: &mut Runtime,
    ancestors: &[NodeSnapshot],
    events: &mut Vec<TraceEvent>,
) {
    let kind = node_kind_name(&node.kind).to_string();
    events.push(TraceEvent::NodeEnter {
        pass: "Execute".into(),
        node_id: node.attrs.id.clone(),
        kind,
        node_properties: node.attrs.properties.clone(),
    });

    // 1. Process declarations with tracing.
    let decls = node.attrs.declarations.clone();
    process_declarations_traced(&decls, ps, &mut node.attrs.inits, events);

    // 2. Fire matching Enter triggers.
    let snap = NodeSnapshot::from_node(node);
    let trigger_snapshot = ps.triggers.clone();
    fire_triggers_traced(Event::Enter, &trigger_snapshot, node, ancestors, ps, events);

    // 3. Recurse.
    let mut child_ancestors: Vec<NodeSnapshot> = ancestors.to_vec();
    child_ancestors.push(snap);
    for child in &mut node.children {
        program_walk_traced(child, ps, &child_ancestors, events);
    }

    // 4. Fire matching Exit triggers.
    let trigger_snapshot = ps.triggers.clone();
    fire_triggers_traced(Event::Exit, &trigger_snapshot, node, ancestors, ps, events);

    events.push(TraceEvent::NodeExit {
        pass: "Execute".into(),
        node_id: node.attrs.id.clone(),
        node_properties: node.attrs.properties.clone(),
    });
}

fn trace_action(
    action: &Action,
    node: &Node,
    state_before: HashMap<String, Value>,
    state_after: HashMap<String, Value>,
    events: &mut Vec<TraceEvent>,
) {
    let node_id = node.attrs.id.clone();
    match action {
        Action::Prepend { template } => {
            let text = interpolate_template(template.val(), &node.attrs.properties);
            events.push(TraceEvent::PrependText { node_id, text });
        }
        Action::Append { template } => {
            let text = interpolate_template(template.val(), &node.attrs.properties);
            events.push(TraceEvent::AppendText { node_id, text });
        }
        _ => {
            events.push(TraceEvent::ActionExec {
                node_id,
                action: format!("{action}"),
                state_before,
                state_after,
                node_properties: node.attrs.properties.clone(),
            });
        }
    }
}

fn fire_triggers_traced(
    event: Event,
    triggers: &[OnDirective],
    node: &mut Node,
    ancestors: &[NodeSnapshot],
    ps: &mut Runtime,
    events: &mut Vec<TraceEvent>,
) {
    for trigger in triggers {
        if trigger.event != event || !matches_selector(&trigger.selector, node, ancestors) {
            continue;
        }
        let event_str = match event { Event::Enter => "enter", Event::Exit => "exit", Event::Click => "click" };
        events.push(TraceEvent::TriggerMatch {
            node_id: node.attrs.id.clone(),
            selector: format!("{}", trigger.selector),
            event: event_str.into(),
        });
        for action in &trigger.actions {
            let state_before = ps.state.clone();
            execute_action(action, node, &mut ps.state, &ps.var_meta, &mut ps.label_map);
            let state_after = ps.state.clone();
            trace_action(action, node, state_before, state_after, events);
        }
    }
}

fn trace_declaration(decl: &Declaration, defs: &Defs, events: &mut Vec<TraceEvent>) {
    match decl {
        Declaration::VarDecl(vd) => {
            if let Bind::Val(v) = &vd.value {
                events.push(TraceEvent::DeclareVar {
                    name: vd.name.clone(),
                    value: v.clone(),
                });
            }
        }
        Declaration::OnDirective(on) => {
            let event_str = match on.event {
                Event::Enter => "enter",
                Event::Exit => "exit",
                Event::Click => "click",
            };
            events.push(TraceEvent::RegisterTrigger {
                selector: format!("{}", on.selector),
                event: event_str.into(),
                actions: on.actions.iter().map(|a| format!("{a}")).collect(),
            });
        }
        Declaration::Def { name, params, .. } => {
            events.push(TraceEvent::DefRegister {
                name: name.clone(),
                params: params.iter().map(|(k, v)| format!("{k}={v}")).collect(),
            });
        }
        Declaration::Use { name, overrides } => {
            let Some((params, _, _)) = defs.get(name).cloned() else {
                return;
            };
            let bindings = if overrides.is_empty() && params.is_empty() {
                HashMap::new()
            } else {
                build_bindings(&params, overrides)
            };
            events.push(TraceEvent::UseExpand {
                name: name.clone(),
                bindings,
            });
        }
    }
}

fn process_declarations_traced(
    decls: &[Declaration],
    ps: &mut Runtime,
    inits: &mut Vec<Init>,
    events: &mut Vec<TraceEvent>,
) {
    for decl in decls {
        trace_declaration(decl, &ps.defs, events);
        process_one_declaration(decl, ps, inits);
    }
}

/// Extract and remove the internal label map from the document root.
fn take_label_map(doc: &mut Document) -> HashMap<String, LabelEntry> {
    let map = doc
        .content
        .attrs
        .properties
        .get("__label_map")
        .map(decode_label_map)
        .unwrap_or_default();
    if !map.is_empty() {
        doc.content.attrs.properties.remove("__label_map");
    }
    map
}

/// Common preamble for ResolveCrossRefs (traced and non-traced).
fn with_label_map(doc: &mut Document, walk: impl FnOnce(&mut Node, &HashMap<String, LabelEntry>)) {
    let label_map = take_label_map(doc);
    if !label_map.is_empty() {
        walk(&mut doc.content, &label_map);
    }
}

// ── Traced ResolveCrossRefs ──

fn resolve_refs_traced(doc: &mut Document, events: &mut Vec<TraceEvent>) {
    with_label_map(doc, |root, lm| resolve_refs_walk_traced(root, lm, events));
}

fn resolve_refs_walk_traced(
    node: &mut Node,
    label_map: &HashMap<String, LabelEntry>,
    events: &mut Vec<TraceEvent>,
) {
    events.push(TraceEvent::NodeEnter {
        pass: "CrossRefs".into(),
        node_id: node.attrs.id.clone(),
        kind: node_kind_name(&node.kind).to_string(),
        node_properties: node.attrs.properties.clone(),
    });

    if let NodeKind::CrossRef { target } = &node.kind
        && let Some(entry) = label_map.get(target)
    {
        let display = format_label_display(&entry.label_template, &entry.number);
        node.attrs
            .properties
            .insert("text".to_string(), Value::Text(display.clone()));
        node.attrs
            .properties
            .insert("href".to_string(), Value::Text(format!("#{target}")));
        events.push(TraceEvent::ResolveRef {
            node_id: node.attrs.id.clone(),
            target: target.clone(),
            resolved_text: display,
            node_properties: node.attrs.properties.clone(),
        });
    }

    for child in &mut node.children {
        resolve_refs_walk_traced(child, label_map, events);
    }

    events.push(TraceEvent::NodeExit {
        pass: "CrossRefs".into(),
        node_id: node.attrs.id.clone(),
        node_properties: node.attrs.properties.clone(),
    });
}

// ── @arg resolution ──

/// Build a parameter name → value map from defaults and overrides.
fn build_bindings(
    params: &[(String, String)],
    overrides: &[(String, String)],
) -> HashMap<String, String> {
    params
        .iter()
        .map(|(name, default)| {
            let value = overrides
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| default.clone());
            (name.clone(), value)
        })
        .collect()
}

/// Resolve all `Bind::Arg` references in a list of declarations.
fn resolve_decls(
    decls: &[Declaration],
    bindings: &HashMap<String, String>,
) -> Vec<Declaration> {
    decls.iter().map(|d| resolve_decl(d, bindings)).collect()
}

fn resolve_decl(decl: &Declaration, bindings: &HashMap<String, String>) -> Declaration {
    match decl {
        Declaration::VarDecl(vd) => {
            let mut vd = vd.clone();
            vd.value = resolve_bind(&vd.value, bindings, |s| {
                crate::parse::parse_initial_value(s)
            });
            vd.formats = resolve_bind(&vd.formats, bindings, |s| {
                crate::parse::parse_format_list(s)
            });
            Declaration::VarDecl(vd)
        }
        Declaration::OnDirective(on) => {
            let mut on = on.clone();
            on.actions = on.actions.iter().map(|a| resolve_action(a, bindings)).collect();
            Declaration::OnDirective(on)
        }
        Declaration::Def { name, params, body, inits: def_inits } => Declaration::Def {
            name: name.clone(),
            params: params.clone(),
            body: resolve_decls(body, bindings),
            inits: resolve_inits(def_inits, bindings),
        },
        Declaration::Use { .. } => decl.clone(),
    }
}

fn resolve_action(action: &Action, bindings: &HashMap<String, String>) -> Action {
    match action {
        Action::Prepend { template } => Action::Prepend {
            template: resolve_bind(template, bindings, |s| s.to_string()),
        },
        Action::Append { template } => Action::Append {
            template: resolve_bind(template, bindings, |s| s.to_string()),
        },
        Action::SetProperty { property, value } => Action::SetProperty {
            property: property.clone(),
            value: resolve_bind(value, bindings, crate::parse::parse_init_value),
        },
        Action::Reset { var, to } => Action::Reset {
            var: var.clone(),
            to: resolve_bind(to, bindings, crate::parse::parse_initial_value),
        },
        other => other.clone(),
    }
}

fn resolve_inits(inits: &[Init], bindings: &HashMap<String, String>) -> Vec<Init> {
    inits
        .iter()
        .map(|init| Init {
            selector: init.selector.clone(),
            property: init.property.clone(),
            value: resolve_bind(&init.value, bindings, |s| {
                crate::parse::parse_init_value(s)
            }),
        })
        .collect()
}

/// Resolve a `Bind<T>`: if `Arg(name)`, look up the binding and parse it; otherwise keep as-is.
fn resolve_bind<T: Clone, F: Fn(&str) -> T>(
    bind: &Bind<T>,
    bindings: &HashMap<String, String>,
    parse: F,
) -> Bind<T> {
    match bind {
        Bind::Val(_) => bind.clone(),
        Bind::Arg(name) => match bindings.get(name) {
            Some(value) => Bind::Val(parse(value)),
            None => bind.clone(),
        },
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
            if let Some(Value::Number(n)) = state.get_mut(var) {
                *n += 1.0;
            }
        }
        Action::Decrement { var } => {
            if let Some(Value::Number(n)) = state.get_mut(var) {
                *n -= 1.0;
            }
        }
        Action::Pop { list, into } => {
            if let Some(Value::List(items)) = state.get_mut(list)
                && let Some(popped) = items.pop()
                && let Some(target) = into
            {
                state.insert(target.clone(), popped);
            }
        }
        Action::Push { list, value } => {
            let push_val = match value {
                PushValue::Var(name) => state.get(name).cloned().unwrap_or(Value::Number(0.0)),
                PushValue::Literal(v) => v.clone(),
            };
            if let Some(Value::List(items)) = state.get_mut(list) {
                items.push(push_val);
            }
        }
        Action::Reset { var, to } => {
            // Use the provided reset value, or fall back to var's initial value.
            let to = to.val();
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
        Action::StoreFormatted { .. } => {
            store_formatted(action, node, state, var_meta, label_map);
        }
        Action::SetProperty { property, value } => {
            node.attrs
                .properties
                .insert(property.clone(), value.val().clone());
        }
        Action::Prepend { template } => {
            let text = interpolate_template(template.val(), &node.attrs.properties);
            // Idempotent: update existing content node if present
            if let Some(first) = node.children.first_mut() {
                if first.attrs.properties.contains_key("__content") {
                    first.kind = NodeKind::Text(text);
                    return;
                }
            }
            let mut tn = Node::text(text);
            tn.attrs.properties
                .insert("__content".to_string(), Value::Bool(true));
            node.children.insert(0, tn);
        }
        Action::Append { template } => {
            let text = interpolate_template(template.val(), &node.attrs.properties);
            // Idempotent: update existing content node if present
            if let Some(last) = node.children.last_mut() {
                if last.attrs.properties.contains_key("__content_append") {
                    last.kind = NodeKind::Text(text);
                    return;
                }
            }
            let mut tn = Node::text(text);
            tn.attrs.properties
                .insert("__content_append".to_string(), Value::Bool(true));
            node.children.push(tn);
        }
    }
}

fn store_formatted(
    action: &Action,
    node: &mut Node,
    state: &WalkState,
    var_meta: &HashMap<String, VarMeta>,
    label_map: &mut HashMap<String, LabelEntry>,
) {
    let Action::StoreFormatted {
        var,
        property,
        formats,
        cycle,
    } = action
    else {
        return;
    };
    let Some(val) = state.get(var) else { return };
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
        .insert(property.to_string(), Value::Text(number.clone()));

    // Build label map entry for cross-refs.
    if node.attrs.auto_id {
        return;
    }
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
            counts
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let n = match v {
                        Value::Number(n) => *n as u32,
                        _ => 0,
                    };
                    let fmt = get_format_at(formats, i, cycle);
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
        if ch != '{' {
            result.push(ch);
            continue;
        }
        let key = read_until_close_brace(&mut chars);
        // Missing key → empty string (by design)
        match properties.get(&key) {
            Some(Value::Text(s)) => result.push_str(s),
            Some(Value::Number(n)) => result.push_str(&n.to_string()),
            Some(Value::Bool(b)) => result.push_str(&b.to_string()),
            _ => {}
        }
    }
    result
}

fn read_until_close_brace(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut key = String::new();
    while let Some(&c) = chars.peek() {
        if c == '}' {
            chars.next();
            break;
        }
        key.push(c);
        chars.next();
    }
    key
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
        with_label_map(doc, resolve_refs);
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

/// Number footnotes in the order they are first referenced. Each referenced
/// note and each of its references gets the note's `number`; each reference
/// also gets a unique `ref_id`, and the first one is what the note links back
/// to. A reference to a note that does not exist stays unnumbered.
pub struct NumberFootnotes;

impl Pass for NumberFootnotes {
    fn run(&mut self, doc: &mut Document) {
        let mut defined = HashSet::new();
        collect_footnote_labels(&doc.content, &mut defined);
        let mut numbers = HashMap::new();
        number_footnote_refs(&mut doc.content, &defined, &mut numbers);
        number_footnote_defs(&mut doc.content, &numbers);
    }
}

fn collect_footnote_labels(node: &Node, labels: &mut HashSet<String>) {
    if let NodeKind::FootnoteDef { label } = &node.kind {
        labels.insert(label.clone());
    }
    for child in &node.children {
        collect_footnote_labels(child, labels);
    }
}

/// Number references in document order. `numbers` maps a label to its number
/// and how many references to it have been seen.
fn number_footnote_refs(
    node: &mut Node,
    defined: &HashSet<String>,
    numbers: &mut HashMap<String, (usize, usize)>,
) {
    if let NodeKind::FootnoteRef { label } = &node.kind
        && defined.contains(label)
    {
        let next = numbers.len() + 1;
        let (number, seen) = numbers.entry(label.clone()).or_insert((next, 0));
        *seen += 1;
        let id = match *seen {
            1 => format!("fnref-{label}"),
            n => format!("fnref-{label}-{n}"),
        };
        let properties = &mut node.attrs.properties;
        properties.insert("number".into(), Value::Number(*number as f64));
        properties.insert("ref_id".into(), Value::Text(id));
    }
    for child in &mut node.children {
        number_footnote_refs(child, defined, numbers);
    }
}

fn number_footnote_defs(node: &mut Node, numbers: &HashMap<String, (usize, usize)>) {
    if let NodeKind::FootnoteDef { label } = &node.kind
        && let Some((number, _)) = numbers.get(label)
    {
        node.attrs.properties.insert("number".into(), Value::Number(*number as f64));
    }
    for child in &mut node.children {
        number_footnote_defs(child, numbers);
    }
}

/// Compile raw blocks, preview code blocks and formatted leaves, storing
/// results in `attrs.properties`.
///
/// Must run after `CascadeInits` (reads the `preview` and `format` properties).
/// For each `RawBlock`, compiles via the provided `RawBlockCompiler`.
/// For each `CodeBlock` with `preview`, compiles using the preview format.
/// For each `Math` or inline `Code` with a `format` property, compiles using
/// that format. Nothing sets `format` by default, so they compile only as configured.
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
        NodeKind::Math { source, .. } | NodeKind::Code(source) => {
            match node.attrs.properties.get("format") {
                Some(Value::Text(format)) => Some((format.clone(), source.clone())),
                _ => None,
            }
        }
        _ => None,
    };
    if let Some((default_fmt, source)) = compile_target {
        let format = get_compile_format(&node.attrs, &default_fmt);
        let placement = placement(&node.kind);
        store_compiled(node, compiler, &format, &source, placement);
    }

    for child in &mut node.children {
        compile_blocks(child, compiler);
    }
}

/// Math set apart with `$$` is a block; other math and inline code sit in a line.
fn placement(kind: &NodeKind) -> Placement {
    match kind {
        NodeKind::Math { display: false, .. } | NodeKind::Code(_) => Placement::Inline,
        _ => Placement::Block,
    }
}

fn store_compiled(
    node: &mut Node,
    compiler: &dyn RawBlockCompiler,
    format: &str,
    source: &str,
    placement: Placement,
) {
    let (key, val) = match compiler.compile(format, source, placement) {
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
            value: Bind::Val(Value::Bool(true)),
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
{@var(fig, 0, format=\"1\")}
{@init(figure, series=\"fig\")}
{@on([series=\"fig\"], @inc(fig), @store(number, fig))}
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

    /// Hierarchical numbering preamble using @var/@on primitives.
    /// Uses format tokens `fmts` (e.g. `["I","1"]`).
    fn hierarchical_preamble(fmts: &[&str], cycle: bool) -> String {
        let fmt_str = fmts
            .iter()
            .map(|f| format!("\"{}\"", f))
            .collect::<Vec<_>>()
            .join(",");
        let cycle_str = if cycle { ", cycle=true" } else { "" };
        format!(
            r#"{{@var(index, [0], format=[{fmt_str}]{cycle_str})}}
{{@var(val, 0)}}
{{@init(heading, series="index")}}
{{@on(enter, section > section, @pop(index, val), @inc(val), @push(index, val), @push(index, 0))}}
{{@on(exit, section > section, @pop(index))}}
{{@on([series="index"], @pop(index, val), @store(number, index), @push(index, val))}}"#
        )
    }

    #[test]
    fn hierarchical_section_numbering() {
        reset_id_counter();
        let input = format!(
            "{}\n\n# Introduction {{#sec:intro}}\n\n## Background {{#sec:bg}}\n\n## Methods {{#sec:methods}}\n\n# Results {{#sec:results}}",
            hierarchical_preamble(&["I", "1"], false)
        );
        let mut doc = parse(&input).unwrap();
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
        let input = format!(
            "{preamble}\n\
{{@var(fig, 0, format=\"1\")}}\n\
{{@init(figure, series=\"fig\")}}\n\
{{@on([series=\"fig\"], @inc(fig), @store(number, fig))}}\n\
{{@on(enter, section > section, @reset(fig))}}\n\
{{@hint(figure, label=\"Figure {{n}}\")}}\n\n\
# Chapter One\n\n\
![A](a.png){{#fig:a}}\n\n\
![B](b.png){{#fig:b}}\n\n\
# Chapter Two\n\n\
![C](c.png){{#fig:c}}",
            preamble = hierarchical_preamble(&["I", "1"], false)
        );
        let mut doc = parse(&input).unwrap();
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
{@var(fig, 0, format=\"1\")}
{@init(figure, series=\"fig\")}
{@on([series=\"fig\"], @inc(fig), @store(number, fig))}
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
{@var(fig, 0, format=\"1\")}
{@init(figure, series=\"fig\")}
{@on([series=\"fig\"], @inc(fig), @store(number, fig))}

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
        let input = format!(
            "{}\n\n# Chapter {{#ch1}}\n\n## Section {{#s1}}\n\n### Sub {{#sub1}}\n\n## Section Two {{#s2}}",
            hierarchical_preamble(&["I", "1", "a"], false)
        );
        let mut doc = parse(&input).unwrap();
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
        let input = format!(
            "{}\n\n# Top {{#t}}\n\n## Mid {{#m}}\n\n### Deep {{#d}}",
            hierarchical_preamble(&["I", "1"], false)
        );
        let mut doc = parse(&input).unwrap();
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
{@var(fig, 0, format=\"1\")}
{@var(tbl, 0, format=\"1\")}
{@init(figure, series=\"fig\")}
{@init(table, series=\"tbl\")}
{@on([series=\"fig\"], @inc(fig), @store(number, fig))}
{@on([series=\"tbl\"], @inc(tbl), @store(number, tbl))}
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
        let input = format!(
            "{}\n\n# Ch {{#c1}}\n\n## S {{#s1}}\n\n### Sub {{#sub1}}\n\n#### Deep {{#d1}}",
            hierarchical_preamble(&["I", "a"], true)
        );
        let mut doc = parse(&input).unwrap();
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
    fn has_declarations_nested() {
        reset_id_counter();
        // Counter declared inside a subsection
        let input = "# Chapter\n\n{@var(fig, 0, format=\"1\")}\n{@init(figure, series=\"fig\")}\n{@on([series=\"fig\"], @inc(fig), @store(number, fig))}\n\n![A](a.png){#fig:a}";
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
        let input = "{@var(fig, 0, format=\"1\")}";
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
        fn compile(&self, format: &str, source: &str, placement: Placement) -> Result<String, String> {
            if format == "fail" {
                Err("compile error".into())
            } else {
                Ok(format!("<compiled {placement:?}>{source}</compiled>"))
            }
        }
    }

    fn compiled_text(node: &Node) -> Option<&str> {
        match node.attrs.properties.get("compiled") {
            Some(Value::Text(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    #[test]
    fn compile_blocks_math_only_when_configured() {
        reset_id_counter();
        let mut doc = parse("Inline $x^2$ here.").unwrap();
        HoistDirectives.run(&mut doc);
        CascadeInits.run(&mut doc);
        CompileBlocks::new(&TestCompiler).run(&mut doc);

        let maths = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Math { .. }));
        assert_eq!(maths.len(), 1);
        assert!(compiled_text(maths[0]).is_none(), "math without a format stays as source");
    }

    #[test]
    fn compile_blocks_math_with_format() {
        reset_id_counter();
        let input = "{@hint(math, format=tmath)}\n\nInline $x^2$ here.\n\n$$\ny\n$$";
        let mut doc = parse(input).unwrap();
        HoistDirectives.run(&mut doc);
        CascadeInits.run(&mut doc);
        CompileBlocks::new(&TestCompiler).run(&mut doc);

        let maths = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Math { .. }));
        assert_eq!(maths.len(), 2);
        assert_eq!(compiled_text(maths[0]), Some("<compiled Inline>x^2</compiled>"));
        assert_eq!(compiled_text(maths[1]), Some("<compiled Block>\ny\n</compiled>"));
    }

    #[test]
    fn compile_blocks_math_format_by_display() {
        reset_id_counter();
        let input = "{@hint(math[display=\"true\"], format=fail)}\n\nInline $x$.\n\n$$\ny\n$$";
        let mut doc = parse(input).unwrap();
        HoistDirectives.run(&mut doc);
        CascadeInits.run(&mut doc);
        CompileBlocks::new(&TestCompiler).run(&mut doc);

        let maths = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Math { .. }));
        assert!(!maths[0].attrs.properties.contains_key("compile_error"));
        assert!(compiled_text(maths[0]).is_none());
        assert!(maths[1].attrs.properties.contains_key("compile_error"));
    }

    #[test]
    fn compile_blocks_inline_code_with_format() {
        reset_id_counter();
        let input = "{@hint(code-inline, format=tcode)}\n\nSome `x` code.";
        let mut doc = parse(input).unwrap();
        HoistDirectives.run(&mut doc);
        CascadeInits.run(&mut doc);
        CompileBlocks::new(&TestCompiler).run(&mut doc);

        let codes = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Code(_)));
        assert_eq!(compiled_text(codes[0]), Some("<compiled Inline>x</compiled>"));
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

    // ── Parent + child reset ──

    #[test]
    fn parent_child_reset() {
        reset_id_counter();
        // Hierarchical heading counter + flat figure counter that resets on section enter
        let input = format!(
            "{preamble}\n\
{{@var(fig, 0, format=\"1\")}}\n\
{{@init(figure, series=\"fig\")}}\n\
{{@on([series=\"fig\"], @inc(fig), @store(number, fig))}}\n\
{{@on(enter, section > section, @reset(fig))}}\n\
{{@hint(figure, label=\"Figure {{n}}\")}}\n\n\
# Chapter 1 {{#ch1}}\n\n\
![A](a.png){{#fig:a}}\n\n\
![B](b.png){{#fig:b}}\n\n\
# Chapter 2 {{#ch2}}\n\n\
![C](c.png){{#fig:c}}",
            preamble = hierarchical_preamble(&["I", "1"], false)
        );
        let mut doc = parse(&input).unwrap();
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
        let input = format!(
            "{}\n\n# Introduction\n\n## Background\n\n## Methods\n\n# Results",
            hierarchical_preamble(&["I", "1"], false)
        );
        let mut doc = parse(&input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 4);
        assert_eq!(get_number(headings[0]), Some("I"));
        assert_eq!(get_number(headings[1]), Some("I.1"));
        assert_eq!(get_number(headings[2]), Some("I.2"));
        assert_eq!(get_number(headings[3]), Some("II"));
    }

    #[test]
    fn section_nesting_drives_numbering_without_h1() {
        // When h1 is absent, sections start at level 2 directly under root.
        // Numbering is driven by section nesting, not heading level.
        reset_id_counter();
        let input = format!(
            "{}\n\n## Background\n\n## Methods",
            hierarchical_preamble(&["1"], false)
        );
        let mut doc = parse(&input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 2);
        assert_eq!(get_number(headings[0]), Some("1"));
        assert_eq!(get_number(headings[1]), Some("2"));
    }

    #[test]
    fn on_reset() {
        reset_id_counter();
        let input = format!(
            "{preamble}\n\
{{@var(fig, 0, format=\"1\")}}\n\
{{@init(figure, series=\"fig\")}}\n\
{{@on([series=\"fig\"], @inc(fig), @store(number, fig))}}\n\
{{@on(enter, section > section, @reset(fig))}}\n\n\
# Chapter One\n\n\
![A](a.png){{#fig:a}}\n\n\
![B](b.png){{#fig:b}}\n\n\
# Chapter Two\n\n\
![C](c.png){{#fig:c}}",
            preamble = hierarchical_preamble(&["I", "1"], false)
        );
        let mut doc = parse(&input).unwrap();
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
        let input = format!(
            "{}\n{{@on(heading, @prepend(@text(\"{{number}}. \")))}}\n\n# Introduction",
            hierarchical_preamble(&["I", "1"], false)
        );
        let mut doc = parse(&input).unwrap();
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
        let input = format!(
            "{}\n{{@on(heading, @append(@text(\" §\")))}}\n\n# Introduction",
            hierarchical_preamble(&["I", "1"], false)
        );
        let mut doc = parse(&input).unwrap();
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
        let input = format!(
            "{}\n{{@content(heading, \"{{number}}. \")}}\n\n# Introduction\n\n## Background",
            hierarchical_preamble(&["I", "1"], false)
        );
        let mut doc = parse(&input).unwrap();
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

    // ── Push/Pop/Exit engine tests ──

    #[test]
    fn pop_push_inc_on_list() {
        // Direct push/pop/inc operations via @var/@on
        reset_id_counter();
        let input = "\
{@var(idx, [0], format=[\"1\"])}
{@var(v, 0)}
{@init(heading, series=\"idx\")}
{@on([series=\"idx\"], @pop(idx, v), @inc(v), @push(idx, v), @store(number, idx))}

# One

# Two

# Three";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 3);
        // Each heading pops 0, incs to 1/2/3, pushes back
        assert_eq!(get_number(headings[0]), Some("1"));
        assert_eq!(get_number(headings[1]), Some("2"));
        assert_eq!(get_number(headings[2]), Some("3"));
    }

    #[test]
    fn exit_actions_fire_after_children() {
        // @on(exit, section) should fire after children are processed
        reset_id_counter();
        let input = "\
{@var(depth, 0)}
{@on(enter, section > section, @inc(depth))}
{@on(exit, section > section, @dec(depth))}
{@on(heading, @store(number, depth))}

# One

## Nested";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        // h1 fires after section(1) enters (depth=1)
        assert_eq!(get_number(headings[0]), Some("1"));
        // h2 fires after section(2) enters (depth=2)
        assert_eq!(get_number(headings[1]), Some("2"));
    }

    #[test]
    fn pop_discard() {
        // @pop(list) without target discards the value
        reset_id_counter();
        let input = "\
{@var(stk, [0], format=[\"1\"])}
{@var(v, 0)}
{@on(heading, @push(stk, 99))}
{@on(figure, @pop(stk), @store(number, stk))}

# Heading

![img](x.png)";
        let mut doc = parse(input).unwrap();
        resolve_document(&mut doc);

        let figs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Figure));
        // stack was [0], heading pushed 99 → [0, 99], figure pops 99 (discard) → [0]
        assert_eq!(get_number(figs[0]), Some("0"));
    }

    #[test]
    fn def_and_use_basic() {
        use crate::parse::parse_prelude;

        let prelude = "\
@def(counter) {
  @var(n, 0);
  @on(enter, heading, @inc(n), @store(number, n));
}
@use(counter);
";
        let (decls, _inits): (Vec<Declaration>, _) = parse_prelude(prelude);
        // Should have Def and Use declarations
        assert!(decls.iter().any(|d| matches!(d, Declaration::Def { .. })));
        assert!(decls.iter().any(|d| matches!(d, Declaration::Use { .. })));
    }

    #[test]
    fn block_syntax_parsing() {
        use crate::parse::parse_prelude;

        let prelude = "\
@var(x, 0);
@on(enter, heading) {
  @inc(x);
  @store(number, x);
}
";
        let (decls, _inits): (Vec<Declaration>, _) = parse_prelude(prelude);
        assert!(decls.iter().any(|d| matches!(d, Declaration::VarDecl(_))));
        assert!(decls.iter().any(|d| matches!(d, Declaration::OnDirective(_))));
    }

    // ── Prelude integration tests ──

    /// Inject prelude declarations onto a document root, mimicking InjectPrelude.
    fn inject_prelude(doc: &mut crate::ast::Document, prelude_text: &str) {
        use crate::parse::parse_prelude;
        let (decls, inits) = parse_prelude(prelude_text);
        doc.content.attrs.declarations.extend(decls);
        doc.content.attrs.inits.extend(inits);
    }

    const PRELUDE: &str = r#"
@def(numbering, format=["I","1"], content="{number}. ") {
  @var(fig, 0);
  @var(index, [0], format=@arg(format));
  @var(val, 0);

  @on(enter, section > section) {
    @pop(index, val);
    @inc(val);
    @push(index, val);
    @push(index, 0);
  }

  @on(exit, section > section) {
    @pop(index);
  }

  @on(enter, heading) {
    @pop(index, val);
    @store(number, index);
    @push(index, val);
  }

  @on(enter, figure) {
    @inc(fig);
    @store(number, fig);
  }

  @content(heading, @arg(content));
}

@use(numbering);
@init(figure, label="Figure {n}");
@init(heading, label="Section {n}");
"#;

    #[test]
    fn prelude_numbering_integration() {
        reset_id_counter();
        let md = "\
# Introduction {#sec:intro}

## Background {#sec:bg}

## Methods {#sec:methods}

# Results {#sec:results}";
        let mut doc = parse(md).unwrap();
        inject_prelude(&mut doc, PRELUDE);
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 4);
        assert_eq!(get_number(headings[0]), Some("I"), "Introduction = I");
        assert_eq!(get_number(headings[1]), Some("I.1"), "Background = I.1");
        assert_eq!(get_number(headings[2]), Some("I.2"), "Methods = I.2");
        assert_eq!(get_number(headings[3]), Some("II"), "Results = II");
    }

    #[test]
    fn prelude_figures() {
        reset_id_counter();
        let md = "\
![First](a.png){#fig:first}

![Second](b.png){#fig:second}

![Third](c.png){#fig:third}";
        let mut doc = parse(md).unwrap();
        inject_prelude(&mut doc, PRELUDE);
        resolve_document(&mut doc);

        let figs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Figure));
        assert_eq!(figs.len(), 3);
        assert_eq!(get_number(figs[0]), Some("1"));
        assert_eq!(get_number(figs[1]), Some("2"));
        assert_eq!(get_number(figs[2]), Some("3"));
    }

    #[test]
    fn prelude_deep_nesting() {
        reset_id_counter();
        let md = "\
# Part {#p1}

## Chapter {#c1}

### Section {#s1}

## Chapter Two {#c2}";
        let mut doc = parse(md).unwrap();
        inject_prelude(&mut doc, PRELUDE);
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 4);
        assert_eq!(get_number(headings[0]), Some("I"), "Part = I");
        assert_eq!(get_number(headings[1]), Some("I.1"), "Chapter = I.1");
        assert_eq!(get_number(headings[2]), Some("I.1.1"), "Section = I.1.1");
        assert_eq!(get_number(headings[3]), Some("I.2"), "Chapter Two = I.2");
    }

    #[test]
    fn prelude_cross_ref_labels() {
        reset_id_counter();
        let md = "\
# Intro {#sec:intro}

![Diagram](d.png){#fig:d}

See @sec:intro and @fig:d.";
        let mut doc = parse(md).unwrap();
        inject_prelude(&mut doc, PRELUDE);
        resolve_document(&mut doc);

        let refs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::CrossRef { .. }));
        assert_eq!(refs.len(), 2);
        let sec_text = refs[0].attrs.properties.get("text").and_then(|v| match v {
            Value::Text(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(sec_text, Some("Section I"));
        let fig_text = refs[1].attrs.properties.get("text").and_then(|v| match v {
            Value::Text(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(fig_text, Some("Figure 1"));
    }

    #[test]
    fn prelude_content_in_def() {
        reset_id_counter();
        let md = "\
# Introduction {#sec:intro}

## Background {#sec:bg}";
        let mut doc = parse(md).unwrap();
        inject_prelude(&mut doc, PRELUDE);
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        assert_eq!(headings.len(), 2);
        // @content(heading, "{number}. ") should prepend a text node
        let first_child = &headings[0].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            assert_eq!(t, "I. ", "h1 should get 'I. ' prefix");
        } else {
            panic!("expected Text node as first child of heading, got {:?}", first_child.kind);
        }
        let first_child = &headings[1].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            assert_eq!(t, "I.1. ", "h2 should get 'I.1. ' prefix");
        } else {
            panic!("expected Text node as first child of heading, got {:?}", first_child.kind);
        }
    }

    #[test]
    fn prelude_content_and_cross_ref() {
        reset_id_counter();
        let md = "\
# Intro {#sec:intro}

See @sec:intro.";
        let mut doc = parse(md).unwrap();
        inject_prelude(&mut doc, PRELUDE);
        resolve_document(&mut doc);

        // Heading gets visible number prefix
        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        let first_child = &headings[0].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            assert_eq!(t, "I. ");
        } else {
            panic!("expected Text prefix");
        }

        // Cross-ref still resolves to label (not the visible prefix)
        let refs = find_all(&doc.content, |n| matches!(n.kind, NodeKind::CrossRef { .. }));
        assert_eq!(refs.len(), 1);
        let sec_text = refs[0].attrs.properties.get("text").and_then(|v| match v {
            Value::Text(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(sec_text, Some("Section I"));
    }

    #[test]
    fn def_params_defaults() {
        // @use with no overrides should use default params
        reset_id_counter();
        let prelude = r#"
@def(numbering, format=["I","1"], content="{number}. ") {
  @var(fig, 0);
  @var(index, [0], format=@arg(format));
  @var(val, 0);
  @on(enter, section > section) {
    @pop(index, val); @inc(val); @push(index, val); @push(index, 0);
  }
  @on(exit, section > section) { @pop(index); }
  @on(enter, heading) {
    @pop(index, val); @store(number, index); @push(index, val);
  }
  @on(enter, figure) { @inc(fig); @store(number, fig); }
  @content(heading, @arg(content));
}
@use(numbering);
@init(heading, label="Section {n}");
"#;
        let md = "# Hello";
        let mut doc = parse(md).unwrap();
        inject_prelude(&mut doc, prelude);
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        let first_child = &headings[0].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            // Default format is ["I","1"], so top-level heading should be Roman "I"
            assert_eq!(t, "I. ");
        } else {
            panic!("expected Text prefix, got {:?}", first_child.kind);
        }
    }

    #[test]
    fn def_params_override() {
        // @use with overrides should substitute @arg values
        reset_id_counter();
        let prelude = r#"
@def(numbering, format=["I","1"], content="{number}. ") {
  @var(fig, 0);
  @var(index, [0], format=@arg(format));
  @var(val, 0);
  @on(enter, section > section) {
    @pop(index, val); @inc(val); @push(index, val); @push(index, 0);
  }
  @on(exit, section > section) { @pop(index); }
  @on(enter, heading) {
    @pop(index, val); @store(number, index); @push(index, val);
  }
  @on(enter, figure) { @inc(fig); @store(number, fig); }
  @content(heading, @arg(content));
}
@use(numbering, format=["1","1"]);
@init(heading, label="Section {n}");
"#;
        let md = "# Hello";
        let mut doc = parse(md).unwrap();
        inject_prelude(&mut doc, prelude);
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        let first_child = &headings[0].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            // Overridden format is ["1","1"], so top-level heading should be Arabic "1"
            assert_eq!(t, "1. ");
        } else {
            panic!("expected Text prefix, got {:?}", first_child.kind);
        }
    }

    #[test]
    fn def_params_partial_override() {
        // Override only content, keep default format
        reset_id_counter();
        let prelude = r#"
@def(numbering, format=["I","1"], content="{number}. ") {
  @var(fig, 0);
  @var(index, [0], format=@arg(format));
  @var(val, 0);
  @on(enter, section > section) {
    @pop(index, val); @inc(val); @push(index, val); @push(index, 0);
  }
  @on(exit, section > section) { @pop(index); }
  @on(enter, heading) {
    @pop(index, val); @store(number, index); @push(index, val);
  }
  @on(enter, figure) { @inc(fig); @store(number, fig); }
  @content(heading, @arg(content));
}
@use(numbering, content="{number}) ");
@init(heading, label="Section {n}");
"#;
        let md = "# Hello";
        let mut doc = parse(md).unwrap();
        inject_prelude(&mut doc, prelude);
        resolve_document(&mut doc);

        let headings = find_all(&doc.content, |n| matches!(n.kind, NodeKind::Heading { .. }));
        let first_child = &headings[0].children[0];
        if let NodeKind::Text(ref t) = first_child.kind {
            // Format defaults to Roman "I", content overridden to ") "
            assert_eq!(t, "I) ");
        } else {
            panic!("expected Text prefix, got {:?}", first_child.kind);
        }
    }

    // ── Runtime dispatch tests ──

    #[test]
    fn dispatch_click_fires_trigger() {
        reset_id_counter();
        let mut doc = parse(concat!(
            "{@var(count, 0)}\n",
            "{@on(click, heading, @inc(count), @store(number, count))}\n\n",
            "# Hello {#greeting}\n",
        ))
        .unwrap();

        HoistDirectives.run(&mut doc);
        CascadeInits.run(&mut doc);
        let mut rt = Runtime::init(&mut doc);

        // Before click: heading has no 'number' property.
        let heading = find_node(&doc.content, |n| {
            matches!(&n.kind, NodeKind::Heading { .. })
        })
        .unwrap();
        assert!(heading.attrs.properties.get("number").is_none());

        // Dispatch click on the heading.
        let changed = rt.dispatch(&mut doc, Event::Click, "greeting");
        assert!(!changed.is_empty(), "click should fire a trigger");

        // After click: heading should have number="1".
        let heading = find_node(&doc.content, |n| {
            matches!(&n.kind, NodeKind::Heading { .. })
        })
        .unwrap();
        assert_eq!(
            heading.attrs.properties.get("number"),
            Some(&Value::Text("1".into())),
        );

        // Second click: number should be "2".
        rt.dispatch(&mut doc, Event::Click, "greeting");
        let heading = find_node(&doc.content, |n| {
            matches!(&n.kind, NodeKind::Heading { .. })
        })
        .unwrap();
        assert_eq!(
            heading.attrs.properties.get("number"),
            Some(&Value::Text("2".into())),
        );
    }

    #[test]
    fn dispatch_no_matching_trigger() {
        reset_id_counter();
        let mut doc = parse(concat!(
            "{@var(count, 0)}\n",
            "{@on(click, figure, @increment(count))}\n\n",
            "# Hello {#greeting}\n",
        ))
        .unwrap();

        HoistDirectives.run(&mut doc);
        CascadeInits.run(&mut doc);
        let mut rt = Runtime::init(&mut doc);

        // Click on heading, but trigger is for figure — should not fire.
        let changed = rt.dispatch(&mut doc, Event::Click, "greeting");
        assert!(changed.is_empty());
    }

    #[test]
    fn dispatch_unknown_node_id() {
        reset_id_counter();
        let mut doc = parse("# Hello\n").unwrap();

        HoistDirectives.run(&mut doc);
        CascadeInits.run(&mut doc);
        let mut rt = Runtime::init(&mut doc);

        let changed = rt.dispatch(&mut doc, Event::Click, "nonexistent");
        assert!(changed.is_empty());
    }

    #[test]
    fn dispatch_reapplies_content_template() {
        reset_id_counter();
        let mut doc = parse(concat!(
            "{@var(count, 0, format=\"1\")}\n",
            "{@on(click, heading, @inc(count), @store(number, count))}\n",
            "{@content(heading, \"{number}. \")}\n\n",
            "# Apple {#fruit:apple}\n",
        ))
        .unwrap();

        HoistDirectives.run(&mut doc);
        CascadeInits.run(&mut doc);
        let mut rt = Runtime::init(&mut doc);

        // Before click: heading has content node but number is empty so text is ". "
        let heading = find_node(&doc.content, |n| {
            matches!(&n.kind, NodeKind::Heading { .. })
        })
        .unwrap();
        assert!(heading.attrs.properties.get("number").is_none());

        // Click: should set number="1" and update prepended text to "1. "
        rt.dispatch(&mut doc, Event::Click, "fruit:apple");
        let heading = find_node(&doc.content, |n| {
            matches!(&n.kind, NodeKind::Heading { .. })
        })
        .unwrap();
        assert_eq!(
            heading.attrs.properties.get("number"),
            Some(&Value::Text("1".into())),
        );
        // First child should be the content text node with "1. "
        let first_child = &heading.children[0];
        assert!(
            matches!(&first_child.kind, NodeKind::Text(s) if s == "1. "),
            "expected Text(\"1. \"), got {:?}",
            first_child.kind,
        );
        assert!(first_child.attrs.properties.contains_key("__content"));
    }

    #[test]
    fn dispatch_content_is_idempotent() {
        reset_id_counter();
        let mut doc = parse(concat!(
            "{@var(count, 0, format=\"1\")}\n",
            "{@on(click, heading, @inc(count), @store(number, count))}\n",
            "{@content(heading, \"{number}. \")}\n\n",
            "# Apple {#fruit:apple}\n",
        ))
        .unwrap();

        HoistDirectives.run(&mut doc);
        CascadeInits.run(&mut doc);
        let mut rt = Runtime::init(&mut doc);

        // Click three times
        rt.dispatch(&mut doc, Event::Click, "fruit:apple");
        rt.dispatch(&mut doc, Event::Click, "fruit:apple");
        rt.dispatch(&mut doc, Event::Click, "fruit:apple");

        let heading = find_node(&doc.content, |n| {
            matches!(&n.kind, NodeKind::Heading { .. })
        })
        .unwrap();

        // Should have number="3"
        assert_eq!(
            heading.attrs.properties.get("number"),
            Some(&Value::Text("3".into())),
        );
        // Should still have exactly one content text node, not three
        let content_nodes: Vec<_> = heading
            .children
            .iter()
            .filter(|c| c.attrs.properties.contains_key("__content"))
            .collect();
        assert_eq!(content_nodes.len(), 1, "should not duplicate content nodes");
        assert!(
            matches!(&content_nodes[0].kind, NodeKind::Text(s) if s == "3. "),
            "expected Text(\"3. \"), got {:?}",
            content_nodes[0].kind,
        );
    }
}
