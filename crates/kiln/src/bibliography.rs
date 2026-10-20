//! A site-wide bibliography: `[@key]` citations resolved against a BibTeX
//! file and formatted, with the list of works a page cites, in a CSL style.

use std::path::Path;

use anyhow::{Context, bail};
use hayagriva::archive::{ArchivedStyle, locales};
use hayagriva::citationberg::{
    FontStyle, FontVariant, FontWeight, IndependentStyle, Locale, Style, TextDecoration,
    VerticalAlign,
};
use hayagriva::{
    BibliographyDriver, BibliographyRequest, CitationItem, CitationRequest, Elem, ElemChild,
    ElemChildren, ElemMeta, Entry, Formatted, Library, RenderedBibliography,
};
use kiln_doc::ast::{Document, Node, NodeKind, Value};
use kiln_doc::html::{escape_attr, escape_html};

use crate::config::BibliographyConfig;

pub struct Bibliography {
    library: Library,
    style: IndependentStyle,
    locales: Vec<Locale>,
}

impl Bibliography {
    /// Load the bibliography that `kiln.toml` names, if it names one.
    pub fn load(config: &BibliographyConfig, root: &Path) -> anyhow::Result<Option<Self>> {
        let Some(file) = &config.file else {
            return Ok(None);
        };
        let path = root.join(file);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read bibliography {}", path.display()))?;
        let bibliography = Self::parse(&text, &config.style)
            .with_context(|| format!("in bibliography {}", path.display()))?;
        Ok(Some(bibliography))
    }

    fn parse(bibtex: &str, style: &str) -> anyhow::Result<Self> {
        let library = hayagriva::io::from_biblatex_str(bibtex).map_err(|errors| {
            let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            anyhow::anyhow!("failed to parse BibTeX: {}", msgs.join("; "))
        })?;
        let style = match ArchivedStyle::by_name(style).map(ArchivedStyle::get) {
            Some(Style::Independent(style)) => style,
            _ => bail!("unknown citation style: {style}"),
        };
        Ok(Self { library, style, locales: locales() })
    }

    /// A request for one citation of the works with these keys, all known.
    fn request(&self, keys: &[&str]) -> CitationRequest<'_, Entry> {
        let items = keys
            .iter()
            .filter_map(|key| self.library.get(key))
            .map(CitationItem::with_entry)
            .collect();
        CitationRequest::from_items(items, &self.style, &self.locales)
    }
}

/// Pass that formats each run of adjacent citations, such as `[@a; @b]`, as
/// one citation, and stores the list of works the page cites on the document
/// root as the `bibliography` property. A key the bibliography does not have
/// is reported and left as it is.
pub struct ResolveCitations<'a> {
    pub bibliography: &'a Bibliography,
}

impl kiln_doc::visit::Pass for ResolveCitations<'_> {
    fn run(&mut self, doc: &mut Document) {
        let bib = self.bibliography;
        let mut runs = Vec::new();
        collect_runs(&doc.content, &mut runs);
        if runs.is_empty() {
            return;
        }

        // Every citation goes to the driver before any is rendered, so styles
        // can number works and tell apart works with the same author and year.
        let cited: Vec<Vec<&str>> = runs.iter().map(|run| known_keys(&bib.library, run)).collect();
        let mut driver = BibliographyDriver::new();
        for keys in cited.iter().filter(|keys| !keys.is_empty()) {
            driver.citation(bib.request(keys));
        }
        let rendered = driver.finish(BibliographyRequest::new(&bib.style, None, &bib.locales));

        // Runs with no known keys were not requested and have no citation.
        let mut citations = rendered.citations.iter();
        let mut formatted = cited.iter().map(|keys| match keys.as_slice() {
            [] => None,
            keys => citations.next().map(|citation| to_html(&citation.citation, keys)),
        });
        assign_runs(&mut doc.content, &bib.library, &mut formatted);

        if let Some(list) = rendered.bibliography {
            doc.content
                .attrs
                .properties
                .insert("bibliography".into(), Value::Text(list_html(&list)));
        }
    }
}

/// The keys of a run that the bibliography has, reporting the others.
fn known_keys<'k>(library: &Library, run: &'k [String]) -> Vec<&'k str> {
    let (known, unknown): (Vec<&str>, Vec<&str>) = run
        .iter()
        .map(String::as_str)
        .partition(|key| library.get(key).is_some());
    for key in unknown {
        eprintln!("    warning: no bibliography entry for citation key: {key}");
    }
    known
}

