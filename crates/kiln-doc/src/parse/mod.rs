mod attrs;
mod block;
mod inline;
mod section;

pub use attrs::parse_prelude;

use crate::ast::{Attrs, Document, Metadata, Node, NodeKind};

use attrs::{
    apply_brace_attrs, extract_lang, extract_raw_format,
    maybe_extract_trailing_attrs_from_children, parse_directive_to_node,
};
use block::{RawBlock, parse_blocks};
use inline::parse_inline;
use section::build_section_tree;

// ── Errors ──

/// Errors that can occur during parsing.
#[derive(Debug)]
pub enum ParseError {
    /// YAML frontmatter failed to parse.
    Frontmatter(String),
    /// A directive was malformed.
    Directive { line: usize, message: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (label, detail) = match self {
            Self::Frontmatter(msg) => ("frontmatter", msg.clone()),
            Self::Directive { line, message } => {
                ("directive", format!("at line {line}: {message}"))
            }
        };
        write!(f, "{label} error: {detail}")
    }
}

impl std::error::Error for ParseError {}

// ── Public API ──

/// Parse a markdown string into a Document AST.
pub fn parse(markdown: &str) -> Result<Document, ParseError> {
    let (meta, body) = extract_frontmatter(markdown)?;
    let lines: Vec<&str> = body.lines().collect();
    let blocks = parse_blocks(&lines);
    let blocks = parse_inlines_in_blocks(blocks);
    let content = build_section_tree(blocks);
    Ok(Document { meta, content })
}

// ── Frontmatter ──

fn extract_frontmatter(input: &str) -> Result<(Metadata, &str), ParseError> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with("---") {
        return Ok((Metadata::default(), input));
    }
    // Find the opening ---
    let start = input.len() - trimmed.len();
    let after_open = start + 3;
    // The opening line must be just "---" (possibly with trailing whitespace)
    let first_newline = input[after_open..]
        .find('\n')
        .map(|i| after_open + i)
        .unwrap_or(input.len());
    if !input[after_open..first_newline].trim().is_empty() {
        return Ok((Metadata::default(), input));
    }
    let yaml_start = first_newline + 1;
    // Find the closing ---
    let rest = &input[yaml_start..];
    let mut pos = 0;
    let closing = loop {
        if pos >= rest.len() {
            // No closing --- found, treat entire input as body
            return Ok((Metadata::default(), input));
        }
        let line_end = rest[pos..]
            .find('\n')
            .map(|i| pos + i)
            .unwrap_or(rest.len());
        let line = rest[pos..line_end].trim();
        if line == "---" {
            break yaml_start + pos;
        }
        pos = line_end + 1;
    };
    let yaml_str = &input[yaml_start..closing];
    let meta: Metadata =
        serde_yaml_ng::from_str(yaml_str).map_err(|e| ParseError::Frontmatter(e.to_string()))?;
    // Body starts after the closing --- line
    let body_start = closing + 3;
    let body_start = input[body_start..]
        .find('\n')
        .map(|i| body_start + i + 1)
        .unwrap_or(input.len());
    Ok((meta, &input[body_start..]))
}

// ── Block-to-inline bridge ──

fn is_single_image(children: &[Node]) -> bool {
    children.len() == 1 && matches!(&children[0].kind, NodeKind::Image { .. })
}

fn promote_to_figure(children: Vec<Node>) -> Node {
    let mut img = children
        .into_iter()
        .next()
        .expect("is_single_image verified one child");
    let alt_text = if let NodeKind::Image { alt, .. } = &img.kind {
        alt.clone()
    } else {
        String::new()
    };
    // Transfer explicit ID and attrs from the image to the figure.
    let mut fig = Node::figure(Vec::new());
    if !img.attrs.auto_id {
        fig.attrs.id = std::mem::take(&mut img.attrs.id);
        fig.attrs.auto_id = false;
        let new_attrs = Attrs::default();
        img.attrs.id = new_attrs.id;
        img.attrs.auto_id = true;
    }
    // Transfer classes and properties from image to figure.
    fig.attrs.classes.append(&mut img.attrs.classes);
    fig.attrs.properties.extend(img.attrs.properties.drain());
    fig.children.push(img);
    if !alt_text.is_empty() {
        fig.children.push(Node::caption(vec![Node::text(alt_text)]));
    }
    fig
}

