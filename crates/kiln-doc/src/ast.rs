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
    Section {
        level: u8,
    },
    FileScope {
        source: String,
    },
    Heading {
        level: u8,
    },
    Paragraph,
    BlockQuote,
    List {
        ordered: bool,
        start: Option<u64>,
    },
    ListItem,
    Table,
    TableRow,
    TableCell,
    ThematicBreak,
    /// A paragraph containing only an Image, promoted to a figure.
    Figure,
    Caption,

    // Leaf blocks — carry content as String
    CodeBlock {
        lang: String,
        source: String,
    },
    RawBlock {
        format: String,
        source: String,
    },
    Math {
        display: bool,
        source: String,
    },

    // Leaf inline — carry content as String
    Text(String),
    Code(String),

    // Container inline
    Emphasis,
    Strong,
    Strikethrough,
    Link {
        url: String,
    },
    Image {
        url: String,
        alt: String,
    },
    Citation {
        key: String,
    },
    CrossRef {
        target: String,
    },
    SoftBreak,
    HardBreak,

    // Layout
    Space {
        spacing: Spacing,
    },

    // Extensible
    Custom {
        content_type: String,
    },
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
/// ```text
/// {@hint(figure, series="figs")}
/// {@hint(heading, font_family="Georgia")}
/// ```
#[derive(Debug, Clone)]
pub struct Hint {
    pub selector: Selector,
    pub property: String,
    pub value: Value,
}

// ── Selector types ──

/// Top-level selector for a hint.
#[derive(Debug, Clone)]
pub enum Selector {
    /// This node only (scoped to attachment point).
    This,
    /// A CSS-like selector expression.
    Expr(SelectorExpr),
}

/// A selector expression supporting boolean combinations and combinators.
#[derive(Debug, Clone)]
pub enum SelectorExpr {
    /// A single compound selector: `figure.wide[lang="rust"]`
    Simple(CompoundSelector),
    /// Combinator chain: `figure > caption` or `section ~> figure`
    Combinator {
        left: Box<SelectorExpr>,
        op: CombinatorOp,
        right: Box<SelectorExpr>,
    },
    /// OR: comma-separated or `|` operator.
    Or(Vec<SelectorExpr>),
    /// AND: explicit `&` operator.
    And(Vec<SelectorExpr>),
}

/// Relationship between two selectors in a combinator chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombinatorOp {
    /// `>` — direct child.
    Child,
    /// `~>` — descendant at any depth.
    Descendant,
}

impl CombinatorOp {
    /// Length of the combinator token in bytes.
    pub fn token_len(self) -> usize {
        match self {
            Self::Child => 1,
            Self::Descendant => 2,
        }
    }
}

/// A compound selector: all parts must match simultaneously (AND).
///
/// `heading.numbered[level=2]` → kind=Heading, classes=["numbered"], attrs=[level=2]
#[derive(Debug, Clone, Default)]
pub struct CompoundSelector {
    /// Kind name in canonical PascalCase (e.g. "Heading", "CodeBlock").
    pub kind: Option<String>,
    /// Explicit element ID (e.g. `#intro`).
    pub id: Option<String>,
    /// CSS classes (e.g. `.wide .dark`).
    pub classes: Vec<String>,
    /// Attribute expression from `[...]` blocks (ANDed with kind/id/classes).
    pub attr_expr: Option<AttrExpr>,
}

/// Attribute-level expression tree for `[...]` contents.
///
/// `[lang="rust", preview | level=2]` → `Or(And(Pred, Pred), Pred)`
#[derive(Debug, Clone)]
pub enum AttrExpr {
    /// A single predicate: `[lang="rust"]` or `[preview]`.
    Pred(AttrPred),
    /// All must match (`,` or `&` inside brackets).
    And(Vec<AttrExpr>),
    /// Any must match (`|` inside brackets, or tuple distribution).
    Or(Vec<AttrExpr>),
}

/// A single attribute predicate.
#[derive(Debug, Clone)]
pub struct AttrPred {
    pub name: String,
    pub op: AttrOp,
}

