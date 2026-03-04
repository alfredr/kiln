use crate::ast::{Attrs, Document, Node, NodeKind, RawBlockCompiler};

// ── Tag builder ──

struct Tag<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, String)>,
}

impl<'a> Tag<'a> {
    fn new(name: &'a str) -> Self {
        Tag {
            name,
            attrs: Vec::new(),
        }
    }

    fn attr(mut self, key: &'a str, value: &str) -> Self {
        self.attrs.push((key, escape_attr(value)));
        self
    }

    fn class(mut self, class: &str) -> Self {
        self.attrs.push(("class", escape_attr(class)));
        self
    }

    fn ast_attrs(mut self, ast: &Attrs) -> Self {
        if !ast.id.is_empty() && !ast.auto_id {
            self.attrs.push(("id", escape_attr(&ast.id)));
        }
        if !ast.classes.is_empty() {
            let classes: Vec<_> = ast.classes.iter().map(|c| escape_attr(c)).collect();
            self.attrs.push(("class", classes.join(" ")));
        }
        self
    }

    fn open(&self, out: &mut String) {
        out.push('<');
        out.push_str(self.name);
        for (key, val) in &self.attrs {
            out.push(' ');
            out.push_str(key);
            out.push_str("=\"");
            out.push_str(val);
            out.push('"');
        }
        out.push('>');
    }

    fn wrap_text(&self, text: &str, out: &mut String) {
        self.open(out);
        out.push_str(&escape_html(text));
        close_tag(self.name, out);
    }

    fn wrap(&self, node: &Node, compiler: Option<&dyn RawBlockCompiler>, out: &mut String) {
        self.open(out);
        render_children(node, compiler, out);
        close_tag(self.name, out);
    }
}

fn close_tag(name: &str, out: &mut String) {
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

// ── Renderer ──

/// Render a Document AST to an HTML string.
pub fn render_html(doc: &Document, compiler: Option<&dyn RawBlockCompiler>) -> String {
    let mut out = String::new();
    render_node(&doc.content, compiler, &mut out);
    out
}

fn wrap_tag_name(kind: &NodeKind) -> Option<&'static str> {
    match kind {
        NodeKind::Paragraph => Some("p"),
        NodeKind::ListItem => Some("li"),
        NodeKind::Figure => Some("figure"),
        _ => None,
    }
}

fn render_node(node: &Node, compiler: Option<&dyn RawBlockCompiler>, out: &mut String) {
    if let Some(name) = wrap_tag_name(&node.kind) {
        Tag::new(name).ast_attrs(&node.attrs).wrap(node, compiler, out);
        return;
    }
    match &node.kind {
        NodeKind::Section { level } => {
            if *level > 0 {
                Tag::new("section").ast_attrs(&node.attrs).open(out);
            }
            render_children(node, compiler, out);
            if *level > 0 {
                close_tag("section", out);
            }
        }
        NodeKind::Heading { level } => {
            let tag = format!("h{}", level.min(&6));
            Tag::new(&tag).ast_attrs(&node.attrs).open(out);
            render_children(node, compiler, out);
            close_tag(&tag, out);
        }
        NodeKind::BlockQuote => {
            Tag::new("blockquote")
                .ast_attrs(&node.attrs)
                .wrap(node, compiler, out);
        }
        NodeKind::List { ordered, start } => render_list(node, *ordered, *start, compiler, out),
        NodeKind::Table => render_table(node, compiler, out),
        NodeKind::TableRow => Tag::new("tr").wrap(node, compiler, out),
        NodeKind::TableCell => Tag::new("td").wrap(node, compiler, out),
        NodeKind::ThematicBreak => out.push_str("<hr>"),
        NodeKind::Caption => Tag::new("figcaption").wrap(node, compiler, out),
        NodeKind::CodeBlock { lang, source } => render_code_block(lang, source, &node.attrs, out),
        NodeKind::RawBlock { format, source } => {
            render_raw_block(format, source, &node.attrs, compiler, out);
        }
        NodeKind::Math { display, source } => render_math(*display, source, &node.attrs, out),
        NodeKind::Text(s) => out.push_str(&escape_html(s)),
        NodeKind::Code(s) => Tag::new("code").wrap_text(s, out),
        NodeKind::Emphasis => Tag::new("em").wrap(node, compiler, out),
        NodeKind::Strong => Tag::new("strong").wrap(node, compiler, out),
        NodeKind::Strikethrough => Tag::new("del").wrap(node, compiler, out),
        NodeKind::Link { url } => {
            Tag::new("a").attr("href", url).open(out);
            render_children(node, compiler, out);
            close_tag("a", out);
        }
        NodeKind::Image { url, alt } => {
            Tag::new("img")
                .attr("src", url)
                .attr("alt", alt)
                .ast_attrs(&node.attrs)
                .open(out);
        }
        NodeKind::Citation { key } => {
            Tag::new("cite").attr("data-key", key).open(out);
            out.push('[');
            out.push_str(&escape_html(key));
            out.push(']');
            close_tag("cite", out);
        }
        NodeKind::CrossRef { target } => {
            Tag::new("a")
                .attr("href", &format!("#{target}"))
                .wrap_text(target, out);
        }
        NodeKind::SoftBreak => out.push('\n'),
        NodeKind::HardBreak => out.push_str("<br>"),
        NodeKind::FileScope { .. } | NodeKind::Custom { .. } => {
            render_children(node, compiler, out);
        }
        NodeKind::Space { .. }
        | NodeKind::Paragraph
        | NodeKind::ListItem
        | NodeKind::Figure => unreachable!("handled by wrap_tag_name"),
    }
}

