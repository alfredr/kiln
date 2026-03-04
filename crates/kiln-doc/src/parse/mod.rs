mod attrs;
mod block;
mod inline;
mod section;

use crate::ast::{Document, Metadata, Node, NodeKind};

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
    let img = children
        .into_iter()
        .next()
        .expect("is_single_image verified one child");
    let alt_text = if let NodeKind::Image { alt, .. } = &img.kind {
        alt.clone()
    } else {
        String::new()
    };
    let mut fig_children = vec![img];
    if !alt_text.is_empty() {
        fig_children.push(Node::caption(vec![Node::text(alt_text)]));
    }
    Node::figure(fig_children)
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
                let children = parse_inline(trimmed);
                if children.is_empty() {
                    return None;
                }
                // Figure promotion: paragraph with single image
                if is_single_image(&children) {
                    return Some(promote_to_figure(children));
                }
                // Check for trailing brace attrs on last child
                let mut children = children;
                maybe_extract_trailing_attrs_from_children(&mut children);
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
    fn empty_document() {
        let doc = parse_ok("");
        assert!(matches!(doc.content.kind, NodeKind::Section { level: 0 }));
        assert!(doc.content.children.is_empty());
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
    fn html_code_block_not_raw() {
        let doc = parse_ok("```html\n<div>hi</div>\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "html"));
    }

    #[test]
    fn raw_block_brace_format() {
        let doc = parse_ok("```{=html}\n<div>hi</div>\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::RawBlock { ref format, .. } if format == "html"));
    }

    #[test]
    fn raw_block_brace_format_typst() {
        let doc = parse_ok("```{=typst}\n#rect()\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::RawBlock { ref format, .. } if format == "typst"));
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
    fn tilde_code_fence() {
        let doc = parse_ok("~~~python\nprint('hi')\n~~~");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "python"));
    }

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
    fn paragraph_interrupted_by_heading() {
        let doc = parse_ok("text\n# Heading");
        // Root should have: paragraph, then section with heading
        assert!(matches!(doc.content.children[0].kind, NodeKind::Paragraph));
        assert!(matches!(
            doc.content.children[1].kind,
            NodeKind::Section { level: 1 }
        ));
    }

    #[test]
    fn paragraph_interrupted_by_fence() {
        let doc = parse_ok("text\n```\ncode\n```");
        assert!(matches!(doc.content.children[0].kind, NodeKind::Paragraph));
        assert!(matches!(
            doc.content.children[1].kind,
            NodeKind::CodeBlock { .. }
        ));
    }

    #[test]
    fn paragraph_interrupted_by_quote() {
        let doc = parse_ok("text\n> quote");
        assert!(matches!(doc.content.children[0].kind, NodeKind::Paragraph));
        assert!(matches!(doc.content.children[1].kind, NodeKind::BlockQuote));
    }

    #[test]
    fn paragraph_interrupted_by_list() {
        let doc = parse_ok("text\n- item");
        assert!(matches!(doc.content.children[0].kind, NodeKind::Paragraph));
        assert!(matches!(
            doc.content.children[1].kind,
            NodeKind::List { .. }
        ));
    }

    #[test]
    fn paragraph_interrupted_by_table() {
        let doc = parse_ok("text\n| A |\n|---|\n| 1 |");
        assert!(matches!(doc.content.children[0].kind, NodeKind::Paragraph));
        assert!(matches!(doc.content.children[1].kind, NodeKind::Table));
    }

    #[test]
    fn thematic_break_stars() {
        let doc = parse_ok("text\n\n***\n\nmore");
        assert!(
            doc.content
                .children
                .iter()
                .any(|n| matches!(n.kind, NodeKind::ThematicBreak))
        );
    }

    #[test]
    fn thematic_break_underscores() {
        let doc = parse_ok("text\n\n___\n\nmore");
        assert!(
            doc.content
                .children
                .iter()
                .any(|n| matches!(n.kind, NodeKind::ThematicBreak))
        );
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
            .any(|n| !n.attrs.hints.is_empty());
        assert!(has_hint);
    }

    #[test]
    fn series_display_roman() {
        use crate::ast::{Declaration, SeriesDisplay};
        let doc = parse_ok("{@series(nums, display=roman)}");
        let has_roman = doc.content.children.iter().any(|n| {
            n.attrs.declarations.iter().any(|d| matches!(d, Declaration::Series { config, .. } if matches!(config.display, SeriesDisplay::Roman)))
        });
        assert!(has_roman);
    }

    #[test]
    fn series_display_alpha() {
        use crate::ast::{Declaration, SeriesDisplay};
        let doc = parse_ok("{@series(lets, display=alpha)}");
        let has_alpha = doc.content.children.iter().any(|n| {
            n.attrs.declarations.iter().any(|d| matches!(d, Declaration::Series { config, .. } if matches!(config.display, SeriesDisplay::Alpha)))
        });
        assert!(has_alpha);
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
            .flat_map(|n| n.attrs.hints.iter())
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
            .flat_map(|n| n.attrs.hints.iter())
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
            .flat_map(|n| n.attrs.hints.iter())
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
            .flat_map(|n| n.attrs.hints.iter())
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
            .flat_map(|n| n.attrs.hints.iter())
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
            .flat_map(|n| n.attrs.hints.iter())
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
            .flat_map(|n| n.attrs.hints.iter())
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
            .flat_map(|n| n.attrs.hints.iter())
            .collect();
        assert!(hints.is_empty());
    }

    #[test]
    fn hint_this_selector() {
        use crate::ast::Selector;
        let doc = parse_ok("# Title {@hint(this, color=red)}");
        fn find_hint(node: &Node) -> Option<&crate::ast::Hint> {
            node.attrs
                .hints
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
            .flat_map(|n| n.attrs.hints.iter())
            .collect();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "theme");
    }

    #[test]
    fn series_display_hierarchical() {
        use crate::ast::{Declaration, SeriesDisplay};
        let doc = parse_ok("{@series(eq, display=hierarchical)}");
        let is_hierarchical = |d: &Declaration| matches!(d, Declaration::Series { config, .. } if matches!(config.display, SeriesDisplay::Hierarchical));
        let has = doc
            .content
            .children
            .iter()
            .any(|n| n.attrs.declarations.iter().any(is_hierarchical));
        assert!(has);
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
    fn code_block_old_form_normalizes() {
        // Old `lang {attrs}` form normalizes to unified brace form
        let doc = parse_ok("```rust {#my-id}\ncode\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "rust"));
        assert_eq!(code.attrs.id, "my-id");
    }

    #[test]
    fn code_block_bare_lang() {
        // Bare lang without braces still works
        let doc = parse_ok("```python\nprint('hi')\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "python"));
    }

    #[test]
    fn code_block_brace_lang_only() {
        // {lang} form with no extra attrs
        let doc = parse_ok("```{rust}\nfn main() {}\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang == "rust"));
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
    fn raw_block_colon_shorthand_with_class() {
        let doc = parse_ok("```{=typst:#fig:x .wide}\n#rect()\n```");
        let node = &doc.content.children[0];
        assert!(matches!(node.kind, NodeKind::RawBlock { ref format, .. } if format == "typst"));
        assert_eq!(node.attrs.id, "fig:x");
        assert!(node.attrs.classes.contains(&"wide".to_string()));
    }

    #[test]
    fn code_block_no_lang() {
        let doc = parse_ok("```\nplain text\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang.is_empty()));
    }

    #[test]
    fn code_block_attrs_only_no_lang() {
        // {#id .class} with no language word
        let doc = parse_ok("```{#my-id .highlight}\ncode\n```");
        let code = &doc.content.children[0];
        assert!(matches!(code.kind, NodeKind::CodeBlock { ref lang, .. } if lang.is_empty()));
        assert_eq!(code.attrs.id, "my-id");
        assert!(code.attrs.classes.contains(&"highlight".to_string()));
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
}
