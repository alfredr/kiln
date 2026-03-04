use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

// ── Id generation ──

static NODE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_id(prefix: &str) -> String {
    let n = NODE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{n}")
}

/// Reset the id counter (useful for tests).
pub fn reset_id_counter() {
    NODE_COUNTER.store(0, Ordering::Relaxed);
}

// ── Core ──

/// A parsed document with metadata and a content tree.
#[derive(Debug, Clone)]
pub struct Document {
    pub meta: Metadata,
    pub content: Node, // root: Section { level: 0 }
}

/// A universal node in the document tree.
///
/// Container kinds (Section, Paragraph, Emphasis, etc.) hold child nodes.
/// Leaf kinds (Text, Code, CodeBlock, RawBlock, Math) carry their content
/// inline on the NodeKind variant — there is no separate `body` field.
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub attrs: Attrs,
    pub children: Vec<Node>,
}

// ── Node kinds ──

/// The type of a document node.
#[derive(Debug, Clone)]
pub enum NodeKind {
    // Structure
    Section { level: u8 },
    FileScope { source: String },
    Heading { level: u8 },
    Paragraph,
    BlockQuote,
    List { ordered: bool, start: Option<u64> },
    ListItem,
    Table,
    TableRow,
    TableCell,
    ThematicBreak,
    /// A paragraph containing only an Image, promoted to a figure.
    Figure,
    Caption,

    // Leaf blocks — carry content as String
    CodeBlock { lang: String, source: String },
    RawBlock { format: String, source: String },
    Math { display: bool, source: String },

    // Leaf inline — carry content as String
    Text(String),
    Code(String),

    // Container inline
    Emphasis,
    Strong,
    Strikethrough,
    Link { url: String },
    Image { url: String, alt: String },
    Citation { key: String },
    CrossRef { target: String },
    SoftBreak,
    HardBreak,

    // Layout
    Space { spacing: Spacing },

    // Extensible
    Custom { content_type: String },
}

// ── Attributes ──

/// Attributes attached to every node.
#[derive(Debug, Clone)]
pub struct Attrs {
    /// Always present — explicit (`{#fig:arch}`) or auto-generated.
    pub id: String,
    /// Whether this ID was auto-generated (should not be emitted in HTML).
    pub auto_id: bool,
    /// CSS-like classes (`{.wide .dark}`).
    pub classes: Vec<String>,
    /// Authored key-value properties (`{width="80%"}`).
    pub properties: HashMap<String, Value>,
    /// Complex config from `---` block within a fenced block.
    pub frontmatter: Option<serde_yaml_ng::Value>,
    /// Resources declared in this scope (e.g. series).
    pub declarations: Vec<Declaration>,
    /// Scoped hints that cascade to descendants.
    pub hints: Vec<Hint>,
    /// Resolved properties filled by the resolver.
    pub resolved: HashMap<String, Value>,
}

impl Attrs {
    pub fn new(id: String) -> Self {
        Self {
            id,
            auto_id: true,
            classes: Vec::new(),
            properties: HashMap::new(),
            frontmatter: None,
            declarations: Vec::new(),
            hints: Vec::new(),
            resolved: HashMap::new(),
        }
    }
}

impl Default for Attrs {
    fn default() -> Self {
        Self::new(next_id("node"))
    }
}

// ── The Hint Cascade ──

/// A scoped hint: "for nodes matching SELECTOR in my scope, set PROPERTY = VALUE."
///
/// Hints cascade to all descendants. When multiple hints match the same node
/// and set the same property, closest ancestor wins (innermost scope).
///
/// The same mechanism binds figures to a series AND headings to a font:
/// ```text
/// hint(id_prefix("fig:"), series: "figs")
/// hint(kind(Heading),     font_family: "Georgia")
/// ```
#[derive(Debug, Clone)]
pub struct Hint {
    pub selector: Selector,
    pub property: String,
    pub value: Value,
}

/// What nodes a hint applies to.
#[derive(Debug, Clone)]
pub enum Selector {
    /// This node only.
    This,
    /// Descendants matching a node kind name (e.g. "Heading", "Figure").
    Kind(String),
    /// Descendants with a CSS class.
    Class(String),
    /// A specific element by id.
    Id(String),
    /// All ids starting with a prefix (e.g. "fig:" matches "fig:arch").
    IdPrefix(String),
}