/// Convert RawBlocks into proper Nodes with inline parsing.
fn parse_inlines_in_blocks(blocks: Vec<RawBlock>) -> Vec<Node> {
    blocks
        .into_iter()
        .filter_map(|block| match block {
            RawBlock::Heading { level, text, attrs } => {
                let children = parse_inline(&text);
                let mut node = Node::heading(level, children);
                if let Some(attr_str) = attrs {
                    apply_brace_attrs(&mut node.attrs, &attr_str);
                }
                Some(node)
            }
            RawBlock::Paragraph(text) => {
                let trimmed = text.trim();
                // Check for standalone directive paragraph
                if trimmed.starts_with("{@") && trimmed.ends_with('}') {
                    // Already handled as Directive, but handle if slipped through
                    return None;
                }
                let mut children = parse_inline(trimmed);
                if children.is_empty() {
                    return None;
                }
                // Extract trailing brace attrs before figure promotion, so
                // `![alt](url){#id}` gets the attrs applied to the Image node
                // and is then eligible for promotion.
                maybe_extract_trailing_attrs_from_children(&mut children);
                // Figure promotion: paragraph with single image
                if is_single_image(&children) {
                    return Some(promote_to_figure(children));
                }
                Some(Node::paragraph(children))
            }
            RawBlock::CodeBlock { source, attrs } => {
                let (raw_format, rest) = extract_raw_format(attrs.as_deref());
                let (mut node, rest) = if let Some(format) = raw_format {
                    (Node::raw_block(&format, source), rest)
                } else {
                    let (lang, rest) = extract_lang(rest.as_deref());
                    (
                        Node::code_block(lang.as_deref().unwrap_or(""), source),
                        rest,
                    )
                };
                if let Some(a) = rest {
                    apply_brace_attrs(&mut node.attrs, &a);
                }
                Some(node)
            }
            RawBlock::BlockQuote(inner) => {
                let children = parse_inlines_in_blocks(inner);
                Some(Node::blockquote(children))
            }
            RawBlock::List {
                ordered,
                start,
                items,
            } => {
                let item_nodes: Vec<Node> = items
                    .into_iter()
                    .map(|item_blocks| {
                        let children = parse_inlines_in_blocks(item_blocks);
                        Node::list_item(children)
                    })
                    .collect();
                Some(Node::list(ordered, start, item_nodes))
            }
            RawBlock::ThematicBreak => Some(Node::thematic_break()),
            RawBlock::Table { header, rows } => {
                let header_cells: Vec<Node> = header
                    .into_iter()
                    .map(|cell| Node::table_cell(parse_inline(&cell)))
                    .collect();
                let mut table_children = vec![Node::table_row(header_cells)];
                for row in rows {
                    let cells: Vec<Node> = row
                        .into_iter()
                        .map(|cell| Node::table_cell(parse_inline(&cell)))
                        .collect();
                    table_children.push(Node::table_row(cells));
                }
                Some(Node::table(table_children))
            }
            RawBlock::Directive(text) => {
                // Parse the directive and return None — attach to parent scope
                // For now, directives at block level are stored as custom nodes
                // that the resolver will process.
                let inner = &text[1..text.len() - 1]; // strip { }
                parse_directive_to_node(inner)
            }
        })
        .collect()
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::reset_id_counter;

    fn parse_ok(md: &str) -> Document {
        reset_id_counter();
        parse(md).unwrap()
    }

    #[test]
    fn frontmatter_parsing() {
        let doc = parse_ok("---\ntitle: Hello\ndate: 2026-01-01\ntags: [a, b]\n---\n\nBody text.");
        assert_eq!(doc.meta.title.as_deref(), Some("Hello"));
        assert_eq!(doc.meta.date.as_deref(), Some("2026-01-01"));
        assert_eq!(doc.meta.tags, vec!["a", "b"]);
    }

    #[test]
    fn headings() {
        let doc = parse_ok("# Title\n\n## Subtitle\n\nSome text.\n\n## Another");
        // Root section should contain one level-1 section
        assert_eq!(doc.content.children.len(), 1);
        let sec1 = &doc.content.children[0];
        assert!(matches!(sec1.kind, NodeKind::Section { level: 1 }));
        // level-1 section has: heading, paragraph, level-2 section, level-2 section
        // First child is the heading
        assert!(matches!(
            sec1.children[0].kind,
            NodeKind::Heading { level: 1 }
        ));
    }

    #[test]
    fn heading_with_attrs() {
        let doc = parse_ok("# My Title {#custom-id .intro}");
        let sec1 = &doc.content.children[0];
        let heading = &sec1.children[0];
        assert_eq!(heading.attrs.id, "custom-id");
        assert_eq!(heading.attrs.classes, vec!["intro"]);
    }

    #[test]
    fn paragraph_inline() {
        let doc = parse_ok("Hello *world* and **bold** text.");
        let sec = &doc.content;
        let para = &sec.children[0];
        assert!(matches!(para.kind, NodeKind::Paragraph));
        // Should have: Text, Emphasis, Text, Strong, Text
        assert!(para.children.len() >= 5);
    }

    #[test]
    fn code_block() {
        let doc = parse_ok("```rust\nfn main() {}\n```");
        let sec = &doc.content;
        let code = &sec.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { .. }));
        if let NodeKind::CodeBlock {
            ref lang,
            ref source,
        } = code.kind
        {
            assert_eq!(lang, "rust");
            assert_eq!(source, "fn main() {}");
        }
    }

    #[test]
    fn typst_code_block_not_raw() {
        // ```typst without {=typst} is a normal code block
        let doc = parse_ok("```typst\n#rect()\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::CodeBlock { .. }));
    }

    #[test]
    fn raw_block_brace_format() {
        let doc = parse_ok("```{=html}\n<div>hi</div>\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::RawBlock { ref format, .. } if format == "html"));
    }

    #[test]
    fn raw_block_brace_format_with_attrs() {
        let doc = parse_ok("```{=typst #fig:x .wide}\n#rect()\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::RawBlock { ref format, .. } if format == "typst"));
        assert_eq!(node.attrs.id, "fig:x");
        assert!(node.attrs.classes.contains(&"wide".to_string()));
    }

    #[test]
    fn inline_math() {
        let doc = parse_ok("The equation $E = mc^2$ is famous.");
        let para = &doc.content.children[0];
        // Should contain: Text, Math(inline), Text
        let has_math = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Math { display: false, .. }));
        assert!(has_math);
    }

    #[test]
    fn display_math() {
        let doc = parse_ok("$$\nE^2 = (pc)^2\n$$");
        let para = &doc.content.children[0];
        let has_math = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Math { display: true, .. }));
        assert!(has_math);
    }

    #[test]
    fn citation() {
        let doc = parse_ok("See [@knuth1997] for details.");
        let para = &doc.content.children[0];
        let has_cite = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Citation { ref key } if key == "knuth1997"));
        assert!(has_cite);
    }

    #[test]
    fn cross_ref() {
        let doc = parse_ok("See @fig:arch for the diagram.");
        let para = &doc.content.children[0];
        let has_ref = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::CrossRef { ref target } if target == "fig:arch"));
        assert!(has_ref);
    }

    #[test]
    fn link() {
        let doc = parse_ok("Visit [example](https://example.com) now.");
        let para = &doc.content.children[0];
        let has_link = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Link { ref url } if url == "https://example.com"));
        assert!(has_link);
    }

    #[test]
    fn image_promoted_to_figure() {
        let doc = parse_ok("![Alt text](image.png)");
        let fig = &doc.content.children[0];
        assert!(matches!(fig.kind, NodeKind::Figure));
    }

    #[test]
    fn unordered_list() {
        let doc = parse_ok("- item one\n- item two\n- item three");
        let list = &doc.content.children[0];
        assert!(matches!(list.kind, NodeKind::List { ordered: false, .. }));
        assert_eq!(list.children.len(), 3);
    }

    #[test]
    fn ordered_list() {
        let doc = parse_ok("1. first\n2. second\n3. third");
        let list = &doc.content.children[0];
        assert!(matches!(
            list.kind,
            NodeKind::List {
                ordered: true,
                start: Some(1)
            }
        ));
        assert_eq!(list.children.len(), 3);
    }

    #[test]
    fn block_quote() {
        let doc = parse_ok("> quoted text\n> more quoted");
        let bq = &doc.content.children[0];
        assert!(matches!(bq.kind, NodeKind::BlockQuote));
    }

    #[test]
    fn thematic_break() {
        let doc = parse_ok("text\n\n---\n\nmore text");
        assert!(
            doc.content
                .children
                .iter()
                .any(|n| matches!(n.kind, NodeKind::ThematicBreak))
        );
    }

    #[test]
    fn escaped_at() {
        let doc = parse_ok("Email me at user\\@example.com");
        let para = &doc.content.children[0];
        // Should NOT have a cross-ref node
        let has_ref = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::CrossRef { .. }));
        assert!(!has_ref);
    }

    #[test]
    fn code_block_with_attrs() {
        let doc = parse_ok("```{rust #lst:hello caption=\"Hello world\"}\nfn main() {}\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "rust"));
        assert_eq!(code.attrs.id, "lst:hello");
        assert!(code.attrs.properties.contains_key("caption"));
    }

    #[test]
    fn section_nesting() {
        let doc = parse_ok("# A\n\n## B\n\n### C\n\n## D");
        // Root > Section1(A) > [Heading, Section2(B), Section2(D)]
        let sec_a = &doc.content.children[0];
        assert!(matches!(sec_a.kind, NodeKind::Section { level: 1 }));
        // B and D are children of A
        let sub_sections: Vec<_> = sec_a
            .children
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Section { .. }))
            .collect();
        assert_eq!(sub_sections.len(), 2);
    }

    #[test]
    fn inline_code() {
        let doc = parse_ok("Use `println!` macro.");
        let para = &doc.content.children[0];
        let has_code = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Code(ref s) if s == "println!"));
        assert!(has_code);
    }

    #[test]
    fn strikethrough() {
        let doc = parse_ok("This is ~~deleted~~ text.");
        let para = &doc.content.children[0];
        let has_del = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Strikethrough));
        assert!(has_del);
    }

    #[test]
    fn series_directive() {
        let doc = parse_ok("{@series(figs, label=\"Figure\", display=hierarchical)}");
        // Should produce a custom node with the series declaration
        let has_series = doc
            .content
            .children
            .iter()
            .any(|n| !n.attrs.declarations.is_empty());
        assert!(has_series);
    }

    #[test]
    fn table_basic() {
        let doc =
            parse_ok("| Name | Value |\n|------|-------|\n| foo  | 1     |\n| bar  | 2     |");
        let table = &doc.content.children[0];
        assert!(matches!(table.kind, NodeKind::Table));
        assert_eq!(table.children.len(), 3); // 1 header row + 2 data rows
    }

    #[test]
    fn table_renders_html() {
        let html = {
            reset_id_counter();
            let doc = parse("| A | B |\n|---|---|\n| 1 | 2 |").unwrap();
            crate::html::render_html(&doc)
        };
        assert!(html.contains("<thead>"));
        assert!(html.contains("<th>"));
        assert!(html.contains("<tbody>"));
        assert!(html.contains("<td>"));
    }

    #[test]
    fn table_escaped_pipe() {
        let doc = parse_ok("| Expr | Result |\n|------|--------|\n| a \\| b | OR |");
        let table = &doc.content.children[0];
        assert!(matches!(table.kind, NodeKind::Table));
        // Should be 2 rows (header + 1 data), each with 2 cells
        assert_eq!(table.children.len(), 2);
        assert_eq!(table.children[1].children.len(), 2);
    }

    #[test]
    fn table_pipe_in_code_span() {
        let doc = parse_ok("| Code | Result |\n|------|--------|\n| `a | b` | OR |");
        let table = &doc.content.children[0];
        assert!(matches!(table.kind, NodeKind::Table));
        assert_eq!(table.children.len(), 2);
        assert_eq!(table.children[1].children.len(), 2);
    }

    #[test]
    fn malformed_frontmatter() {
        let result = parse("---\n: invalid yaml [[\n---\nBody");
        assert!(matches!(result, Err(ParseError::Frontmatter(_))));
    }

    #[test]
    fn unclosed_frontmatter() {
        // Missing closing --- — should treat entire content as frontmatter attempt
        let result = parse("---\ntitle: hello\n\nBody text without closing");
        // The parser should either error or treat it as no frontmatter
        // Current behavior: it looks for closing ---, doesn't find it, so no frontmatter
        assert!(result.is_ok());
        let doc = result.unwrap();
        // Without closing ---, no frontmatter is extracted
        assert!(doc.meta.title.is_none());
    }

    #[test]
    fn auto_id_flag_set() {
        reset_id_counter();
        let doc = parse("Hello.").unwrap();
        // The paragraph node should have auto_id = true
        assert!(!doc.content.children.is_empty());
        assert!(doc.content.children[0].attrs.auto_id);
    }

    #[test]
    fn explicit_id_clears_auto_flag() {
        reset_id_counter();
        let doc = parse("# Title {#my-title}").unwrap();
        // Find the heading node (inside the section)
        let section = &doc.content.children[0];
        let heading = &section.children[0];
        assert!(matches!(&heading.kind, NodeKind::Heading { level: 1 }));
        assert_eq!(heading.attrs.id, "my-title");
        assert!(!heading.attrs.auto_id);
    }

    // ── Block-level coverage ──

    #[test]
    fn blockquote_lazy_continuation() {
        let doc = parse_ok("> quote\ncontinued");
        let bq = &doc.content.children[0];
        assert!(matches!(bq.kind, NodeKind::BlockQuote));
    }

    #[test]
    fn blockquote_empty_line() {
        let doc = parse_ok("> text\n>\n> more");
        let bq = &doc.content.children[0];
        assert!(matches!(bq.kind, NodeKind::BlockQuote));
    }

    #[test]
    fn list_with_blank_lines() {
        let doc = parse_ok("- a\n\n- b");
        let list = &doc.content.children[0];
        assert!(matches!(list.kind, NodeKind::List { ordered: false, .. }));
        assert_eq!(list.children.len(), 2);
    }

    #[test]
    fn ordered_list_paren() {
        let doc = parse_ok("1) first\n2) second");
        let list = &doc.content.children[0];
        assert!(matches!(
            list.kind,
            NodeKind::List {
                ordered: true,
                start: Some(1)
            }
        ));
        assert_eq!(list.children.len(), 2);
    }

    #[test]
    fn heading_no_space_not_heading() {
        let doc = parse_ok("##NoSpace");
        // Should be a paragraph, not a heading
        assert!(matches!(doc.content.children[0].kind, NodeKind::Paragraph));
    }

    // ── Inline-level coverage ──

    #[test]
    fn strong_emphasis_combo() {
        let doc = parse_ok("***bold italic***");
        let para = &doc.content.children[0];
        let has_strong = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Strong));
        assert!(has_strong);
    }

    #[test]
    fn underscore_emphasis() {
        let doc = parse_ok("_italic_ and __bold__");
        let para = &doc.content.children[0];
        let has_em = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Emphasis));
        let has_strong = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Strong));
        assert!(has_em);
        assert!(has_strong);
    }

    #[test]
    fn hard_break_two_spaces() {
        let doc = parse_ok("line1  \nline2");
        let para = &doc.content.children[0];
        assert!(
            para.children
                .iter()
                .any(|n| matches!(n.kind, NodeKind::HardBreak))
        );
    }

    #[test]
    fn hard_break_backslash() {
        let doc = parse_ok("line1\\\nline2");
        let para = &doc.content.children[0];
        assert!(
            para.children
                .iter()
                .any(|n| matches!(n.kind, NodeKind::HardBreak))
        );
    }

    #[test]
    fn soft_break() {
        let doc = parse_ok("line1\nline2");
        let para = &doc.content.children[0];
        assert!(
            para.children
                .iter()
                .any(|n| matches!(n.kind, NodeKind::SoftBreak))
        );
    }

    #[test]
    fn non_escapable_backslash() {
        let doc = parse_ok("\\z");
        let para = &doc.content.children[0];
        // Backslash before non-escapable char should remain as literal text
        let has_text = para
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Text(ref t) if t.contains('\\')));
        assert!(has_text);
    }

    #[test]
    fn multiple_citations() {
        let doc = parse_ok("[@a; @b; @c]");
        let para = &doc.content.children[0];
        let cite_count = para
            .children
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Citation { .. }))
            .count();
        assert_eq!(cite_count, 3);
    }

    // ── Attribute coverage ──

    #[test]
    fn paragraph_trailing_attrs() {
        let doc = parse_ok("*emphasized* {.highlight}");
        let para = &doc.content.children[0];
        // Trailing {.highlight} applies to the preceding emphasis node
        let has_class = para
            .children
            .iter()
            .any(|n| n.attrs.classes.contains(&"highlight".to_string()));
        assert!(has_class);
    }

    #[test]
    fn inline_trailing_attrs_on_link() {
        let doc = parse_ok("[click](url) {.btn}");
        let para = &doc.content.children[0];
        let link = para
            .children
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Link { .. }));
        assert!(link.is_some());
        assert!(link.unwrap().attrs.classes.contains(&"btn".to_string()));
    }

    #[test]
    fn code_block_key_value_attrs() {
        let doc = parse_ok("```{rust width=\"80%\"}\ncode\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "rust"));
        assert!(code.attrs.properties.contains_key("width"));
    }

    #[test]
    fn hint_directive() {
        let doc = parse_ok("{@hint(figure, series=figs)}");
        let has_hint = doc
            .content
            .children
            .iter()
            .any(|n| !n.attrs.inits.is_empty());
        assert!(has_hint);
    }

    fn extract_series_config(
        doc: &crate::ast::Document,
    ) -> Option<crate::ast::SeriesConfig> {
        use crate::ast::Declaration;
        let decl = doc
            .content
            .children
            .iter()
            .flat_map(|n| &n.attrs.declarations)
            .find(|d| matches!(d, Declaration::Series { .. }))?;
        if let Declaration::Series { config, .. } = decl {
            Some(config.clone())
        } else {
            None
        }
    }

    #[test]
    fn series_formats_list() {
        let doc = parse_ok("{@series(nums, formats=[\"I\",\"1\",\"a\"])}");
        assert_eq!(
            extract_series_config(&doc).unwrap().formats,
            vec!["I", "1", "a"]
        );
    }

    #[test]
    fn figure_empty_alt() {
        let doc = parse_ok("![](image.png)");
        let fig = &doc.content.children[0];
        assert!(matches!(fig.kind, NodeKind::Figure));
        // No caption since alt is empty
        let has_caption = fig
            .children
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Caption));
        assert!(!has_caption);
    }

    #[test]
    fn directive_error_display() {
        let err = ParseError::Directive {
            line: 5,
            message: "bad".to_string(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("directive"));
        assert!(msg.contains("line 5"));
    }

    #[test]
    fn hint_multiple_properties() {
        let doc = parse_ok("{@hint(figure, layout=full, position=top)}");
        let hints: Vec<_> = doc
            .content
            .children
            .iter()
            .flat_map(|n| n.attrs.inits.iter())
            .collect();
        assert_eq!(hints.len(), 2, "should produce 2 hints for 2 properties");
    }

    #[test]
    fn hint_with_compound_selector() {
        let doc = parse_ok("{@hint(code-block.wide, theme=dark)}");
        let hints: Vec<_> = doc
            .content
            .children
            .iter()
            .flat_map(|n| n.attrs.inits.iter())
            .collect();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "theme");
    }

    #[test]
    fn hint_quoted_value() {
        use crate::ast::Value;
        let doc = parse_ok("{@hint(figure, label=\"Figure\")}");
        let hints: Vec<_> = doc
            .content
            .children
            .iter()
            .flat_map(|n| n.attrs.inits.iter())
            .collect();
        assert_eq!(hints.len(), 1);
        assert!(matches!(&hints[0].value, Value::Text(s) if s == "Figure"));
    }

    #[test]
    fn hint_bool_value() {
        use crate::ast::Value;
        let doc = parse_ok("{@hint(heading, numbering=true)}");
        let hints: Vec<_> = doc
            .content
            .children
            .iter()
            .flat_map(|n| n.attrs.inits.iter())
            .collect();
        assert_eq!(hints.len(), 1);
        assert!(matches!(&hints[0].value, Value::Bool(true)));
    }

    #[test]
    fn hint_false_bool_value() {
        use crate::ast::Value;
        let doc = parse_ok("{@hint(heading, numbering=false)}");
        let hints: Vec<_> = doc
            .content
            .children
            .iter()
            .flat_map(|n| n.attrs.inits.iter())
            .collect();
        assert_eq!(hints.len(), 1);
        assert!(matches!(&hints[0].value, Value::Bool(false)));
    }

    #[test]
    fn hint_number_value() {
        use crate::ast::Value;
        let doc = parse_ok("{@hint(heading, indent=4)}");
        let hints: Vec<_> = doc
            .content
            .children
            .iter()
            .flat_map(|n| n.attrs.inits.iter())
            .collect();
        assert_eq!(hints.len(), 1);
        assert!(matches!(&hints[0].value, Value::Number(n) if *n == 4.0));
    }

    #[test]
    fn hint_no_properties_produces_empty() {
        // Only selector, no key=value → no hints produced
        let doc = parse_ok("{@hint(figure)}");
        let hints: Vec<_> = doc
            .content
            .children
            .iter()
            .flat_map(|n| n.attrs.inits.iter())
            .collect();
        assert!(hints.is_empty());
    }

    #[test]
    fn hint_invalid_selector_produces_empty() {
        let doc = parse_ok("{@hint(nonexistent, x=1)}");
        let hints: Vec<_> = doc
            .content
            .children
            .iter()
            .flat_map(|n| n.attrs.inits.iter())
            .collect();
        assert!(hints.is_empty());
    }

    #[test]
    fn hint_this_selector() {
        use crate::ast::Selector;
        let doc = parse_ok("# Title {@hint(this, color=red)}");
        fn find_hint(node: &Node) -> Option<&crate::ast::Init> {
            node.attrs
                .inits
                .iter()
                .find(|h| matches!(h.selector, Selector::This))
                .or_else(|| node.children.iter().find_map(find_hint))
        }
        let hint = find_hint(&doc.content).expect("should find this hint");
        assert_eq!(hint.property, "color");
    }

    #[test]
    fn hint_or_selector() {
        let doc = parse_ok("{@hint(figure, code-block, theme=dark)}");
        let hints: Vec<_> = doc
            .content
            .children
            .iter()
            .flat_map(|n| n.attrs.inits.iter())
            .collect();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "theme");
    }

    #[test]
    fn series_parent_and_cycle() {
        let doc = parse_ok("{@series(fig, parent=sec, cycle=true)}");
        let config = extract_series_config(&doc).unwrap();
        assert_eq!(config.parent, Some("sec".to_string()));
        assert!(config.cycle);
    }

    #[test]
    fn brace_attrs_key_value_bare() {
        // Unquoted value
        let doc = parse_ok("```{html width=80%}\ncode\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "html"));
        assert!(code.attrs.properties.contains_key("width"));
    }

    #[test]
    fn raw_block_colon_shorthand() {
        // {=typst:#id} colon shorthand
        let doc = parse_ok("```{=typst:#fig:x}\n#rect()\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::RawBlock { ref format, .. } if format == "typst"));
        assert_eq!(node.attrs.id, "fig:x");
    }

    #[test]
    fn code_block_bare_boolean_attr() {
        use crate::ast::Value;
        let doc = parse_ok("```{rust nowrap}\ncode\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "rust"));
        assert!(matches!(
            code.attrs.properties.get("nowrap"),
            Some(Value::Bool(true))
        ));
    }

    #[test]
    fn raw_block_colon_empty_remainder() {
        // {=typst:} — colon with nothing after it
        let doc = parse_ok("```{=typst:}\n#rect()\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::RawBlock { ref format, .. } if format == "typst"));
    }

    #[test]
    fn heading_level_too_deep() {
        // ####### (7 hashes) should not be a heading
        let doc = parse_ok("####### not a heading");
        assert!(matches!(doc.content.children[0].kind, NodeKind::Paragraph));
    }

    #[test]
    fn two_backticks_not_a_fence() {
        // `` is not a valid fence (need 3+)
        let doc = parse_ok("``not a fence``");
        assert!(matches!(doc.content.children[0].kind, NodeKind::Paragraph));
    }

    #[test]
    fn code_block_with_multiple_attrs() {
        use crate::ast::Value;
        let doc = parse_ok("```{rust #lst:main .wide caption=\"Main\" nowrap}\nfn main() {}\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "rust"));
        assert_eq!(code.attrs.id, "lst:main");
        assert!(code.attrs.classes.contains(&"wide".to_string()));
        assert!(
            matches!(code.attrs.properties.get("caption"), Some(Value::Text(s)) if s == "Main")
        );
        assert!(matches!(
            code.attrs.properties.get("nowrap"),
            Some(Value::Bool(true))
        ));
    }

    #[test]
    fn series_counts_generates_hint() {
        use crate::ast::Value;
        use crate::visit::Pass;
        let doc = parse_ok("{@series(fig, counts=figure)}");
        // After hoisting, root should have a hint targeting Figure with series="fig".
        let mut doc = doc;
        crate::resolve::HoistDirectives.run(&mut doc);
        let hint = doc
            .content
            .attrs
            .inits
            .iter()
            .find(|h| h.property == "series");
        assert!(hint.is_some(), "counts= should generate a series hint");
        assert!(matches!(&hint.unwrap().value, Value::Text(s) if s == "fig"));
    }

    #[test]
    fn series_default_formats() {
        let doc = parse_ok("{@series(x)}");
        let config = extract_series_config(&doc).unwrap();
        assert_eq!(config.formats, vec!["1"]);
        assert!(config.parent.is_none());
        assert!(!config.cycle);
    }

    #[test]
    fn figure_trailing_id_promoted() {
        // Trailing {#id} on an image should transfer to the Figure wrapper.
        let doc = parse_ok("![Alt](img.png){#fig:test}");
        let fig = &doc.content.children[0];
        assert!(matches!(fig.kind, NodeKind::Figure));
        assert_eq!(fig.attrs.id, "fig:test");
        assert!(!fig.attrs.auto_id);
    }

    // ── Table parsing tests ──

    #[test]
    fn table_alignment_markers() {
        let doc = parse_ok("| Left | Center | Right |\n|:---|:---:|---:|\n| a | b | c |");
        let table = &doc.content.children[0];
        assert!(matches!(table.kind, NodeKind::Table));
        assert_eq!(table.children.len(), 2);
        let header = &table.children[0];
        assert_eq!(header.children.len(), 3);
    }

    #[test]
    fn table_multiple_body_rows() {
        let doc = parse_ok("| H |\n|---|\n| a |\n| b |\n| c |");
        let table = &doc.content.children[0];
        assert!(matches!(table.kind, NodeKind::Table));
        // header + 3 body rows
        assert_eq!(table.children.len(), 4);
    }

    // ── Ordered list tests ──

    #[test]
    fn ordered_list_custom_start() {
        let doc = parse_ok("5. fifth\n6. sixth");
        let list = &doc.content.children[0];
        assert!(matches!(list.kind, NodeKind::List { ordered: true, start: Some(5) }));
    }

    // ── Nested blockquote tests ──

    #[test]
    fn nested_blockquote_two_levels() {
        let doc = parse_ok("> outer\n> > inner");
        let bq = &doc.content.children[0];
        assert!(matches!(bq.kind, NodeKind::BlockQuote));
        assert!(!bq.children.is_empty());
    }

    #[test]
    fn blockquote_multiline() {
        let doc = parse_ok("> first line\n> second line");
        let bq = &doc.content.children[0];
        assert!(matches!(bq.kind, NodeKind::BlockQuote));
    }

    // ── Thematic break tests ──

    #[test]
    fn two_dashes_not_thematic_break() {
        let doc = parse_ok("--");
        // Should be a paragraph, not a thematic break
        assert!(!doc.content.children.iter().any(|c| matches!(c.kind, NodeKind::ThematicBreak)));
    }

    // ── Code fence edge cases ──

    #[test]
    fn tilde_fence() {
        let doc = parse_ok("~~~\ncode here\n~~~");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::CodeBlock { .. }));
        if let NodeKind::CodeBlock { source, .. } = &node.kind {
            assert_eq!(source, "code here");
        }
    }

    #[test]
    fn unclosed_fence_extends_to_eof() {
        let doc = parse_ok("```\ncode without close");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::CodeBlock { .. }));
    }

    #[test]
    fn fence_mismatched_char_not_closed() {
        // Opening with backticks, "closing" with tildes should not close
        let doc = parse_ok("```\ncode\n~~~");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::CodeBlock { .. }));
        if let NodeKind::CodeBlock { source, .. } = &node.kind {
            assert!(source.contains("~~~"), "tilde line should be part of source");
        }
    }

    // ── Frontmatter edge cases ──

    #[test]
    fn frontmatter_with_content_after_dashes() {
        // --- followed by text on same line is not valid frontmatter
        let doc = parse_ok("--- not frontmatter\nbody text");
        assert!(doc.meta.title.is_none());
    }

    #[test]
    fn frontmatter_empty() {
        let doc = parse_ok("---\n---\n\nBody.");
        // Empty YAML should give default metadata
        assert!(doc.meta.title.is_none());
        assert!(!doc.content.children.is_empty());
    }

    // ── Directive parsing ──

    #[test]
    fn directive_hint_with_selector_and_value() {
        use crate::ast::Value;
        use crate::visit::Pass;
        let doc = parse_ok("{@hint(figure.wide, layout=\"full\")}");
        let mut doc = doc;
        crate::resolve::HoistDirectives.run(&mut doc);
        let hint = doc.content.attrs.inits.iter().find(|h| h.property == "layout");
        assert!(hint.is_some());
        assert!(matches!(&hint.unwrap().value, Value::Text(s) if s == "full"));
    }

    #[test]
    fn directive_series_with_parent_and_cycle() {
        let doc = parse_ok("{@series(fig, parent=sec, formats=[\"I\",\"1\"], cycle=true)}");
        let config = extract_series_config(&doc).unwrap();
        assert_eq!(config.parent.as_deref(), Some("sec"));
        assert_eq!(config.formats, vec!["I", "1"]);
        assert!(config.cycle);
    }

    #[test]
    fn directive_series_with_counts_and_format() {
        use crate::ast::Value;
        use crate::visit::Pass;
        let doc = parse_ok("{@series(sec, counts=heading, formats=[\"I\",\"1\",\"a\"])}");
        let config = extract_series_config(&doc).unwrap();
        assert_eq!(config.formats, vec!["I", "1", "a"]);

        // Also verify that counts= produces a hint
        let mut doc2 = parse_ok("{@series(sec, counts=heading)}");
        crate::resolve::HoistDirectives.run(&mut doc2);
        let hint = doc2.content.attrs.inits.iter().find(|h| h.property == "series");
        assert!(hint.is_some());
        assert!(matches!(&hint.unwrap().value, Value::Text(s) if s == "sec"));
    }

    // ── Lang extraction from code fences ──

    #[test]
    fn code_block_lang_with_extra_attrs() {
        let doc = parse_ok("```python #my-code .highlight\nprint('hello')\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "python"));
        assert_eq!(node.attrs.id, "my-code");
        assert!(node.attrs.classes.contains(&"highlight".to_string()));
    }

    // ── Brace attr edge cases ──

    #[test]
    fn image_with_classes() {
        let doc = parse_ok("![Alt](img.png){.wide .dark}");
        let fig = &doc.content.children[0];
        assert!(matches!(fig.kind, NodeKind::Figure));
        // Classes should transfer to figure
        assert!(fig.attrs.classes.contains(&"wide".to_string()));
        assert!(fig.attrs.classes.contains(&"dark".to_string()));
    }

    #[test]
    fn image_with_property() {
        use crate::ast::Value;
        let doc = parse_ok("![Alt](img.png){width=\"80%\"}");
        let fig = &doc.content.children[0];
        assert!(matches!(fig.kind, NodeKind::Figure));
        assert!(matches!(fig.attrs.properties.get("width"), Some(Value::Text(s)) if s == "80%"));
    }

    // ── Nested list ──

    #[test]
    fn nested_unordered_list() {
        let doc = parse_ok("- outer\n  - inner\n- second");
        let list = &doc.content.children[0];
        assert!(matches!(list.kind, NodeKind::List { ordered: false, .. }));
        // First item should contain a nested list
        let first_item = &list.children[0];
        let has_nested = first_item.children.iter().any(|c| matches!(c.kind, NodeKind::List { .. }));
        assert!(has_nested, "Should have a nested list");
    }

    #[test]
    fn malformed_frontmatter_error() {
        let result = parse("---\n{invalid yaml: [[\n---\n");
        assert!(result.is_err());
    }

    #[test]
    fn list_item_with_continuation() {
        let doc = parse_ok("- line 1\n  line 2\n- item 2");
        let list = &doc.content.children[0];
        assert!(matches!(list.kind, NodeKind::List { .. }));
        assert_eq!(list.children.len(), 2);
    }

    #[test]
    fn list_item_multiline_paragraph() {
        let doc = parse_ok("- para 1\n\n  para 2\n\n- item 2");
        let list = &doc.content.children[0];
        assert!(matches!(list.kind, NodeKind::List { .. }));
    }

    // ── Paragraph break conditions ──

    #[test]
    fn paragraph_stops_at_heading() {
        let doc = parse_ok("para text\n# Heading");
        // Should have both a paragraph and a section with heading
        assert!(doc.content.children.len() >= 2 ||
            doc.content.children.iter().any(|c| matches!(c.kind, NodeKind::Section { .. })));
    }

    #[test]
    fn paragraph_stops_at_fence() {
        let doc = parse_ok("para text\n```\ncode\n```");
        assert!(doc.content.children.len() >= 2);
        let has_code = doc.content.children.iter().any(|c| matches!(c.kind, NodeKind::CodeBlock { .. }));
        assert!(has_code);
    }

    #[test]
    fn paragraph_stops_at_blockquote() {
        let doc = parse_ok("para text\n> quoted");
        assert!(doc.content.children.len() >= 2);
    }

    #[test]
    fn paragraph_stops_at_list() {
        let doc = parse_ok("para text\n- item");
        assert!(doc.content.children.len() >= 2);
    }

    #[test]
    fn paragraph_stops_at_directive() {
        let doc = parse_ok("para text\n{@hint(figure, series=figs)}");
        assert!(!doc.content.children.is_empty());
    }

    #[test]
    fn paragraph_stops_at_table() {
        let doc = parse_ok("para text\n| A | B |\n|---|---|\n| 1 | 2 |");
        let has_table = doc.content.children.iter().any(|c| matches!(c.kind, NodeKind::Table));
        assert!(has_table);
    }

    // ── Code block: display math block ──

    #[test]
    fn display_math_with_id() {
        let doc = parse_ok("$$ {#eq:one}\nx + y\n$$");
        // Math with attrs might be captured differently — verify it parses
        assert!(!doc.content.children.is_empty());
    }

    // ── Heading trailing hash stripping ──

    #[test]
    fn heading_trailing_hashes_stripped() {
        let doc = parse_ok("## Title ##");
        let sec = &doc.content.children[0];
        let heading = &sec.children[0];
        // The heading text should be "Title" (trailing ## stripped)
        if let Some(NodeKind::Text(s)) = heading.children.first().map(|n| &n.kind) {
            assert_eq!(s, "Title");
        }
    }

    // ── Multiple inline attrs ──

    #[test]
    fn heading_multiple_classes() {
        let doc = parse_ok("# Title {.intro .highlighted}");
        let sec = &doc.content.children[0];
        let heading = &sec.children[0];
        assert!(heading.attrs.classes.contains(&"intro".to_string()));
        assert!(heading.attrs.classes.contains(&"highlighted".to_string()));
    }

    // ── Table without outer pipes ──

    #[test]
    fn table_no_outer_pipes() {
        let doc = parse_ok("A | B\n---|---\n1 | 2");
        let table = &doc.content.children[0];
        assert!(matches!(table.kind, NodeKind::Table));
    }

    // ── Empty raw block ──

    #[test]
    fn empty_code_block() {
        let doc = parse_ok("```\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::CodeBlock { ref source, .. } if source.is_empty()));
    }

    // ── Code block with inner blank lines ──

    #[test]
    fn code_block_with_blank_lines() {
        let doc = parse_ok("```\nline 1\n\nline 3\n```");
        let node = &doc.content.children[0];
        if let NodeKind::CodeBlock { source, .. } = &node.kind {
            assert!(source.contains("line 1"));
            assert!(source.contains("line 3"));
        }
    }

    // ── Standalone image without alt = no caption ──

    #[test]
    fn figure_no_alt_promoted() {
        let doc = parse_ok("![](img.png)");
        let fig = &doc.content.children[0];
        assert!(matches!(fig.kind, NodeKind::Figure));
        // No alt text → no Caption child
        let has_caption = fig.children.iter().any(|c| matches!(c.kind, NodeKind::Caption));
        assert!(!has_caption, "empty alt should not create a caption");
    }

    // ── Hint with attribute selector ──

    #[test]
    fn hint_attr_selector() {
        use crate::ast::Value;
        use crate::visit::Pass;
        let doc = parse_ok("{@hint(code-block[lang=\"rust\"], theme=monokai)}");
        let mut doc = doc;
        crate::resolve::HoistDirectives.run(&mut doc);
        let hint = doc.content.attrs.inits.iter().find(|h| h.property == "theme");
        assert!(hint.is_some());
        assert!(matches!(&hint.unwrap().value, Value::Text(s) if s == "monokai"));
    }

    // ── Multiple hints on same directive ──

    #[test]
    fn multiple_hints_from_single_directive() {
        use crate::visit::Pass;
        let doc = parse_ok("{@hint(figure, series=figs, label=\"Figure {n}\")}");
        let mut doc = doc;
        crate::resolve::HoistDirectives.run(&mut doc);
        assert!(doc.content.attrs.inits.len() >= 2);
    }
}