fn render_children(node: &Node, compiler: Option<&dyn RawBlockCompiler>, out: &mut String) {
    for child in &node.children {
        render_node(child, compiler, out);
    }
}

fn render_list(
    node: &Node,
    ordered: bool,
    start: Option<u64>,
    compiler: Option<&dyn RawBlockCompiler>,
    out: &mut String,
) {
    let mut tag = Tag::new(if ordered { "ol" } else { "ul" });
    tag = tag.ast_attrs(&node.attrs);
    if let Some(s) = start
        && s != 1
    {
        tag = tag.attr("start", &s.to_string());
    }
    tag.wrap(node, compiler, out);
}

fn render_table(node: &Node, compiler: Option<&dyn RawBlockCompiler>, out: &mut String) {
    Tag::new("table").ast_attrs(&node.attrs).open(out);
    let mut rows = node.children.iter();
    if let Some(header_row) = rows.next() {
        out.push_str("<thead><tr>");
        render_table_cells(header_row, "th", compiler, out);
        out.push_str("</tr></thead>");
    }
    let body_rows: Vec<_> = rows.collect();
    if !body_rows.is_empty() {
        out.push_str("<tbody>");
        for row in body_rows {
            out.push_str("<tr>");
            render_table_cells(row, "td", compiler, out);
            out.push_str("</tr>");
        }
        out.push_str("</tbody>");
    }
    close_tag("table", out);
}

fn render_table_cells(
    row: &Node,
    tag: &str,
    compiler: Option<&dyn RawBlockCompiler>,
    out: &mut String,
) {
    for cell in &row.children {
        Tag::new(tag).open(out);
        render_children(cell, compiler, out);
        close_tag(tag, out);
    }
}

fn render_code_block(lang: &str, source: &str, attrs: &Attrs, out: &mut String) {
    Tag::new("pre").ast_attrs(attrs).open(out);
    let mut code = Tag::new("code");
    if !lang.is_empty() {
        code = code.class(&format!("language-{lang}"));
    }
    code.open(out);
    out.push_str(&escape_html(source));
    out.push_str("</code></pre>");
}

fn render_raw_block(
    format: &str,
    source: &str,
    attrs: &Attrs,
    compiler: Option<&dyn RawBlockCompiler>,
    out: &mut String,
) {
    if let Some(comp) = compiler {
        match comp.compile(format, source) {
            Ok(html) => out.push_str(&html),
            Err(e) => {
                Tag::new("pre").class("raw-error").open(out);
                out.push_str(&escape_html(&e));
                close_tag("pre", out);
            }
        }
        return;
    }
    Tag::new("pre")
        .ast_attrs(attrs)
        .attr("data-format", format)
        .open(out);
    out.push_str(&escape_html(source));
    close_tag("pre", out);
}

fn render_math(display: bool, source: &str, attrs: &Attrs, out: &mut String) {
    if display {
        let mut tag = Tag::new("div").class("math display");
        if !attrs.id.is_empty() && !attrs.auto_id {
            tag = tag.attr("id", &attrs.id);
        }
        tag.open(out);
        out.push_str(&escape_html(source));
        close_tag("div", out);
    } else {
        Tag::new("span").class("math").open(out);
        out.push_str(&escape_html(source));
        close_tag("span", out);
    }
}