/// The keys of each run of adjacent citations, in document order.
fn collect_runs(node: &Node, runs: &mut Vec<Vec<String>>) {
    let mut i = 0;
    while i < node.children.len() {
        let run: Vec<String> = node.children[i..]
            .iter()
            .map_while(|child| match &child.kind {
                NodeKind::Citation { key } => Some(key.clone()),
                _ => None,
            })
            .collect();
        if run.is_empty() {
            collect_runs(&node.children[i], runs);
            i += 1;
        } else {
            i += run.len();
            runs.push(run);
        }
    }
}

/// Store each run's formatted citation on its first known citation, in the
/// same order as `collect_runs`, and leave the run's other known citations
/// empty.
fn assign_runs(
    node: &mut Node,
    library: &Library,
    formatted: &mut impl Iterator<Item = Option<String>>,
) {
    let mut i = 0;
    while i < node.children.len() {
        let len = node.children[i..]
            .iter()
            .take_while(|child| matches!(child.kind, NodeKind::Citation { .. }))
            .count();
        if len == 0 {
            assign_runs(&mut node.children[i], library, formatted);
            i += 1;
            continue;
        }
        let html = formatted.next().flatten();
        assign_run(&mut node.children[i..i + len], library, html);
        i += len;
    }
}

/// Store a run's formatted citation on its first known citation and leave the
/// run's other known citations empty.
fn assign_run(run: &mut [Node], library: &Library, mut html: Option<String>) {
    let known = run.iter_mut().filter(|citation| match &citation.kind {
        NodeKind::Citation { key } => library.get(key).is_some(),
        _ => false,
    });
    for citation in known {
        let text = html.take().unwrap_or_default();
        citation.attrs.properties.insert("compiled".into(), Value::Text(text));
    }
}

/// The list of cited works, each with an `id` that its citations link to.
fn list_html(list: &RenderedBibliography) -> String {
    let mut out = String::from(if list.hanging_indent {
        "<ul class=\"references hanging\">"
    } else {
        "<ul class=\"references\">"
    });
    for item in &list.items {
        out.push_str(&format!("<li id=\"ref-{}\">", escape_attr(&item.key)));
        if let Some(label) = &item.first_field {
            out.push_str("<span class=\"label\">");
            write_html(&ElemChildren(vec![label.clone()]), &[], None, &mut out);
            out.push_str("</span> ");
        }
        write_html(&item.content, &[], None, &mut out);
        out.push_str("</li>");
    }
    out.push_str("</ul>");
    out
}

fn to_html(children: &ElemChildren, keys: &[&str]) -> String {
    let mut out = String::new();
    write_html(children, keys, None, &mut out);
    out
}

/// Write formatted citation or bibliography content as HTML. Hayagriva has an
/// HTML writer, but it does not escape text. An item of a citation links to
/// the work it cites, `keys[i]` for the citation's `i`th work; `number` is
/// the key whose citation number, once reached, is the link.
fn write_html(children: &ElemChildren, keys: &[&str], number: Option<&str>, out: &mut String) {
    for child in &children.0 {
        match child {
            ElemChild::Text(text) => write_formatted(text, out),
            ElemChild::Markup(markup) => out.push_str(&escape_html(markup)),
            ElemChild::Link { text, url } => {
                out.push_str(&format!("<a href=\"{}\">", escape_attr(url)));
                write_formatted(text, out);
                out.push_str("</a>");
            }
            ElemChild::Elem(elem) => write_elem(elem, keys, number, out),
            ElemChild::Transparent { .. } => {}
        }
    }
}

/// Write an element. An item of a citation links to the work it cites: only
/// its number when the style prints one, so brackets around the number stay
/// plain text, and otherwise the whole item.
fn write_elem(elem: &Elem, keys: &[&str], number: Option<&str>, out: &mut String) {
    let entry = match elem.meta {
        Some(ElemMeta::Entry(i)) => keys.get(i).copied(),
        _ => None,
    };
    let is_number = elem.meta == Some(ElemMeta::CitationNumber);
    match (entry, number) {
        (Some(key), _) if has_number(elem) => write_html(&elem.children, keys, Some(key), out),
        (Some(key), _) => write_link(key, &elem.children, keys, out),
        (None, Some(key)) if is_number => write_link(key, &elem.children, keys, out),
        _ => write_html(&elem.children, keys, number, out),
    }
}

fn write_link(key: &str, children: &ElemChildren, keys: &[&str], out: &mut String) {
    out.push_str(&format!("<a href=\"#ref-{}\">", escape_attr(key)));
    write_html(children, keys, None, out);
    out.push_str("</a>");
}

/// Whether an element holds a citation number.
fn has_number(elem: &Elem) -> bool {
    elem.children.0.iter().any(|child| match child {
        ElemChild::Elem(inner) => inner.meta == Some(ElemMeta::CitationNumber) || has_number(inner),
        _ => false,
    })
}