// ── Declarations ──

/// A named resource declared in a scope.
#[derive(Debug, Clone)]
pub enum Declaration {
    Series { id: String, config: SeriesConfig },
}

/// Configuration for a series (counter/sequence manager).
#[derive(Debug, Clone)]
pub struct SeriesConfig {
    /// Display prefix: "Figure", "Listing", "Equation".
    pub label: String,
    /// How the resolved number renders.
    pub display: SeriesDisplay,
}

/// How a series number is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesDisplay {
    /// 1, 2, 3 (flat, ignores section depth).
    Arabic,
    /// 2.3 (section.seq, hierarchical).
    Hierarchical,
    /// i, ii, iii.
    Roman,
    /// a, b, c.
    Alpha,
}

// ── Typed values ──

/// A typed value for properties, hints, and resolved attributes.
#[derive(Debug, Clone)]
pub enum Value {
    Text(String),
    Number(f64),
    Bool(bool),
    Length(Length),
    Spacing(Spacing),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
}

/// A length with a unit.
#[derive(Debug, Clone, Copy)]
pub struct Length {
    pub value: f64,
    pub unit: LengthUnit,
}

/// Length units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Em,
    Rem,
    Px,
    Pt,
    Cm,
    Mm,
    Percent,
}

/// Spacing values for layout hints.
#[derive(Debug, Clone, Copy)]
pub enum Spacing {
    Fixed(Length),
    Flex { grow: f64 },
    Auto,
}

// ── Metadata ──

/// Document metadata extracted from YAML frontmatter.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Metadata {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub slug: Option<String>,
    /// Extensible key-value pairs for user-defined metadata.
    /// Also captures `series` and `hints` keys from frontmatter,
    /// which the parser extracts into Declaration/Hint structs.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml_ng::Value>,
}

// ── Raw block compilation trait ──

/// Trait for compiling raw blocks (e.g. Typst diagrams) into HTML.
///
/// `kiln-doc` does NOT depend on `typst`. Instead, the consumer provides
/// an implementation of this trait. The kiln binary provides a `TypstCompiler`.
pub trait RawBlockCompiler {
    fn compile(&self, format: &str, source: &str) -> Result<String, String>;
}

// ── Convenience constructors ──