/// How an attribute predicate matches.
#[derive(Debug, Clone)]
pub enum AttrOp {
    /// `[attr]` — attribute exists (any value).
    Exists,
    /// `[attr="val"]` — exact match.
    Eq(String),
    /// `[attr^="val"]` — starts with.
    StartsWith(String),
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
    /// Parent series id — inherits barriers, resets this counter on parent bump.
    pub parent: Option<String>,
    /// Format tokens per depth level: `["I", "1", "a"]`.
    /// Each token is a shorthand: "1" = arabic, "I" = roman upper, "a" = alpha lower, etc.
    /// For depths beyond the array length, repeats the last element (or cycles if `cycle` is set).
    pub formats: Vec<String>,
    /// Whether the format list cycles (`depth % len`) or repeats the last element.
    pub cycle: bool,
}

/// Number formatting style for a single counter level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberFormat {
    Arabic,
    RomanUpper,
    RomanLower,
    AlphaUpper,
    AlphaLower,
}

/// A parsed format token: style with optional prefix/suffix wrapping.
#[derive(Debug, Clone)]
pub struct FormatToken {
    pub prefix: String,
    pub format: NumberFormat,
    pub suffix: String,
}

impl NumberFormat {
    /// Render a count in this format.
    pub fn render(self, n: u32) -> String {
        match self {
            Self::Arabic => n.to_string(),
            Self::RomanUpper => to_roman(n, true),
            Self::RomanLower => to_roman(n, false),
            Self::AlphaUpper => to_alpha(n, true),
            Self::AlphaLower => to_alpha(n, false),
        }
    }
}

impl FormatToken {
    /// Parse a shorthand token: "1", "I", "i", "A", "a", "(a)", "(1)", etc.
    pub fn parse(token: &str) -> Self {
        let token = token.trim();
        // Check for prefix/suffix wrapping: "(X)" or similar
        if token.len() >= 3 && token.starts_with('(') && token.ends_with(')') {
            let inner = &token[1..token.len() - 1];
            let format = Self::parse_style(inner);
            return Self {
                prefix: "(".to_string(),
                format,
                suffix: ")".to_string(),
            };
        }
        let format = Self::parse_style(token);
        Self {
            prefix: String::new(),
            format,
            suffix: String::new(),
        }
    }

    fn parse_style(s: &str) -> NumberFormat {
        match s {
            "I" => NumberFormat::RomanUpper,
            "i" => NumberFormat::RomanLower,
            "A" => NumberFormat::AlphaUpper,
            "a" => NumberFormat::AlphaLower,
            _ => NumberFormat::Arabic,
        }
    }

    /// Render a count using this token's format and wrapping.
    pub fn render(&self, n: u32) -> String {
        format!("{}{}{}", self.prefix, self.format.render(n), self.suffix)
    }
}

fn to_roman(mut n: u32, upper: bool) -> String {
    const VALS: [(u32, &str); 13] = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"),
        (100, "C"), (90, "XC"), (50, "L"), (40, "XL"),
        (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ];
    if n == 0 {
        return "0".to_string();
    }
    let mut result = String::new();
    for &(val, sym) in &VALS {
        while n >= val {
            result.push_str(sym);
            n -= val;
        }
    }
    if upper { result } else { result.to_lowercase() }
}

