use std::path::Path;

use anyhow::Context;
use typst::foundations::Label;
use typst::layout::{Abs, PagedDocument};
use typst::utils::PicoStr;
use typst_html::HtmlDocument;

use kiln_doc::ast::{
    Bind, CompoundSelector, Declaration, Document, Init, Placement, Selector, SelectorExpr, Value,
};

use crate::config::{Config, TypstConfig};
use crate::content::{ContentKind, ContentNode, ContentTree, discover_content};
use crate::template::{load_templates, render_index, render_page, render_post};
use crate::world::KilnWorld;

const BUILTIN_PRELUDE: &str = include_str!("prelude.kiln");

/// Typst's default text size, in points. SVG output is sized in em against it,
/// so Typst text at its default size matches the size of the text around it.
const TYPST_EM_PT: f64 = 11.0;

/// Default text color for SVG output, rewritten to `currentColor` so output
/// takes the color of the text around it. Explicit colors are left alone.
const INK: &str = "#010203";

/// Label of a marker placed where inline content starts. Typst positions it on
/// the line's baseline, which is how inline SVG lines up with the text around it.
const BASELINE_LABEL: &str = "kiln-baseline";

/// A RawBlockCompiler that compiles Typst blocks using KilnWorld.
struct TypstCompiler<'a> {
    project_root: &'a Path,
    typst_config: &'a TypstConfig,
}

impl kiln_doc::ast::RawBlockCompiler for TypstCompiler<'_> {
    fn compile(&self, format: &str, source: &str, placement: Placement) -> Result<String, String> {
        // Passthrough formats
        match format {
            "html" => return Ok(source.to_string()),
            "mermaid" => {
                return mermaid_rs_renderer::render(source)
                    .map_err(|e| format!("mermaid rendering failed: {e}"));
            }
            _ => {}
        }

        // Resolve output mode, preamble and content (templated if an alias).
        let (output_mode, preamble, content): (&str, &str, String) = match format {
            "typst" | "typst-html" => (format, "", source.to_string()),
            _ => match self.typst_config.formats.iter().find(|f| f.name == format) {
                Some(alias) => (
                    alias.output.as_str(),
                    alias.preamble.as_str(),
                    alias.template.replace("{source}", source),
                ),
                None => return Err(format!("unsupported raw block format: {format}")),
            },
        };

        // Kiln's page and text rules come first so a preamble can override
        // them. The baseline marker goes directly before the content, with no
        // space between them.
        let mut wrapped = String::from("#set page(width: auto, height: auto, margin: 0pt)\n");
        if output_mode != "typst-html" {
            wrapped.push_str(&format!("#set page(fill: none)\n#set text(fill: rgb(\"{INK}\"))\n"));
        }
        if placement == Placement::Inline {
            wrapped.push_str("#set text(top-edge: \"bounds\", bottom-edge: \"bounds\")\n");
        }
        if !preamble.is_empty() {
            wrapped.push_str(preamble);
            wrapped.push('\n');
        }
        if placement == Placement::Inline {
            // The zero-width `h` keeps the marker inside the paragraph: one at
            // the very start is moved out of it, to the top of the page.
            wrapped.push_str(&format!("#h(0pt)#metadata(none)<{BASELINE_LABEL}>"));
        }
        wrapped.push_str(&content);

        let world = KilnWorld::from_source(&wrapped, self.project_root)
            .map_err(|e| format!("failed to create typst world: {e}"))?;

        if output_mode == "typst-html" {
            return self.compile_typst_html(&world);
        }

        let typst::diag::Warned { output, warnings } = typst::compile::<PagedDocument>(&world);

        for warning in &warnings {
            eprintln!("    typst block warn: {warning:?}");
        }

        let document = output.map_err(|errs: ecow::EcoVec<typst::diag::SourceDiagnostic>| {
            let msgs: Vec<String> = errs.iter().map(|e| format!("{e:?}")).collect();
            let msg = format!("typst compilation failed: {}", msgs.join("; "));
            eprintln!("    typst block error: {msg}");
            msg
        })?;

        let svg = match document.pages.as_slice() {
            [page] => {
                let em = Abs::pt(TYPST_EM_PT);
                let svg = typst_svg::svg_html_frame(&page.frame, em, None, &[], &document.introspector);
                match placement {
                    Placement::Inline => align_baseline(svg, &document, page.frame.height()),
                    Placement::Block => svg,
                }
            }
            // Content that breaks across pages keeps its pages stacked.
            _ => typst_svg::svg_merged(&document, Abs::zero()),
        };
        Ok(svg.replace(&format!("\"{INK}\""), "\"currentColor\""))
    }
}

