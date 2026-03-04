use crate::ast::{Attrs, Document, Node, NodeKind, Value};

use syntect::highlighting::ThemeSet;
use syntect::html::highlighted_html_for_string;
use syntect::parsing::SyntaxSet;

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

    fn wrap(&self, node: &Node, out: &mut String) {
        self.open(out);
        render_children(node, out);
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
///
/// Raw blocks and preview blocks should be pre-compiled via the `CompileBlocks`
/// pass before calling this function. Without pre-compilation, raw blocks fall
/// back to escaped `<pre>` output.
pub fn render_html(doc: &Document) -> String {
    let mut out = String::new();
    render_node(&doc.content, &mut out);
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

fn render_node(node: &Node, out: &mut String) {
    if let Some(name) = wrap_tag_name(&node.kind) {
        Tag::new(name).ast_attrs(&node.attrs).wrap(node, out);
        return;
    }
    match &node.kind {
        NodeKind::Section { level } => {
            if *level > 0 {
                Tag::new("section").ast_attrs(&node.attrs).open(out);
            }
            render_children(node, out);
            if *level > 0 {
                close_tag("section", out);
            }
        }
        NodeKind::Heading { level } => {
            let tag = format!("h{}", level.min(&6));
            Tag::new(&tag).ast_attrs(&node.attrs).open(out);
            render_children(node, out);
            close_tag(&tag, out);
        }
        NodeKind::BlockQuote => {
            Tag::new("blockquote")
                .ast_attrs(&node.attrs)
                .wrap(node, out);
        }
        NodeKind::List { ordered, start } => render_list(node, *ordered, *start, out),
        NodeKind::Table => render_table(node, out),
        NodeKind::TableRow => Tag::new("tr").wrap(node, out),
        NodeKind::TableCell => Tag::new("td").wrap(node, out),
        NodeKind::ThematicBreak => out.push_str("<hr>"),
        NodeKind::Caption => Tag::new("figcaption").wrap(node, out),
        NodeKind::CodeBlock { lang, source } => {
            if node.attrs.properties.contains_key("preview") {
                render_preview_block(lang, source, &node.attrs, out);
            } else {
                render_code_block(lang, source, &node.attrs, out);
            }
        }
        NodeKind::RawBlock { format, source } => {
            if node.attrs.properties.contains_key("preview") {
                render_preview_block(format, source, &node.attrs, out);
            } else {
                render_raw_block(format, source, &node.attrs, out);
            }
        }
        NodeKind::Math { display, source } => render_math(*display, source, &node.attrs, out),
        NodeKind::Text(s) => out.push_str(&escape_html(s)),
        NodeKind::Code(s) => Tag::new("code").wrap_text(s, out),
        NodeKind::Emphasis => Tag::new("em").wrap(node, out),
        NodeKind::Strong => Tag::new("strong").wrap(node, out),
        NodeKind::Strikethrough => Tag::new("del").wrap(node, out),
        NodeKind::Link { url } => {
            Tag::new("a").attr("href", url).open(out);
            render_children(node, out);
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
            render_children(node, out);
        }
        NodeKind::Space { .. } | NodeKind::Paragraph | NodeKind::ListItem | NodeKind::Figure => {
            unreachable!("handled by wrap_tag_name")
        }
    }
}

fn render_children(node: &Node, out: &mut String) {
    for child in &node.children {
        render_node(child, out);
    }
}

fn render_list(node: &Node, ordered: bool, start: Option<u64>, out: &mut String) {
    let mut tag = Tag::new(if ordered { "ol" } else { "ul" });
    tag = tag.ast_attrs(&node.attrs);
    if let Some(s) = start
        && s != 1
    {
        tag = tag.attr("start", &s.to_string());
    }
    tag.wrap(node, out);
}

fn render_table(node: &Node, out: &mut String) {
    Tag::new("table").ast_attrs(&node.attrs).open(out);
    let mut rows = node.children.iter();
    if let Some(header_row) = rows.next() {
        out.push_str("<thead><tr>");
        render_table_cells(header_row, "th", out);
        out.push_str("</tr></thead>");
    }
    let body_rows: Vec<_> = rows.collect();
    if !body_rows.is_empty() {
        out.push_str("<tbody>");
        for row in body_rows {
            out.push_str("<tr>");
            render_table_cells(row, "td", out);
            out.push_str("</tr>");
        }
        out.push_str("</tbody>");
    }
    close_tag("table", out);
}

fn render_table_cells(row: &Node, tag: &str, out: &mut String) {
    for cell in &row.children {
        Tag::new(tag).open(out);
        render_children(cell, out);
        close_tag(tag, out);
    }
}

fn render_code_block(lang: &str, source: &str, attrs: &Attrs, out: &mut String) {
    if !lang.is_empty()
        && let Some(highlighted) = highlight_code(lang, source)
    {
        Tag::new("pre").ast_attrs(attrs).open(out);
        let code = Tag::new("code").class(&format!("language-{lang}"));
        code.open(out);
        out.push_str(&highlighted);
        out.push_str("</code></pre>");
        return;
    }
    Tag::new("pre").ast_attrs(attrs).open(out);
    let mut code = Tag::new("code");
    if !lang.is_empty() {
        code = code.class(&format!("language-{lang}"));
    }
    code.open(out);
    out.push_str(&escape_html(source));
    out.push_str("</code></pre>");
}

fn render_preview_block(lang: &str, source: &str, attrs: &Attrs, out: &mut String) {
    // Read pre-compiled output from the CompileBlocks pass.
    // If not compiled (no compiler was available), fall back to a code block.
    let preview_html = match attrs.properties.get("compiled") {
        Some(Value::Text(html)) => html.as_str(),
        _ => {
            render_code_block(lang, source, attrs, out);
            return;
        }
    };

    let id = &attrs.id;

    // Open wrapper.
    let mut wrapper = Tag::new("div").class("preview-block");
    if !attrs.auto_id {
        wrapper = wrapper.attr("id", id);
    }
    wrapper.open(out);

    // Radio inputs for CSS-only tab switching (preview checked by default).
    out.push_str(&format!(
        "<input type=\"radio\" name=\"{id}\" id=\"{id}-preview\" class=\"preview-tab-input\" checked>"
    ));
    out.push_str(&format!(
        "<input type=\"radio\" name=\"{id}\" id=\"{id}-source\" class=\"preview-tab-input\">"
    ));

    // Tab labels.
    out.push_str(&format!(
        "<div class=\"preview-tabs\">\
         <label for=\"{id}-preview\">Preview</label>\
         <label for=\"{id}-source\">Source</label>\
         </div>"
    ));

    // Preview panel.
    out.push_str("<div class=\"preview-panel preview-output\">");
    out.push_str(preview_html);
    out.push_str("</div>");

    // Source panel (reuse code block rendering without outer attrs).
    out.push_str("<div class=\"preview-panel preview-source\">");
    render_code_block_inner(lang, source, out);
    out.push_str("</div>");

    close_tag("div", out);
}

/// Render just the `<pre><code>` part of a code block, without node attributes.
fn render_code_block_inner(lang: &str, source: &str, out: &mut String) {
    if !lang.is_empty()
        && let Some(highlighted) = highlight_code(lang, source)
    {
        Tag::new("pre").open(out);
        Tag::new("code")
            .class(&format!("language-{lang}"))
            .open(out);
        out.push_str(&highlighted);
        out.push_str("</code></pre>");
        return;
    }
    Tag::new("pre").open(out);
    let mut code = Tag::new("code");
    if !lang.is_empty() {
        code = code.class(&format!("language-{lang}"));
    }
    code.open(out);
    out.push_str(&escape_html(source));
    out.push_str("</code></pre>");
}

fn highlight_code(lang: &str, source: &str) -> Option<String> {
    let ss = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();
    let syntax = ss.find_syntax_by_token(lang)?;
    let theme = &ts.themes["base16-ocean.dark"];
    let full = highlighted_html_for_string(source, &ss, syntax, theme).ok()?;
    // Strip syntect's outer <pre style="..."> wrapper — we provide our own <pre>
    let inner = full
        .strip_prefix("<pre style=\"")
        .and_then(|s| s.find("\">").map(|i| &s[i + 2..]))
        .and_then(|s| {
            s.strip_suffix("</pre>\n")
                .or_else(|| s.strip_suffix("</pre>"))
        })
        .unwrap_or(&full);
    Some(inner.trim_start_matches('\n').to_string())
}

fn render_raw_block(format: &str, source: &str, attrs: &Attrs, out: &mut String) {
    // Check for compilation error from CompileBlocks pass.
    if let Some(Value::Text(e)) = attrs.properties.get("compile_error") {
        Tag::new("pre").class("raw-error").open(out);
        out.push_str(&escape_html(e));
        close_tag("pre", out);
        return;
    }

    // Check for compiled output from CompileBlocks pass.
    if let Some(Value::Text(html)) = attrs.properties.get("compiled") {
        let has_user_attrs = !attrs.classes.is_empty()
            || (!attrs.id.is_empty() && !attrs.auto_id)
            || attrs
                .properties
                .keys()
                .any(|k| k != "compiled" && k != "compile_error" && k != "preview");
        if has_user_attrs {
            Tag::new("div").ast_attrs(attrs).open(out);
            out.push_str(html);
            close_tag("div", out);
        } else {
            out.push_str(html);
        }
        return;
    }

    // No compilation result — fallback (no compiler was available).
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
        render_html(&doc)
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
        // Syntax-highlighted: tokens are wrapped in styled spans
        assert!(html.contains("fn"));
        assert!(html.contains("main"));
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

    use crate::ast::RawBlockCompiler;
    use crate::visit::Pass;

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
        let mut doc = parse(md).unwrap();
        crate::resolve::resolve_document(&mut doc);
        let comp = TestCompiler;
        crate::resolve::CompileBlocks::new(&comp).run(&mut doc);
        render_html(&doc)
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
    fn raw_block_with_attrs_wraps_in_div() {
        let html = render_with_compiler("```{=typst .center}\n#rect()\n```");
        assert!(
            html.contains("<div class=\"center\">"),
            "compiled raw block with attrs should be wrapped in <div>: {html}"
        );
        assert!(html.contains("</div>"));
    }

    #[test]
    fn raw_block_without_attrs_no_wrapper() {
        let html = render_with_compiler("```{=typst}\n#rect()\n```");
        // The compiler output is <div class="typst">…</div>; there should be
        // no extra wrapper <div> around it.
        assert!(
            !html.contains("<div><div") && !html.contains("<div class=\"center\""),
            "compiled raw block without attrs should not have wrapper <div>: {html}"
        );
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
        // Content is rendered (either escaped or in styled spans)
        assert!(html.contains("div"));
        assert!(html.contains("hello"));
    }

    // -- preview block tests --

    #[test]
    fn preview_block_with_property() {
        let html = render_with_compiler("```html {preview}\n<strong>hello</strong>\n```");
        assert!(html.contains("class=\"preview-block\""));
        assert!(html.contains("class=\"preview-tabs\""));
        // Preview panel contains compiled output (html passthrough)
        assert!(html.contains("<strong>hello</strong>"));
        // Source panel contains syntax-highlighted code
        assert!(html.contains("class=\"preview-panel preview-source\""));
        assert!(html.contains("language-html"));
    }

    #[test]
    fn preview_block_with_format_override() {
        let html = render_with_compiler("```typst {preview=\"typst\"}\n#rect()\n```");
        assert!(html.contains("class=\"preview-block\""));
        // Compiled via typst format
        assert!(html.contains("<div class=\"typst\">#rect()</div>"));
    }

    #[test]
    fn preview_block_without_compiler_falls_back() {
        // No compiler available — fall back to normal code block.
        let html = render("```html {preview}\n<strong>hello</strong>\n```");
        assert!(!html.contains("preview-block"));
        assert!(html.contains("<pre"));
        assert!(html.contains("language-html"));
    }

    #[test]
    fn preview_unsupported_format_falls_back() {
        // When the compiler doesn't support the format, fall back to normal code block.
        let html = render_with_compiler("```mermaid {preview}\ngraph TD\n```");
        assert!(!html.contains("preview-block"));
        assert!(html.contains("<pre"));
    }

    #[test]
    fn preview_block_via_cascade() {
        let html =
            render_with_compiler("{@hint(code-block, preview=true)}\n\n```html\n<em>hi</em>\n```");
        assert!(html.contains("class=\"preview-block\""));
        assert!(html.contains("<em>hi</em>"));
    }

    #[test]
    fn no_preview_without_attr() {
        let html = render_with_compiler("```html\n<div>hello</div>\n```");
        assert!(!html.contains("preview-block"));
        assert!(html.contains("<pre"));
        assert!(html.contains("language-html"));
    }

    #[test]
    fn raw_block_preview_via_property() {
        let html = render_with_compiler("```{=html preview}\n<strong>hi</strong>\n```");
        assert!(html.contains("preview-block"));
        assert!(html.contains("<strong>hi</strong>"));
    }

    #[test]
    fn raw_block_preview_via_cascade() {
        let html = render_with_compiler(
            "{@hint(raw-block, preview=true)}\n\n```{=html}\n<em>hi</em>\n```",
        );
        assert!(html.contains("preview-block"));
        assert!(html.contains("<em>hi</em>"));
    }

    #[test]
    fn raw_block_no_preview_without_attr() {
        let html = render_with_compiler("```{=html}\n<div>hello</div>\n```");
        assert!(!html.contains("preview-block"));
        assert!(html.contains("<div>hello</div>"));
    }
}
