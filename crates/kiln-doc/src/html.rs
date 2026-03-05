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
            let default_href = format!("#{target}");
            let href = node
                .attrs
                .properties
                .get("href")
                .and_then(|v| match v {
                    Value::Text(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or(&default_href);
            let text = node
                .attrs
                .properties
                .get("text")
                .and_then(|v| match v {
                    Value::Text(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or(target);
            Tag::new("a").attr("href", href).wrap_text(text, out);
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

    // ── Table rendering ──

    #[test]
    fn table_thead_tbody() {
        let html = render("| H1 | H2 |\n|---|---|\n| a | b |\n| c | d |");
        assert!(html.contains("<thead>"), "table should have thead");
        assert!(html.contains("<tbody>"), "table should have tbody");
        assert!(html.contains("<th>"), "header cells should be th");
        assert!(html.contains("<td>"), "body cells should be td");
        assert!(html.contains("</table>"));
    }

    #[test]
    fn table_header_only() {
        // Table with just a header and separator, no body rows
        let html = render("| H1 | H2 |\n|---|---|");
        assert!(html.contains("<thead>"));
        assert!(html.contains("<th>"));
        assert!(!html.contains("<tbody>"), "no body rows means no tbody");
    }

    #[test]
    fn table_alignment() {
        let html = render("| L | C | R |\n|:---|:---:|---:|\n| a | b | c |");
        assert!(html.contains("<table"));
        assert!(html.contains("<thead>"));
        assert!(html.contains("<tbody>"));
    }

    // ── Ordered list start ──

    #[test]
    fn ordered_list_custom_start() {
        let html = render("5. fifth\n6. sixth");
        assert!(
            html.contains("start=\"5\""),
            "ordered list with start=5 should have start attr: {html}"
        );
    }

    #[test]
    fn ordered_list_start_one_no_attr() {
        let html = render("1. first\n2. second");
        assert!(
            !html.contains("start="),
            "ordered list starting at 1 should not have start attr: {html}"
        );
    }

    // ── Math with ID ──

    #[test]
    fn math_display_with_id() {
        let html = render("$$ {#my-eq}\nE=mc^2\n$$");
        assert!(html.contains("class=\"math display\""));
        // Display math should contain the math content
        assert!(html.contains("E=mc^2") || html.contains("E&#x3D;mc^2"));
    }

    // ── Figure and caption ──

    #[test]
    fn figure_with_caption_text() {
        let html = render("![My caption](img.png)");
        assert!(html.contains("<figure"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("My caption"));
        assert!(html.contains("<img"));
        assert!(html.contains("</figure>"));
    }

    #[test]
    fn figure_with_id_and_class() {
        let html = render("![Alt](img.png){#fig:test .wide}");
        assert!(html.contains("id=\"fig:test\""));
        assert!(html.contains("class=\"wide\""));
    }

    #[test]
    fn image_no_alt_no_caption() {
        let html = render("![](img.png)");
        assert!(html.contains("<figure"));
        assert!(html.contains("<img"));
        // No alt text = no figcaption
        assert!(!html.contains("<figcaption>"), "empty alt should not produce figcaption: {html}");
    }

    // ── Code block variations ──

    #[test]
    fn code_block_no_lang() {
        let html = render("```\nplain code\n```");
        assert!(html.contains("<pre>"));
        assert!(html.contains("<code>"));
        assert!(!html.contains("language-"), "no lang should mean no language- class: {html}");
    }

    #[test]
    fn code_block_unknown_lang() {
        let html = render("```zzzz_nonexistent\nsome code\n```");
        assert!(html.contains("<pre>"));
        assert!(html.contains("language-zzzz_nonexistent"));
        // Falls back to escaped (no highlighting), should still contain code text
        assert!(html.contains("some code"));
    }

    // ── Cross-ref rendering ──

    #[test]
    fn cross_ref_with_resolved_text() {
        reset_id_counter();
        let mut doc = parse("{@var(fig, 0, format=\"1\")}\n{@init(figure, series=\"fig\")}\n{@on([series=\"fig\"], @inc(fig), @store(number, fig))}\n{@hint(figure, label=\"Figure {n}\")}\n\n![Pic](a.png){#fig:pic}\n\nSee @fig:pic.").unwrap();
        crate::resolve::resolve_document(&mut doc);
        let html = render_html(&doc);
        assert!(html.contains("Figure 1"), "resolved cross-ref should show label: {html}");
        assert!(html.contains("href=\"#fig:pic\""));
    }

    // ── Other rendering ──

    #[test]
    fn hard_break_renders() {
        let html = render("line one  \nline two");
        assert!(html.contains("<br>"));
    }

    #[test]
    fn emphasis_wraps_content() {
        let html = render("*emphasized text*");
        assert!(html.contains("<em>"));
        assert!(html.contains("emphasized text"));
    }

    #[test]
    fn multiple_paragraphs() {
        let html = render("First para.\n\nSecond para.");
        // Count <p> occurrences
        let p_count = html.matches("<p>").count();
        assert!(p_count >= 2, "should have at least 2 paragraphs: {html}");
    }

    #[test]
    fn section_nesting() {
        let html = render("# Top\n\n## Sub\n\nContent.");
        assert!(html.contains("<section"));
        assert!(html.contains("<h1"));
        assert!(html.contains("<h2"));
    }

    #[test]
    fn tilde_fence_renders() {
        let html = render("~~~python\nprint('hello')\n~~~");
        assert!(html.contains("<pre"));
        assert!(html.contains("language-python"));
    }

    #[test]
    fn blockquote_with_nested_content() {
        let html = render("> Line one\n> Line two");
        assert!(html.contains("<blockquote"));
        assert!(html.contains("</blockquote>"));
    }

    #[test]
    fn unordered_list_structure() {
        let html = render("- item a\n- item b\n- item c");
        assert!(html.contains("<ul"));
        let li_count = html.matches("<li").count();
        assert_eq!(li_count, 3);
    }

    // ── Direct AST rendering tests ──

    #[test]
    fn render_cross_ref_with_properties() {
        use crate::ast::{Node, Value};
        reset_id_counter();
        let mut r = Node::cross_ref("fig:test");
        r.attrs.properties.insert("text".to_string(), Value::Text("Figure 1".to_string()));
        r.attrs.properties.insert("href".to_string(), Value::Text("#fig:test".to_string()));
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![Node::paragraph(vec![r])]),
        };
        let html = render_html(&doc);
        assert!(html.contains("Figure 1"), "should show resolved text: {html}");
        assert!(html.contains("href=\"#fig:test\""));
    }

    #[test]
    fn render_math_display_with_explicit_id() {
        use crate::ast::Node;
        reset_id_counter();
        let mut math = Node::math(true, "x^2");
        math.attrs.id = "eq:one".to_string();
        math.attrs.auto_id = false;
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![math]),
        };
        let html = render_html(&doc);
        assert!(html.contains("id=\"eq:one\""), "explicit ID should appear: {html}");
        assert!(html.contains("math display"));
    }

    #[test]
    fn render_code_block_no_lang() {
        use crate::ast::Node;
        reset_id_counter();
        let code = Node::code_block("", "plain code");
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![code]),
        };
        let html = render_html(&doc);
        assert!(html.contains("<pre>"));
        assert!(html.contains("<code>"));
        assert!(!html.contains("language-"), "empty lang should not have language class: {html}");
    }

    #[test]
    fn render_soft_break() {
        use crate::ast::Node;
        reset_id_counter();
        let para = Node::paragraph(vec![
            Node::text("line1"),
            Node::soft_break(),
            Node::text("line2"),
        ]);
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![para]),
        };
        let html = render_html(&doc);
        assert!(html.contains("line1\nline2"), "soft break should be newline: {html}");
    }

    #[test]
    fn render_file_scope_transparent() {
        use crate::ast::Node;
        reset_id_counter();
        let fs = Node::file_scope("test.md", vec![Node::paragraph(vec![Node::text("content")])]);
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![fs]),
        };
        let html = render_html(&doc);
        assert!(html.contains("content"));
    }

    #[test]
    fn render_raw_block_compiled_no_attrs() {
        use crate::ast::{Node, Value};
        reset_id_counter();
        let mut raw = Node::raw_block("typst", "#rect()");
        raw.attrs.properties.insert("compiled".to_string(), Value::Text("<div>compiled</div>".to_string()));
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![raw]),
        };
        let html = render_html(&doc);
        assert!(html.contains("<div>compiled</div>"));
        // No wrapper div since no user attrs
        assert!(!html.contains("data-format"));
    }

    #[test]
    fn render_raw_block_compiled_with_class() {
        use crate::ast::{Node, Value};
        reset_id_counter();
        let mut raw = Node::raw_block("typst", "#rect()");
        raw.attrs.properties.insert("compiled".to_string(), Value::Text("<span>out</span>".to_string()));
        raw.attrs.classes.push("center".to_string());
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![raw]),
        };
        let html = render_html(&doc);
        assert!(html.contains("<div class=\"center\">"));
        assert!(html.contains("<span>out</span>"));
    }

    #[test]
    fn render_raw_block_compile_error() {
        use crate::ast::{Node, Value};
        reset_id_counter();
        let mut raw = Node::raw_block("typst", "#rect()");
        raw.attrs.properties.insert("compile_error".to_string(), Value::Text("some error".to_string()));
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![raw]),
        };
        let html = render_html(&doc);
        assert!(html.contains("raw-error"));
        assert!(html.contains("some error"));
    }

    #[test]
    fn render_table_ast() {
        use crate::ast::Node;
        reset_id_counter();
        let header = Node::table_row(vec![
            Node::table_cell(vec![Node::text("H1")]),
            Node::table_cell(vec![Node::text("H2")]),
        ]);
        let row = Node::table_row(vec![
            Node::table_cell(vec![Node::text("a")]),
            Node::table_cell(vec![Node::text("b")]),
        ]);
        let table = Node::table(vec![header, row]);
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![table]),
        };
        let html = render_html(&doc);
        assert!(html.contains("<thead>"), "should have thead: {html}");
        assert!(html.contains("<th>H1</th>"));
        assert!(html.contains("<th>H2</th>"));
        assert!(html.contains("<tbody>"));
        assert!(html.contains("<td>a</td>"));
        assert!(html.contains("</table>"));
    }

    #[test]
    fn render_ordered_list_start() {
        use crate::ast::Node;
        reset_id_counter();
        let items = vec![
            Node::list_item(vec![Node::paragraph(vec![Node::text("fifth")])]),
            Node::list_item(vec![Node::paragraph(vec![Node::text("sixth")])]),
        ];
        let list = Node::list(true, Some(5), items);
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![list]),
        };
        let html = render_html(&doc);
        assert!(html.contains("start=\"5\""), "should have start=5: {html}");
    }

    #[test]
    fn render_blockquote_ast() {
        use crate::ast::Node;
        reset_id_counter();
        let bq = Node::blockquote(vec![Node::paragraph(vec![Node::text("quoted")])]);
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![bq]),
        };
        let html = render_html(&doc);
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("quoted"));
    }

    #[test]
    fn render_citation_ast() {
        use crate::ast::Node;
        reset_id_counter();
        let cite = Node::citation("knuth");
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![Node::paragraph(vec![cite])]),
        };
        let html = render_html(&doc);
        assert!(html.contains("<cite"));
        assert!(html.contains("data-key=\"knuth\""));
        assert!(html.contains("[knuth]"));
    }

    #[test]
    fn render_image_with_explicit_id() {
        use crate::ast::Node;
        reset_id_counter();
        let mut img = Node::image("pic.png", "alt text");
        img.attrs.id = "my-img".to_string();
        img.attrs.auto_id = false;
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![Node::paragraph(vec![img])]),
        };
        let html = render_html(&doc);
        assert!(html.contains("id=\"my-img\""));
        assert!(html.contains("src=\"pic.png\""));
    }

    #[test]
    fn render_link_ast() {
        use crate::ast::Node;
        reset_id_counter();
        let link = Node::link("https://example.com", vec![Node::text("click")]);
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![Node::paragraph(vec![link])]),
        };
        let html = render_html(&doc);
        assert!(html.contains("<a href=\"https://example.com\">"));
        assert!(html.contains("click"));
        assert!(html.contains("</a>"));
    }

    #[test]
    fn render_heading_level_clamp() {
        use crate::ast::Node;
        reset_id_counter();
        // Level 7 heading (doesn't exist in HTML) should clamp to h6
        let h = Node::heading(7, vec![Node::text("Deep")]);
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![Node::section(7, vec![h])]),
        };
        let html = render_html(&doc);
        // min(7, 6) = 6
        assert!(html.contains("<h6"));
    }

    #[test]
    fn render_multiple_classes() {
        use crate::ast::Node;
        reset_id_counter();
        let mut para = Node::paragraph(vec![Node::text("styled")]);
        para.attrs.classes = vec!["a".to_string(), "b".to_string()];
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![para]),
        };
        let html = render_html(&doc);
        assert!(html.contains("class=\"a b\""), "should join classes with space: {html}");
    }

    #[test]
    fn render_thematic_break() {
        use crate::ast::Node;
        reset_id_counter();
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![
                Node::paragraph(vec![Node::text("above")]),
                Node::thematic_break(),
                Node::paragraph(vec![Node::text("below")]),
            ]),
        };
        let html = render_html(&doc);
        assert!(html.contains("<hr>"));
    }

    // ── Preview block rendering ──

    #[test]
    fn render_preview_block_compiled() {
        use crate::ast::{Node, Value};
        reset_id_counter();
        let mut cb = Node::code_block("typst", "#rect()");
        cb.attrs.properties.insert("preview".to_string(), Value::Bool(true));
        cb.attrs.properties.insert("compiled".to_string(), Value::Text("<div>rect</div>".to_string()));
        cb.attrs.id = "prev-1".to_string();
        cb.attrs.auto_id = false;
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![cb]),
        };
        let html = render_html(&doc);
        assert!(html.contains("preview-block"), "should have preview-block class: {html}");
        assert!(html.contains("preview-tab-input"), "should have tab inputs");
        assert!(html.contains("Preview</label>"), "should have Preview tab label");
        assert!(html.contains("Source</label>"), "should have Source tab label");
        assert!(html.contains("<div>rect</div>"), "should contain compiled output");
        assert!(html.contains("preview-output"), "should have preview output panel");
        assert!(html.contains("preview-source"), "should have preview source panel");
        assert!(html.contains("id=\"prev-1\""), "should have explicit id");
    }

    #[test]
    fn render_preview_block_auto_id_no_id_attr() {
        use crate::ast::{Node, Value};
        reset_id_counter();
        let mut cb = Node::code_block("typst", "#circle()");
        cb.attrs.properties.insert("preview".to_string(), Value::Bool(true));
        cb.attrs.properties.insert("compiled".to_string(), Value::Text("<svg/>".to_string()));
        // auto_id = true (default), so wrapper div should NOT have id= attribute
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![cb]),
        };
        let html = render_html(&doc);
        assert!(html.contains("preview-block"));
        // The wrapper div should not have an explicit id attribute
        assert!(html.contains("<div class=\"preview-block\">"), "auto-id wrapper should not have id attr: {html}");
    }

    #[test]
    fn render_preview_block_no_compiled_falls_back() {
        use crate::ast::{Node, Value};
        reset_id_counter();
        let mut cb = Node::code_block("typst", "#line()");
        cb.attrs.properties.insert("preview".to_string(), Value::Bool(true));
        // No "compiled" property → falls back to code block
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![cb]),
        };
        let html = render_html(&doc);
        assert!(!html.contains("preview-block"), "should fall back to code block");
        assert!(html.contains("<pre"), "should render as code block");
    }

    #[test]
    fn render_raw_block_compiled_with_explicit_id() {
        use crate::ast::{Node, Value};
        reset_id_counter();
        let mut raw = Node::raw_block("typst", "#rect()");
        raw.attrs.properties.insert("compiled".to_string(), Value::Text("<p>ok</p>".to_string()));
        raw.attrs.id = "my-block".to_string();
        raw.attrs.auto_id = false;
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![raw]),
        };
        let html = render_html(&doc);
        assert!(html.contains("id=\"my-block\""), "should have explicit id");
    }

    // ── Table rendering through parse ──

    #[test]
    fn render_table_full_pipeline() {
        let html = render("| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |");
        assert!(html.contains("<thead>"));
        assert!(html.contains("<th>A</th>"));
        assert!(html.contains("<th>B</th>"));
        assert!(html.contains("<tbody>"));
        assert!(html.contains("<td>1</td>"));
        assert!(html.contains("<td>2</td>"));
        assert!(html.contains("<td>3</td>"));
        assert!(html.contains("<td>4</td>"));
        assert!(html.contains("</table>"));
    }

    // ── Syntax highlighting ──

    #[test]
    fn render_code_block_with_known_lang() {
        // "rust" is in syntect's default set, should produce highlighted output
        let html = render("```rust\nfn main() {}\n```");
        assert!(html.contains("language-rust"), "should have language class: {html}");
        assert!(html.contains("<code"), "should have code element");
        assert!(html.contains("<pre"), "should have pre element");
    }

    // ── Escape functions ──

    #[test]
    fn escape_html_all_specials() {
        let result = escape_html("a & b < c > d");
        assert_eq!(result, "a &amp; b &lt; c &gt; d");
    }

    #[test]
    fn escape_html_no_specials() {
        let result = escape_html("hello world");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn escape_attr_all_specials() {
        let result = escape_attr("a & b < c > d \"e\"");
        assert_eq!(result, "a &amp; b &lt; c &gt; d &quot;e&quot;");
    }

    // ── Cross-ref with non-text property value ──

    #[test]
    fn render_cross_ref_non_text_href() {
        use crate::ast::{Node, Value};
        reset_id_counter();
        let mut cr = Node::cross_ref("target");
        // Set href to a non-Text value to exercise the `_ => None` branch
        cr.attrs.properties.insert("href".to_string(), Value::Number(42.0));
        cr.attrs.properties.insert("text".to_string(), Value::Number(99.0));
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![Node::paragraph(vec![cr])]),
        };
        let html = render_html(&doc);
        // Falls back to default href=#target and text=target
        assert!(html.contains("href=\"#target\""), "should use default href: {html}");
        assert!(html.contains(">target</a>"), "should use target as text: {html}");
    }

    // ── Caption, strikethrough, custom node ──

    #[test]
    fn render_strikethrough() {
        let html = render("~~deleted~~");
        assert!(html.contains("<del>"));
        assert!(html.contains("deleted"));
        assert!(html.contains("</del>"));
    }

    #[test]
    fn render_custom_node_transparent() {
        use crate::ast::Node;
        reset_id_counter();
        let custom = Node::new(crate::ast::NodeKind::Custom {
            content_type: "test".to_string(),
        }).with_children(vec![Node::paragraph(vec![Node::text("inner")])]);
        let doc = crate::ast::Document {
            meta: crate::ast::Metadata::default(),
            content: Node::section(0, vec![custom]),
        };
        let html = render_html(&doc);
        assert!(html.contains("inner"), "custom node should render children transparently");
    }

    #[test]
    fn numbered_headings_html_integration() {
        use crate::ast::reset_id_counter;
        use crate::resolve::resolve_document;
        reset_id_counter();
        let md = "\
{@var(index, [0], format=[\"I\",\"1\"])}
{@var(val, 0)}
{@init(heading, series=\"index\")}
{@on(enter, section > section, @pop(index, val), @inc(val), @push(index, val), @push(index, 0))}
{@on(exit, section > section, @pop(index))}
{@on([series=\"index\"], @pop(index, val), @store(number, index), @push(index, val))}
{@content(heading, \"{number}. \")}

# Introduction

## Background";
        let mut doc = crate::parse::parse(md).unwrap();
        resolve_document(&mut doc);
        let html = render_html(&doc);
        assert!(html.contains("I. Introduction"), "h1 should have Roman numeral: {html}");
        assert!(html.contains("I.1. Background"), "h2 should have hierarchical number: {html}");
    }

    #[test]
    fn cross_ref_resolves_to_label_html() {
        use crate::ast::reset_id_counter;
        use crate::resolve::resolve_document;
        reset_id_counter();
        let md = "\
{@var(fig, 0, format=\"1\")}
{@init(figure, series=\"fig\")}
{@on([series=\"fig\"], @inc(fig), @store(number, fig))}
{@init(figure, label=\"Figure {n}\")}

![Arch](arch.png){#fig:arch}

See @fig:arch.";
        let mut doc = crate::parse::parse(md).unwrap();
        resolve_document(&mut doc);
        let html = render_html(&doc);
        assert!(html.contains(">Figure 1</a>"), "cross-ref should resolve to 'Figure 1': {html}");
    }
}