/// Shift inline SVG down by the depth of its content below the baseline, so the
/// content's baseline sits on the baseline of the text around it.
fn align_baseline(svg: String, document: &PagedDocument, height: Abs) -> String {
    let label = Label::new(PicoStr::intern(BASELINE_LABEL)).expect("label is not empty");
    let Some(location) = document.introspector.query_label(label).ok().and_then(|c| c.location())
    else {
        return svg;
    };
    let baseline = document.introspector.position(location).point.y;
    let depth = (height - baseline) / Abs::pt(TYPST_EM_PT);
    svg.replacen(
        "overflow: visible;",
        &format!("overflow: visible; vertical-align: {}em;", -depth),
        1,
    )
}

impl TypstCompiler<'_> {
    fn compile_typst_html(&self, world: &KilnWorld) -> Result<String, String> {
        let typst::diag::Warned { output, warnings } = typst::compile::<HtmlDocument>(world);

        for warning in &warnings {
            eprintln!("    typst block warn: {warning:?}");
        }

        let document = output.map_err(|errs: ecow::EcoVec<typst::diag::SourceDiagnostic>| {
            let msgs: Vec<String> = errs.iter().map(|e| format!("{e:?}")).collect();
            format!("typst compilation failed: {}", msgs.join("; "))
        })?;

        let raw_html = typst_html::html(&document).map_err(|errs| {
            let msgs: Vec<String> = errs.iter().map(|e| format!("{e:?}")).collect();
            format!("typst html encoding failed: {}", msgs.join("; "))
        })?;

        // Extract content between <body> and </body>
        let body_start = raw_html.find("<body>").map(|i| i + 6);
        let body_end = raw_html.rfind("</body>");
        Ok(match (body_start, body_end) {
            (Some(start), Some(end)) if start < end => raw_html[start..end].trim().to_string(),
            _ => raw_html,
        })
    }
}

pub fn build(project_root: &Path, config: &Config, include_drafts: bool) -> anyhow::Result<()> {
    let content_dir = project_root.join(&config.build.content_dir);
    let output_dir = project_root.join(&config.build.output_dir);
    let template_dir = project_root.join(&config.build.template_dir);
    let static_dir = project_root.join(&config.build.static_dir);

    let slug_re = config
        .build
        .slug_pattern
        .as_deref()
        .map(regex::Regex::new)
        .transpose()
        .context("invalid slug_pattern regex in kiln.toml")?;

    let tree = discover_content(&content_dir, include_drafts, slug_re.as_ref())?;

    let data_dir = project_root.join(&config.build.data_dir);
    let tera = load_templates(&template_dir, &data_dir, &static_dir)?;

    // Clean and recreate output directory
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir)?;
    }
    std::fs::create_dir_all(&output_dir)?;

    // Copy static files
    if static_dir.exists() {
        copy_dir_recursive(&static_dir, &output_dir)?;
    }

    let compiler = TypstCompiler { project_root, typst_config: &config.typst };

    // Load prelude (built-in or custom) and merge with config inits
    let prelude_text = match config.build.prelude.as_deref() {
        None => BUILTIN_PRELUDE.to_string(),
        Some(path) => std::fs::read_to_string(project_root.join(path))
            .with_context(|| format!("failed to read prelude: {path}"))?,
    };
    let (declarations, mut inits) = kiln_doc::parse_prelude(&prelude_text);
    inits.extend(config_inits(&config.hints));

    let prelude = Prelude { inits, declarations };

    // Compile the content tree
    compile_tree(
        config,
        &tera,
        &tree.root,
        &output_dir,
        &compiler,
        &tree,
        &prelude,
    )?;

    // Generate index page
    let index_html = render_index(&tera, config, &tree)?;
    std::fs::write(output_dir.join("index.html"), &index_html)?;

    eprintln!(
        "built {} post(s), {} page(s) → {}",
        tree.posts.len(),
        tree.pages.len(),
        output_dir.display()
    );
    Ok(())
}

struct Prelude {
    inits: Vec<Init>,
    declarations: Vec<Declaration>,
}