fn to_alpha(n: u32, upper: bool) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let base = if upper { b'A' } else { b'a' };
    let idx = ((n - 1) % 26) as u8;
    ((base + idx) as char).to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_roman_values() {
        assert_eq!(to_roman(0, true), "0");
        assert_eq!(to_roman(1, true), "I");
        assert_eq!(to_roman(4, true), "IV");
        assert_eq!(to_roman(9, true), "IX");
        assert_eq!(to_roman(14, true), "XIV");
        assert_eq!(to_roman(42, true), "XLII");
        assert_eq!(to_roman(1999, true), "MCMXCIX");
    }

    #[test]
    fn to_roman_lowercase() {
        assert_eq!(to_roman(4, false), "iv");
        assert_eq!(to_roman(42, false), "xlii");
    }

    #[test]
    fn to_alpha_values() {
        assert_eq!(to_alpha(0, true), "0");
        assert_eq!(to_alpha(1, true), "A");
        assert_eq!(to_alpha(26, true), "Z");
        assert_eq!(to_alpha(27, true), "A"); // wraps
        assert_eq!(to_alpha(1, false), "a");
        assert_eq!(to_alpha(26, false), "z");
        assert_eq!(to_alpha(27, false), "a"); // wraps
    }

    #[test]
    fn format_token_bare_tokens() {
        let t = FormatToken::parse("1");
        assert!(matches!(t.format, NumberFormat::Arabic));
        assert!(t.prefix.is_empty());
        assert!(t.suffix.is_empty());

        let t = FormatToken::parse("I");
        assert!(matches!(t.format, NumberFormat::RomanUpper));

        let t = FormatToken::parse("i");
        assert!(matches!(t.format, NumberFormat::RomanLower));

        let t = FormatToken::parse("A");
        assert!(matches!(t.format, NumberFormat::AlphaUpper));

        let t = FormatToken::parse("a");
        assert!(matches!(t.format, NumberFormat::AlphaLower));
    }

    #[test]
    fn format_token_wrapped() {
        let t = FormatToken::parse("(a)");
        assert_eq!(t.prefix, "(");
        assert_eq!(t.suffix, ")");
        assert!(matches!(t.format, NumberFormat::AlphaLower));
        assert_eq!(t.render(3), "(c)");
    }

    #[test]
    fn format_token_unknown_defaults_arabic() {
        let t = FormatToken::parse("xyz");
        assert!(matches!(t.format, NumberFormat::Arabic));
    }

    #[test]
    fn number_format_render() {
        assert_eq!(NumberFormat::Arabic.render(42), "42");
        assert_eq!(NumberFormat::RomanUpper.render(4), "IV");
        assert_eq!(NumberFormat::RomanLower.render(4), "iv");
        assert_eq!(NumberFormat::AlphaUpper.render(3), "C");
        assert_eq!(NumberFormat::AlphaLower.render(3), "c");
    }

    // ── Node constructor tests ──

    #[test]
    fn node_text_constructor() {
        let n = Node::text("hello");
        assert!(matches!(n.kind, NodeKind::Text(ref s) if s == "hello"));
        assert!(n.attrs.auto_id);
        assert!(n.children.is_empty());
    }

    #[test]
    fn node_code_constructor() {
        let n = Node::code("x + 1");
        assert!(matches!(n.kind, NodeKind::Code(ref s) if s == "x + 1"));
    }

    #[test]
    fn node_code_block_constructor() {
        let n = Node::code_block("rust", "fn main() {}");
        assert!(matches!(n.kind, NodeKind::CodeBlock { ref lang, ref source } if lang == "rust" && source == "fn main() {}"));
    }

    #[test]
    fn node_raw_block_constructor() {
        let n = Node::raw_block("typst", "#rect()");
        assert!(matches!(n.kind, NodeKind::RawBlock { ref format, ref source } if format == "typst" && source == "#rect()"));
    }

    #[test]
    fn node_math_constructors() {
        let inline = Node::math(false, "E=mc^2");
        assert!(matches!(inline.kind, NodeKind::Math { display: false, ref source } if source == "E=mc^2"));
        let display = Node::math(true, "\\sum_i x_i");
        assert!(matches!(display.kind, NodeKind::Math { display: true, .. }));
    }

    #[test]
    fn node_break_constructors() {
        let soft = Node::soft_break();
        assert!(matches!(soft.kind, NodeKind::SoftBreak));
        let hard = Node::hard_break();
        assert!(matches!(hard.kind, NodeKind::HardBreak));
        let hr = Node::thematic_break();
        assert!(matches!(hr.kind, NodeKind::ThematicBreak));
    }

    #[test]
    fn node_section_constructor() {
        let children = vec![Node::text("child")];
        let sec = Node::section(2, children);
        assert!(matches!(sec.kind, NodeKind::Section { level: 2 }));
        assert_eq!(sec.children.len(), 1);
    }

    #[test]
    fn node_heading_constructor() {
        let h = Node::heading(3, vec![Node::text("Title")]);
        assert!(matches!(h.kind, NodeKind::Heading { level: 3 }));
        assert_eq!(h.children.len(), 1);
    }

    #[test]
    fn node_paragraph_constructor() {
        let p = Node::paragraph(vec![Node::text("hello")]);
        assert!(matches!(p.kind, NodeKind::Paragraph));
    }

    #[test]
    fn node_blockquote_constructor() {
        let bq = Node::blockquote(vec![Node::paragraph(vec![Node::text("quoted")])]);
        assert!(matches!(bq.kind, NodeKind::BlockQuote));
        assert_eq!(bq.children.len(), 1);
    }

    #[test]
    fn node_list_constructors() {
        let ul = Node::list(false, None, vec![Node::list_item(vec![Node::text("a")])]);
        assert!(matches!(ul.kind, NodeKind::List { ordered: false, start: None }));

        let ol = Node::list(true, Some(5), vec![Node::list_item(vec![Node::text("b")])]);
        assert!(matches!(ol.kind, NodeKind::List { ordered: true, start: Some(5) }));
    }

    #[test]
    fn node_figure_caption_constructors() {
        let fig = Node::figure(vec![]);
        assert!(matches!(fig.kind, NodeKind::Figure));
        let cap = Node::caption(vec![Node::text("Caption text")]);
        assert!(matches!(cap.kind, NodeKind::Caption));
    }

    #[test]
    fn node_emphasis_strong_strikethrough() {
        let em = Node::emphasis(vec![Node::text("em")]);
        assert!(matches!(em.kind, NodeKind::Emphasis));
        let strong = Node::strong(vec![Node::text("bold")]);
        assert!(matches!(strong.kind, NodeKind::Strong));
        let del = Node::strikethrough(vec![Node::text("del")]);
        assert!(matches!(del.kind, NodeKind::Strikethrough));
    }

    #[test]
    fn node_link_constructor() {
        let a = Node::link("https://example.com", vec![Node::text("click")]);
        assert!(matches!(a.kind, NodeKind::Link { ref url } if url == "https://example.com"));
        assert_eq!(a.children.len(), 1);
    }

    #[test]
    fn node_image_constructor() {
        let img = Node::image("img.png", "Alt text");
        assert!(matches!(img.kind, NodeKind::Image { ref url, ref alt } if url == "img.png" && alt == "Alt text"));
    }

    #[test]
    fn node_citation_constructor() {
        let cite = Node::citation("knuth");
        assert!(matches!(cite.kind, NodeKind::Citation { ref key } if key == "knuth"));
    }

    #[test]
    fn node_cross_ref_constructor() {
        let r = Node::cross_ref("fig:arch");
        assert!(matches!(r.kind, NodeKind::CrossRef { ref target } if target == "fig:arch"));
    }

    #[test]
    fn node_table_constructors() {
        let cell = Node::table_cell(vec![Node::text("data")]);
        assert!(matches!(cell.kind, NodeKind::TableCell));
        let row = Node::table_row(vec![cell]);
        assert!(matches!(row.kind, NodeKind::TableRow));
        let table = Node::table(vec![row]);
        assert!(matches!(table.kind, NodeKind::Table));
    }

    #[test]
    fn node_file_scope_constructor() {
        let fs = Node::file_scope("main.md", vec![Node::text("content")]);
        assert!(matches!(fs.kind, NodeKind::FileScope { ref source } if source == "main.md"));
    }

    // ── Builder methods ──

    #[test]
    fn node_with_id() {
        let n = Node::paragraph(vec![]).with_id("custom-id");
        assert_eq!(n.attrs.id, "custom-id");
    }

    #[test]
    fn node_with_classes() {
        let n = Node::paragraph(vec![]).with_classes(vec!["a".into(), "b".into()]);
        assert_eq!(n.attrs.classes, vec!["a", "b"]);
    }

    #[test]
    fn node_with_children() {
        let n = Node::paragraph(vec![]).with_children(vec![Node::text("x"), Node::text("y")]);
        assert_eq!(n.children.len(), 2);
    }

    // ── Attrs ──

    #[test]
    fn attrs_new_has_auto_id() {
        let a = Attrs::new("test-id".into());
        assert_eq!(a.id, "test-id");
        assert!(a.auto_id);
        assert!(a.classes.is_empty());
        assert!(a.properties.is_empty());
    }

    #[test]
    fn attrs_default_generates_id() {
        reset_id_counter();
        let a = Attrs::default();
        assert!(a.id.starts_with("node-"));
        assert!(a.auto_id);
    }

    // ── Metadata ──

    #[test]
    fn metadata_default() {
        let m = Metadata::default();
        assert!(m.title.is_none());
        assert!(m.date.is_none());
        assert!(m.tags.is_empty());
        assert!(m.summary.is_none());
        assert!(!m.draft);
        assert!(m.slug.is_none());
    }

    // ── Value variants ──

    #[test]
    fn value_text() {
        let v = Value::Text("hello".into());
        assert!(matches!(v, Value::Text(s) if s == "hello"));
    }

    #[test]
    fn value_number() {
        let v = Value::Number(2.5);
        assert!(matches!(v, Value::Number(n) if (n - 2.5).abs() < f64::EPSILON));
    }

    #[test]
    fn value_bool() {
        let v = Value::Bool(true);
        assert!(matches!(v, Value::Bool(true)));
    }

    #[test]
    fn value_length() {
        let v = Value::Length(Length { value: 10.0, unit: LengthUnit::Px });
        assert!(matches!(v, Value::Length(Length { unit: LengthUnit::Px, .. })));
    }

    #[test]
    fn value_spacing() {
        let v = Value::Spacing(Spacing::Auto);
        assert!(matches!(v, Value::Spacing(Spacing::Auto)));
        let v2 = Value::Spacing(Spacing::Flex { grow: 1.0 });
        assert!(matches!(v2, Value::Spacing(Spacing::Flex { grow }) if grow == 1.0));
    }

    #[test]
    fn value_list() {
        let v = Value::List(vec![Value::Number(1.0), Value::Text("x".into())]);
        if let Value::List(items) = v {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn value_map() {
        use std::collections::HashMap;
        let mut m = HashMap::new();
        m.insert("key".to_string(), Value::Bool(true));
        let v = Value::Map(m);
        if let Value::Map(map) = v {
            assert!(matches!(map.get("key"), Some(Value::Bool(true))));
        } else {
            panic!("expected Map");
        }
    }

    // ── SeriesConfig ──

    #[test]
    fn series_config_defaults() {
        let sc = SeriesConfig {
            parent: None,
            formats: vec!["1".to_string()],
            cycle: false,
        };
        assert!(sc.parent.is_none());
        assert_eq!(sc.formats.len(), 1);
        assert!(!sc.cycle);
    }

    // ── CombinatorOp ──

    #[test]
    fn combinator_op_token_len() {
        assert_eq!(CombinatorOp::Child.token_len(), 1);
        assert_eq!(CombinatorOp::Descendant.token_len(), 2);
    }

    // ── ID generation ──

    #[test]
    fn reset_id_counter_works() {
        reset_id_counter();
        let a = Attrs::default();
        assert!(a.id.contains("-0"), "first ID after reset should be -0: {}", a.id);
        let b = Attrs::default();
        assert!(b.id.contains("-1"), "second ID should be -1: {}", b.id);
        reset_id_counter();
        let c = Attrs::default();
        assert!(c.id.contains("-0"), "after re-reset should be -0 again: {}", c.id);
    }

    // ── Roman numeral edge cases ──

    #[test]
    fn to_roman_large() {
        assert_eq!(to_roman(3999, true), "MMMCMXCIX");
    }

    #[test]
    fn to_alpha_edge_cases() {
        assert_eq!(to_alpha(26, true), "Z");
        assert_eq!(to_alpha(26, false), "z");
        // Wraps at 27
        assert_eq!(to_alpha(52, true), "Z");
        assert_eq!(to_alpha(53, true), "A");
    }

    // ── FormatToken additional tests ──

    #[test]
    fn format_token_render_all_formats() {
        assert_eq!(FormatToken::parse("1").render(10), "10");
        assert_eq!(FormatToken::parse("I").render(10), "X");
        assert_eq!(FormatToken::parse("i").render(10), "x");
        assert_eq!(FormatToken::parse("A").render(10), "J");
        assert_eq!(FormatToken::parse("a").render(10), "j");
    }

    #[test]
    fn format_token_whitespace_trimmed() {
        let t = FormatToken::parse("  1  ");
        assert!(matches!(t.format, NumberFormat::Arabic));
    }
}