impl Node {
    pub fn new(kind: NodeKind) -> Self {
        let prefix = match &kind {
            NodeKind::Section { level } => format!("sec-{level}"),
            NodeKind::Heading { level } => format!("h{level}"),
            NodeKind::Paragraph => "p".into(),
            NodeKind::BlockQuote => "bq".into(),
            NodeKind::List { .. } => "list".into(),
            NodeKind::ListItem => "li".into(),
            NodeKind::Table => "table".into(),
            NodeKind::TableRow => "tr".into(),
            NodeKind::TableCell => "td".into(),
            NodeKind::ThematicBreak => "hr".into(),
            NodeKind::Figure => "fig".into(),
            NodeKind::Caption => "cap".into(),
            NodeKind::CodeBlock { .. } => "code".into(),
            NodeKind::RawBlock { .. } => "raw".into(),
            NodeKind::Math { .. } => "math".into(),
            NodeKind::Text(_) => "t".into(),
            NodeKind::Code(_) => "ic".into(),
            NodeKind::Emphasis => "em".into(),
            NodeKind::Strong => "strong".into(),
            NodeKind::Strikethrough => "del".into(),
            NodeKind::Link { .. } => "a".into(),
            NodeKind::Image { .. } => "img".into(),
            NodeKind::Citation { .. } => "cite".into(),
            NodeKind::CrossRef { .. } => "ref".into(),
            NodeKind::SoftBreak | NodeKind::HardBreak => "br".into(),
            NodeKind::FileScope { .. } => "file".into(),
            NodeKind::Space { .. } => "sp".into(),
            NodeKind::Custom { .. } => "custom".into(),
        };
        Self {
            kind,
            attrs: Attrs::new(next_id(&prefix)),
            children: Vec::new(),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.attrs.id = id.into();
        self
    }

    pub fn with_children(mut self, children: Vec<Node>) -> Self {
        self.children = children;
        self
    }

    pub fn with_classes(mut self, classes: Vec<String>) -> Self {
        self.attrs.classes = classes;
        self
    }

    // -- Leaf constructors --

    pub fn text(s: impl Into<String>) -> Self {
        Self::new(NodeKind::Text(s.into()))
    }

    pub fn code(s: impl Into<String>) -> Self {
        Self::new(NodeKind::Code(s.into()))
    }

    pub fn code_block(lang: impl Into<String>, source: impl Into<String>) -> Self {
        Self::new(NodeKind::CodeBlock {
            lang: lang.into(),
            source: source.into(),
        })
    }

    pub fn raw_block(format: impl Into<String>, source: impl Into<String>) -> Self {
        Self::new(NodeKind::RawBlock {
            format: format.into(),
            source: source.into(),
        })
    }

    pub fn math(display: bool, source: impl Into<String>) -> Self {
        Self::new(NodeKind::Math {
            display,
            source: source.into(),
        })
    }

    pub fn soft_break() -> Self {
        Self::new(NodeKind::SoftBreak)
    }

    pub fn hard_break() -> Self {
        Self::new(NodeKind::HardBreak)
    }

    pub fn thematic_break() -> Self {
        Self::new(NodeKind::ThematicBreak)
    }

    // -- Container constructors --

    pub fn section(level: u8, children: Vec<Node>) -> Self {
        Self::new(NodeKind::Section { level }).with_children(children)
    }

    pub fn heading(level: u8, children: Vec<Node>) -> Self {
        Self::new(NodeKind::Heading { level }).with_children(children)
    }

    pub fn paragraph(children: Vec<Node>) -> Self {
        Self::new(NodeKind::Paragraph).with_children(children)
    }

    pub fn blockquote(children: Vec<Node>) -> Self {
        Self::new(NodeKind::BlockQuote).with_children(children)
    }

    pub fn list(ordered: bool, start: Option<u64>, items: Vec<Node>) -> Self {
        Self::new(NodeKind::List { ordered, start }).with_children(items)
    }

    pub fn list_item(children: Vec<Node>) -> Self {
        Self::new(NodeKind::ListItem).with_children(children)
    }

    pub fn figure(children: Vec<Node>) -> Self {
        Self::new(NodeKind::Figure).with_children(children)
    }

    pub fn caption(children: Vec<Node>) -> Self {
        Self::new(NodeKind::Caption).with_children(children)
    }

    pub fn emphasis(children: Vec<Node>) -> Self {
        Self::new(NodeKind::Emphasis).with_children(children)
    }

    pub fn strong(children: Vec<Node>) -> Self {
        Self::new(NodeKind::Strong).with_children(children)
    }

    pub fn strikethrough(children: Vec<Node>) -> Self {
        Self::new(NodeKind::Strikethrough).with_children(children)
    }

    pub fn link(url: impl Into<String>, children: Vec<Node>) -> Self {
        Self::new(NodeKind::Link { url: url.into() }).with_children(children)
    }

    pub fn image(url: impl Into<String>, alt: impl Into<String>) -> Self {
        Self::new(NodeKind::Image {
            url: url.into(),
            alt: alt.into(),
        })
    }

    pub fn citation(key: impl Into<String>) -> Self {
        Self::new(NodeKind::Citation { key: key.into() })
    }

    pub fn cross_ref(target: impl Into<String>) -> Self {
        Self::new(NodeKind::CrossRef {
            target: target.into(),
        })
    }

    pub fn table(children: Vec<Node>) -> Self {
        Self::new(NodeKind::Table).with_children(children)
    }

    pub fn table_row(children: Vec<Node>) -> Self {
        Self::new(NodeKind::TableRow).with_children(children)
    }

    pub fn table_cell(children: Vec<Node>) -> Self {
        Self::new(NodeKind::TableCell).with_children(children)
    }

    pub fn file_scope(source: impl Into<String>, children: Vec<Node>) -> Self {
        Self::new(NodeKind::FileScope {
            source: source.into(),
        })
        .with_children(children)
    }
}