fn compile_tree(
    config: &Config,
    tera: &tera::Tera,
    node: &ContentNode,
    output_dir: &Path,
    compiler: &TypstCompiler,
    tree: &ContentTree,
    prelude: &Prelude,
) -> anyhow::Result<()> {
    // Collect child content directory names for asset isolation
    let child_dirs: Vec<&str> = node
        .children
        .iter()
        .filter_map(|c| match &c.kind {
            ContentKind::Page(p) => p.source_dir.file_name().and_then(|n| n.to_str()),
            ContentKind::Post(p) => p.source_dir.file_name().and_then(|n| n.to_str()),
            ContentKind::Virtual { slug } => Some(slug.as_str()),
        })
        .collect();

    let item = match &node.kind {
        ContentKind::Post(item) | ContentKind::Page(item) => Some(item),
        ContentKind::Virtual { .. } => None,
    };

    if let Some(item) = item {
        let is_post = matches!(node.kind, ContentKind::Post(_));
        let label = if is_post { "post" } else { "page" };
        eprintln!("  compiling {label}: {}", item.path);

        let dest = if is_post {
            output_dir.join("posts").join(&item.path)
        } else {
            output_dir.join(&item.path)
        };

        let mut inject = InjectPrelude {
            inits: prelude.inits.clone(),
            declarations: prelude.declarations.clone(),
        };
        let mut hoist = kiln_doc::resolve::HoistDirectives;
        let mut cascade = kiln_doc::resolve::CascadeInits;
        let mut execute = kiln_doc::resolve::ExecuteProgram;
        let mut cross_refs = kiln_doc::resolve::ResolveCrossRefs;
        let mut compile = kiln_doc::resolve::CompileBlocks::new(compiler);
        let passes: &mut [&mut dyn kiln_doc::visit::Pass] = &mut [
            &mut inject,
            &mut hoist,
            &mut cascade,
            &mut execute,
            &mut cross_refs,
            &mut compile,
        ];

        let render_fn = if is_post { render_post } else { render_page };

        compile_content(
            &item.md_path,
            &item.source_dir,
            &dest,
            &child_dirs,
            passes,
            |body| render_fn(tera, config, item, body, tree),
        )?;
    }

    for child in &node.children {
        compile_tree(config, tera, child, output_dir, compiler, tree, prelude)?;
    }

    Ok(())
}

fn config_inits(configs: &[crate::config::HintConfig]) -> Vec<Init> {
    configs
        .iter()
        .filter_map(|cfg| {
            let selector = selector_from_hint(cfg)?;
            Some(cfg.properties.iter().map(move |(k, v)| Init {
                selector: selector.clone(),
                property: k.clone(),
                value: Bind::Val(match v {
                    toml::Value::Boolean(b) => Value::Bool(*b),
                    toml::Value::Integer(n) => Value::Number(*n as f64),
                    toml::Value::Float(n) => Value::Number(*n),
                    toml::Value::String(s) => Value::Text(s.clone()),
                    other => Value::Text(other.to_string()),
                }),
            }))
        })
        .flatten()
        .collect()
}

fn selector_from_hint(cfg: &crate::config::HintConfig) -> Option<Selector> {

    if let Some(sel_str) = &cfg.selector {
        let sel = kiln_doc::selector::parse_selector(sel_str);
        if sel.is_none() {
            eprintln!("  warning: invalid selector in config: {sel_str}");
        }
        return sel;
    }
    if let Some(k) = &cfg.kind {
        let pascal = kiln_doc::selector::resolve_kind_name(k).unwrap_or(k.as_str());
        return Some(Selector::Expr(SelectorExpr::Simple(CompoundSelector {
            kind: Some(pascal.to_string()),
            ..Default::default()
        })));
    }
    if let Some(c) = &cfg.class {
        return Some(Selector::Expr(SelectorExpr::Simple(CompoundSelector {
            classes: vec![c.clone()],
            ..Default::default()
        })));
    }
    cfg.id.as_ref().map(|id| {
        Selector::Expr(SelectorExpr::Simple(CompoundSelector {
            id: Some(id.clone()),
            ..Default::default()
        }))
    })
}

/// Pass that injects prelude declarations and config-level inits onto the document root.
struct InjectPrelude {
    inits: Vec<Init>,
    declarations: Vec<Declaration>,
}

impl kiln_doc::visit::Pass for InjectPrelude {
    fn run(&mut self, doc: &mut Document) {
        doc.content.attrs.inits.extend(self.inits.iter().cloned());
        doc.content
            .attrs
            .declarations
            .extend(self.declarations.iter().cloned());
    }
}