// ── Escaping ──

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::reset_id_counter;
    use crate::parse::parse;

    fn render(md: &str) -> String {
        reset_id_counter();
        let doc = parse(md).unwrap();
        render_html(&doc, None)
    }

    #[test]
    fn simple_paragraph() {
        let html = render("Hello world.");
        assert!(html.contains("<p"));
        assert!(html.contains("Hello world."));
        assert!(html.contains("</p>"));
    }

    #[test]
    fn heading_renders() {
        let html = render("# Title");
        assert!(html.contains("<h1"));
        assert!(html.contains("Title"));
        assert!(html.contains("</h1>"));
    }

    #[test]
    fn emphasis_and_strong() {
        let html = render("*em* and **strong**");
        assert!(html.contains("<em>em</em>"));
        assert!(html.contains("<strong>strong</strong>"));
    }

    #[test]
    fn inline_code() {
        let html = render("Use `println!` here.");
        assert!(html.contains("<code>println!</code>"));
    }

    #[test]
    fn code_block_renders() {
        let html = render("```rust\nfn main() {}\n```");
        assert!(html.contains("<pre"));
        assert!(html.contains("class=\"language-rust\""));
        assert!(html.contains("fn main() {}"));
    }

    #[test]
    fn link_renders() {
        let html = render("[click](https://example.com)");
        assert!(html.contains("<a href=\"https://example.com\">"));
        assert!(html.contains("click"));
        assert!(html.contains("</a>"));
    }

    #[test]
    fn image_figure() {
        let html = render("![Alt](img.png)");
        assert!(html.contains("<figure"));
        assert!(html.contains("<img src=\"img.png\""));
        assert!(html.contains("<figcaption>"));
    }

    #[test]
    fn math_inline() {
        let html = render("Equation $E=mc^2$ here.");
        assert!(html.contains("<span class=\"math\">E=mc^2</span>"));
    }

    #[test]
    fn math_display() {
        let html = render("$$\nE=mc^2\n$$");
        assert!(html.contains("<div class=\"math display\""));
        assert!(html.contains("E=mc^2"));
    }

    #[test]
    fn citation_renders() {
        let html = render("See [@knuth].");
        assert!(html.contains("<cite data-key=\"knuth\">[knuth]</cite>"));
    }

    #[test]
    fn cross_ref_renders() {
        let html = render("See @fig:arch.");
        assert!(html.contains("<a href=\"#fig:arch\">fig:arch</a>"));
    }

    #[test]
    fn unordered_list_renders() {
        let html = render("- one\n- two");
        assert!(html.contains("<ul"));
        assert!(html.contains("<li"));
        assert!(html.contains("one"));
    }

    #[test]
    fn ordered_list_renders() {
        let html = render("1. first\n2. second");
        assert!(html.contains("<ol"));
        assert!(html.contains("<li"));
    }

    #[test]
    fn blockquote_renders() {
        let html = render("> quoted text");
        assert!(html.contains("<blockquote"));
        assert!(html.contains("quoted text"));
    }

    #[test]
    fn thematic_break_renders() {
        let html = render("above\n\n---\n\nbelow");
        assert!(html.contains("<hr>"));
    }

    #[test]
    fn html_escaping() {
        let html = render("Use `<div>` & things.");
        assert!(html.contains("&lt;div&gt;"));
        assert!(html.contains("&amp;"));
    }

    #[test]
    fn raw_block_without_compiler() {
        let html = render("```{=typst}\n#rect()\n```");
        assert!(html.contains("data-format=\"typst\""));
        assert!(html.contains("#rect()"));
    }

    #[test]
    fn section_wrapping() {
        let html = render("# Section One\n\nContent here.");
        assert!(html.contains("<section"));
        assert!(html.contains("</section>"));
    }

    #[test]
    fn strikethrough_renders() {
        let html = render("~~deleted~~");
        assert!(html.contains("<del>deleted</del>"));
    }

    // -- raw block tests with compiler --

    struct TestCompiler;

    impl RawBlockCompiler for TestCompiler {
        fn compile(&self, format: &str, source: &str) -> Result<String, String> {
            match format {
                "html" => Ok(source.to_string()),
                "typst" => Ok(format!("<div class=\"typst\">{source}</div>")),
                _ => Err(format!("unsupported: {format}")),
            }
        }
    }

    fn render_with_compiler(md: &str) -> String {
        reset_id_counter();
        let doc = parse(md).unwrap();
        let comp = TestCompiler;
        render_html(&doc, Some(&comp))
    }

    #[test]
    fn raw_block_html_passthrough() {
        let html = render_with_compiler("```{=html}\n<div class=\"widget\">hello</div>\n```");
        assert!(html.contains("<div class=\"widget\">hello</div>"));
        // Should NOT be escaped or wrapped in <pre>
        assert!(!html.contains("&lt;div"));
        assert!(!html.contains("data-format"));
    }

    #[test]
    fn raw_block_html_without_compiler() {
        let html = render("```{=html}\n<div>hello</div>\n```");
        // Falls back to escaped <pre> with data-format
        assert!(html.contains("data-format=\"html\""));
        assert!(html.contains("&lt;div&gt;hello&lt;/div&gt;"));
    }

    #[test]
    fn raw_block_compiler_error() {
        let html = render_with_compiler("```{=mermaid}\ngraph TD\n```");
        assert!(html.contains("<pre class=\"raw-error\">"));
        assert!(html.contains("unsupported: mermaid"));
    }

    #[test]
    fn auto_ids_not_in_output() {
        let html = render("Hello world.\n\nAnother paragraph.");
        assert!(
            !html.contains(" id=\""),
            "auto-generated IDs should not appear in HTML output"
        );
    }

    #[test]
    fn explicit_id_in_output() {
        let html = render("# Title {#my-title}");
        assert!(html.contains("id=\"my-title\""));
    }

    #[test]
    fn html_code_block_renders_as_code() {
        // ```html (without {=html}) should render as a normal code block
        let html = render("```html\n<div>hello</div>\n```");
        assert!(html.contains("<pre"));
        assert!(html.contains("language-html"));
        assert!(html.contains("&lt;div&gt;"));
    }
}