fn write_formatted(text: &Formatted, out: &mut String) {
    let f = &text.formatting;
    let tags: Vec<(&str, &str)> = [
        (f.font_style == FontStyle::Italic, ("<i>", "</i>")),
        (f.font_weight == FontWeight::Bold, ("<b>", "</b>")),
        (
            f.font_variant == FontVariant::SmallCaps,
            ("<span style=\"font-variant:small-caps\">", "</span>"),
        ),
        (f.text_decoration == TextDecoration::Underline, ("<u>", "</u>")),
        (f.vertical_align == VerticalAlign::Sup, ("<sup>", "</sup>")),
        (f.vertical_align == VerticalAlign::Sub, ("<sub>", "</sub>")),
    ]
    .into_iter()
    .filter_map(|(on, tags)| on.then_some(tags))
    .collect();
    for (open, _) in &tags {
        out.push_str(open);
    }
    out.push_str(&escape_html(&text.text));
    for (_, close) in tags.iter().rev() {
        out.push_str(close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_doc::visit::Pass;

    const BIBTEX: &str = r#"
@article{naor,
  author = {Dwork, Cynthia and Naor, Moni},
  title = {On the Difficulties of Disclosure Prevention},
  journal = {Journal of Privacy \& Confidentiality},
  year = {2010},
}
@book{cover,
  author = {Cover, Thomas M. and Thomas, Joy A.},
  title = {Elements of Information Theory},
  publisher = {Wiley},
  year = {2006},
}
@book{unused,
  author = {Nobody, A.},
  title = {Never Cited},
  publisher = {Nowhere},
  year = {2000},
}
"#;

    fn resolve(markdown: &str, style: &str) -> Document {
        let bibliography = Bibliography::parse(BIBTEX, style).unwrap();
        let mut doc = kiln_doc::parse::parse(markdown).unwrap();
        ResolveCitations { bibliography: &bibliography }.run(&mut doc);
        doc
    }

    fn body(doc: &Document) -> String {
        kiln_doc::html::render_html(doc)
    }

    fn list(doc: &Document) -> String {
        match doc.content.attrs.properties.get("bibliography") {
            Some(Value::Text(html)) => html.clone(),
            _ => String::new(),
        }
    }

    #[test]
    fn citations_are_numbered_and_link_to_their_works() {
        // Only the number is the link; the brackets around it are not.
        let doc = resolve("First [@naor], then [@cover], then [@naor] again.", "ieee");
        let html = body(&doc);
        assert!(html.contains("<cite data-key=\"naor\">[<a href=\"#ref-naor\">1</a>]</cite>"), "{html}");
        assert!(html.contains("<cite data-key=\"cover\">[<a href=\"#ref-cover\">2</a>]</cite>"), "{html}");
    }

    #[test]
    fn a_run_of_citations_is_one_citation() {
        let doc = resolve("Both [@naor; @cover] agree.", "ieee");
        let html = body(&doc);
        assert_eq!(html.matches("<cite").count(), 1, "{html}");
        assert!(html.contains("[<a href=\"#ref-naor\">1</a>], [<a href=\"#ref-cover\">2</a>]"), "{html}");
    }

    #[test]
    fn without_a_number_the_whole_item_is_the_link() {
        let doc = resolve("See [@naor].", "apa");
        let html = body(&doc);
        assert!(html.contains("<a href=\"#ref-naor\">Dwork"), "{html}");
    }

    #[test]
    fn the_list_has_only_cited_works_with_escaped_text() {
        let doc = resolve("See [@naor].", "ieee");
        let list = list(&doc);
        assert!(list.contains("<li id=\"ref-naor\">"), "{list}");
        assert!(list.contains("Privacy &amp; Confidentiality"), "{list}");
        assert!(!list.contains("ref-cover") && !list.contains("Never Cited"), "{list}");
    }

    #[test]
    fn author_date_styles_work_too() {
        let doc = resolve("See [@naor].", "apa");
        let html = body(&doc);
        assert!(html.contains("Dwork") && html.contains("2010"), "{html}");
    }

    #[test]
    fn unknown_keys_stay_as_they_are() {
        let doc = resolve("See [@naor; @missing].", "ieee");
        let html = body(&doc);
        assert!(html.contains("<cite data-key=\"missing\">[missing]</cite>"), "{html}");
        assert!(html.contains("#ref-naor"), "{html}");
    }

    #[test]
    fn a_page_without_citations_has_no_list() {
        let doc = resolve("No citations here.", "ieee");
        assert!(list(&doc).is_empty());
    }

    #[test]
    fn unknown_style_is_an_error() {
        assert!(Bibliography::parse(BIBTEX, "no-such-style").is_err());
    }
}