fn compile_content(
    md_path: &Path,
    source_dir: &Path,
    dest_dir: &Path,
    skip_dirs: &[&str],
    passes: &mut [&mut dyn kiln_doc::visit::Pass],
    render_fn: impl FnOnce(&str) -> anyhow::Result<String>,
) -> anyhow::Result<()> {
    let markdown = std::fs::read_to_string(md_path)
        .with_context(|| format!("failed to read {}", md_path.display()))?;

    let mut doc = kiln_doc::parse::parse(&markdown)
        .map_err(|e| anyhow::anyhow!("parse error in '{}': {e}", md_path.display()))?;

    for pass in passes.iter_mut() {
        pass.run(&mut doc);
    }

    let body_html = kiln_doc::html::render_html(&doc);
    let full_html = render_fn(&body_html)?;

    std::fs::create_dir_all(dest_dir)?;
    std::fs::write(dest_dir.join("index.html"), &full_html)?;

    copy_assets(source_dir, dest_dir, skip_dirs)?;

    Ok(())
}

fn copy_dir_filtered(
    src: &Path,
    dst: &Path,
    include: impl Fn(&walkdir::DirEntry) -> bool,
) -> anyhow::Result<()> {
    for entry in walkdir::WalkDir::new(src)
        .into_iter()
        .filter_entry(|e| include(e))
    {
        let entry = entry?;
        let path = entry.path();
        if path == src {
            continue;
        }
        let relative = path.strip_prefix(src)?;
        let target = dst.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(path, &target)?;
    }
    Ok(())
}

fn copy_assets(source_dir: &Path, dest_dir: &Path, skip_dirs: &[&str]) -> anyhow::Result<()> {
    if !source_dir.is_dir() {
        return Ok(());
    }
    copy_dir_filtered(source_dir, dest_dir, |e| {
        let name = e.file_name().to_string_lossy();
        // Exclude child content directories
        if e.file_type().is_dir() && skip_dirs.contains(&name.as_ref()) {
            return false;
        }
        // Exclude markdown source files and metadata
        if name.ends_with(".md") || name == "meta.toml" {
            return false;
        }
        true
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    copy_dir_filtered(src, dst, |_| true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TypstFormatConfig;
    use kiln_doc::ast::RawBlockCompiler;

    static EMPTY_TYPST_CONFIG: TypstConfig = TypstConfig { formats: vec![] };

    fn compiler() -> TypstCompiler<'static> {
        TypstCompiler {
            project_root: Path::new("/tmp"),
            typst_config: &EMPTY_TYPST_CONFIG,
        }
    }

    #[test]
    fn typst_compiler_handles_html() {
        let c = compiler();
        let result = c.compile("html", "<div>hello</div>", Placement::Block);
        assert_eq!(result.unwrap(), "<div>hello</div>");
    }

    #[test]
    fn typst_compiler_handles_mermaid() {
        let c = compiler();
        let result = c.compile("mermaid", "flowchart LR\n    A --> B", Placement::Block);
        let svg = result.unwrap();
        assert!(svg.contains("<svg"), "mermaid should produce SVG output");
    }

    #[test]
    fn typst_compiler_rejects_unknown() {
        let c = compiler();
        let result = c.compile("latex", "\\frac{1}{2}", Placement::Block);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported"));
    }

    fn write_file(dir: &Path, relative: &str, content: &str) {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }

    #[test]
    fn copy_assets_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        write_file(&src, "style.css", "body {}");
        write_file(&src, "script.js", "console.log()");

        copy_assets(&src, &dst, &[]).unwrap();
        assert!(dst.join("style.css").exists());
        assert!(dst.join("script.js").exists());
    }

    #[test]
    fn copy_assets_skips_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        write_file(&src, "index.md", "# Hello");
        write_file(&src, "other.md", "# Other");
        write_file(&src, "image.png", "PNG");

        copy_assets(&src, &dst, &[]).unwrap();
        assert!(!dst.join("index.md").exists());
        assert!(!dst.join("other.md").exists());
        assert!(dst.join("image.png").exists());
    }

    #[test]
    fn copy_assets_skips_meta_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        write_file(&src, "meta.toml", "title = \"test\"");
        write_file(&src, "data.json", "{}");

        copy_assets(&src, &dst, &[]).unwrap();
        assert!(!dst.join("meta.toml").exists());
        assert!(dst.join("data.json").exists());
    }

    #[test]
    fn copy_assets_skips_child_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        write_file(&src, "image.png", "PNG");
        write_file(&src, "child-post/index.md", "# Child");
        write_file(&src, "child-post/photo.jpg", "JPG");

        copy_assets(&src, &dst, &["child-post"]).unwrap();
        assert!(dst.join("image.png").exists());
        assert!(!dst.join("child-post").exists());
    }

    #[test]
    fn copy_assets_empty_source() {
        let result = copy_assets(Path::new("/nonexistent"), Path::new("/tmp"), &[]);
        assert!(result.is_ok());
    }

    // ── config_inits / parse_config_selector tests ──

    fn make_hint_config(
        selector: Option<&str>,
        kind: Option<&str>,
        class: Option<&str>,
        id: Option<&str>,
        props: Vec<(&str, toml::Value)>,
    ) -> crate::config::HintConfig {
        crate::config::HintConfig {
            selector: selector.map(String::from),
            kind: kind.map(String::from),
            class: class.map(String::from),
            id: id.map(String::from),
            properties: props.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        }
    }

    #[test]
    fn config_selector_field() {
        let cfg = make_hint_config(
            Some("code-block[lang=\"rust\"]"),
            None,
            None,
            None,
            vec![("theme", toml::Value::String("dark".into()))],
        );
        let hints = config_inits(&[cfg]);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "theme");
    }

    #[test]
    fn config_kind_field() {
        let cfg = make_hint_config(
            None,
            Some("heading"),
            None,
            None,
            vec![("numbering", toml::Value::Boolean(true))],
        );
        let hints = config_inits(&[cfg]);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "numbering");
        assert!(matches!(hints[0].value.val(), kiln_doc::ast::Value::Bool(true)));
    }

    #[test]
    fn config_class_field() {
        let cfg = make_hint_config(
            None,
            None,
            Some("wide"),
            None,
            vec![("layout", toml::Value::String("full".into()))],
        );
        let hints = config_inits(&[cfg]);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "layout");
    }

    #[test]
    fn config_id_field() {
        let cfg = make_hint_config(
            None,
            None,
            None,
            Some("fig-main"),
            vec![("position", toml::Value::String("top".into()))],
        );
        let hints = config_inits(&[cfg]);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "position");
    }

    #[test]
    fn config_no_selector_produces_empty() {
        let cfg = make_hint_config(
            None,
            None,
            None,
            None,
            vec![("x", toml::Value::String("y".into()))],
        );
        let hints = config_inits(&[cfg]);
        assert!(hints.is_empty());
    }

    #[test]
    fn config_invalid_selector_produces_empty() {
        let cfg = make_hint_config(
            Some("nonexistent_kind"),
            None,
            None,
            None,
            vec![("x", toml::Value::String("y".into()))],
        );
        let hints = config_inits(&[cfg]);
        assert!(hints.is_empty());
    }

    #[test]
    fn config_value_types() {
        use kiln_doc::ast::Value;
        let cfg = make_hint_config(
            Some("heading"),
            None,
            None,
            None,
            vec![
                ("flag", toml::Value::Boolean(false)),
                ("count", toml::Value::Integer(42)),
                ("ratio", toml::Value::Float(1.23)),
                ("label", toml::Value::String("hello".into())),
            ],
        );
        let hints = config_inits(&[cfg]);
        assert_eq!(hints.len(), 4);

        let find = |name: &str| hints.iter().find(|h| h.property == name).unwrap();
        assert!(matches!(find("flag").value.val(), Value::Bool(false)));
        assert!(matches!(find("count").value.val(), Value::Number(n) if *n == 42.0));
        assert!(matches!(find("ratio").value.val(), Value::Number(n) if (*n - 1.23).abs() < 0.001));
        assert!(matches!(find("label").value.val(), Value::Text(s) if s == "hello"));
    }

    #[test]
    fn config_multiple_hints() {
        let cfg1 = make_hint_config(
            Some("heading"),
            None,
            None,
            None,
            vec![("a", toml::Value::Boolean(true))],
        );
        let cfg2 = make_hint_config(
            Some("paragraph"),
            None,
            None,
            None,
            vec![("b", toml::Value::Boolean(true))],
        );
        let hints = config_inits(&[cfg1, cfg2]);
        assert_eq!(hints.len(), 2);
    }

    fn compiler_with_alias(name: &str, preamble: &str, output: &str) -> TypstCompiler<'static> {
        compiler_with_template(name, preamble, output, "{source}")
    }

    fn compiler_with_template(
        name: &str,
        preamble: &str,
        output: &str,
        template: &str,
    ) -> TypstCompiler<'static> {
        let config = Box::leak(Box::new(TypstConfig {
            formats: vec![TypstFormatConfig {
                name: name.to_string(),
                output: output.to_string(),
                preamble: preamble.to_string(),
                template: template.to_string(),
            }],
        }));
        TypstCompiler { project_root: Path::new("/tmp"), typst_config: config }
    }

    /// Read an em length such as `width: 0.52em;` from an SVG's style.
    fn style_em(svg: &str, property: &str) -> Option<f64> {
        let start = svg.find(&format!("{property}: "))? + property.len() + 2;
        let end = start + svg[start..].find("em;")?;
        svg[start..end].parse().ok()
    }

    fn inline_math(preamble: &str, source: &str) -> String {
        compiler_with_template("math", preamble, "typst", "${source}$")
            .compile("math", source, Placement::Inline)
            .unwrap()
    }

    #[test]
    fn typst_inline_sits_on_the_baseline() {
        // `x` has no descender, `y` does: only `y` drops below the baseline.
        let x = style_em(&inline_math("", "x"), "vertical-align").unwrap();
        let y = style_em(&inline_math("", "y"), "vertical-align").unwrap();
        assert!(x.abs() < 0.02, "x should sit on the baseline: {x}");
        assert!(y < -0.1, "y should drop below the baseline: {y}");
    }

    #[test]
    fn typst_inline_is_sized_in_em() {
        let svg = inline_math("", "x");
        let width = style_em(&svg, "width").unwrap();
        let height = style_em(&svg, "height").unwrap();
        assert!(width > 0.3 && width < 0.8, "x is about half an em wide: {width}");
        assert!(height > 0.3 && height < 0.6, "x is about an x-height tall: {height}");
    }

    #[test]
    fn typst_preamble_adds_no_width() {
        let bare = style_em(&inline_math("", "x"), "width").unwrap();
        let with_preamble = style_em(&inline_math("#let a = 1", "x"), "width").unwrap();
        assert!((bare - with_preamble).abs() < 1e-9, "{bare} vs {with_preamble}");
    }

    #[test]
    fn typst_output_takes_the_text_color() {
        let svg = inline_math("", "sqrt(x)");
        assert!(svg.contains("\"currentColor\""));
        assert!(!svg.contains(INK));
        assert!(!svg.contains("#ffffff"), "no page background");
    }

    #[test]
    fn typst_explicit_color_is_kept() {
        let svg = inline_math("", "#text(fill: rgb(\"#ff0000\"))[x]");
        assert!(svg.contains("#ff0000"));
        assert!(!svg.contains("currentColor"));
    }

    #[test]
    fn typst_block_is_sized_in_em_without_baseline_shift() {
        let svg = compiler().compile("typst", "$ x $", Placement::Block).unwrap();
        assert!(style_em(&svg, "width").is_some());
        assert!(style_em(&svg, "vertical-align").is_none());
    }

    #[test]
    fn typst_template_wraps_source() {
        let c = compiler_with_template("math", "", "typst", "${source}$");
        assert!(c.compile("math", "x^2", Placement::Inline).is_ok());
        // Without the template, `x^2` is markup and `^` is not math.
        let plain = compiler_with_alias("plain", "", "typst");
        let svg = plain.compile("plain", "x^2", Placement::Inline).unwrap();
        assert_ne!(svg, c.compile("math", "x^2", Placement::Inline).unwrap());
    }

    #[test]
    fn typst_error_is_reported() {
        let err = inline_math_err("frac(");
        assert!(err.contains("typst compilation failed"), "{err}");
    }

    fn inline_math_err(source: &str) -> String {
        compiler_with_template("math", "", "typst", "${source}$")
            .compile("math", source, Placement::Inline)
            .unwrap_err()
    }

    #[test]
    fn typst_compiler_rejects_unknown_with_no_aliases() {
        let c = compiler();
        let result = c.compile("latex", "\\frac{1}{2}", Placement::Block);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported"));
    }

    #[test]
    fn typst_compiler_named_alias_not_rejected_as_unsupported() {
        // "feynman" alias should not be rejected as unsupported format.
        let c = compiler_with_alias("feynman", "", "typst");
        let result = c.compile("feynman", "#rect()", Placement::Block);
        if let Err(e) = &result {
            assert!(!e.contains("unsupported"), "alias should not be unsupported: {e}");
        }
    }

    #[test]
    fn typst_compiler_unknown_with_alias_present_still_rejects_others() {
        let c = compiler_with_alias("feynman", "", "typst");
        let result = c.compile("latex", "hi", Placement::Block);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported"));
    }
}

