use std::path::Path;

use anyhow::Context;
use typst::layout::PagedDocument;
use typst_html::HtmlDocument;

use crate::config::Config;
use crate::content::{ContentKind, ContentNode, ContentTree, discover_content};
use crate::template::{load_templates, render_index, render_page, render_post};
use crate::world::KilnWorld;

/// A RawBlockCompiler that compiles Typst blocks using KilnWorld.
struct TypstCompiler<'a> {
    project_root: &'a Path,
}

impl kiln_doc::ast::RawBlockCompiler for TypstCompiler<'_> {
    fn compile(&self, format: &str, source: &str) -> Result<String, String> {
        match format {
            "html" => return Ok(source.to_string()),
            "mermaid" => {
                return mermaid_rs_renderer::render(source)
                    .map_err(|e| format!("mermaid rendering failed: {e}"));
            }
            "typst" | "typst-html" => {}
            _ => return Err(format!("unsupported raw block format: {format}")),
        }
        let wrapped = format!("#set page(width: auto, height: auto, margin: 0pt)\n{source}");

        let world = KilnWorld::from_source(&wrapped, self.project_root)
            .map_err(|e| format!("failed to create typst world: {e}"))?;

        if format == "typst-html" {
            return self.compile_typst_html(&world);
        }

        let typst::diag::Warned { output, warnings } = typst::compile::<PagedDocument>(&world);

        for warning in &warnings {
            eprintln!("    typst block warn: {warning:?}");
        }

        let document = output.map_err(|errs: ecow::EcoVec<typst::diag::SourceDiagnostic>| {
            let msgs: Vec<String> = errs.iter().map(|e| format!("{e:?}")).collect();
            format!("typst compilation failed: {}", msgs.join("; "))
        })?;

        Ok(typst_svg::svg_merged(&document, typst::layout::Abs::zero()))
    }
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

    let tera = load_templates(&template_dir)?;

    // Clean and recreate output directory
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir)?;
    }
    std::fs::create_dir_all(&output_dir)?;

    // Copy static files
    if static_dir.exists() {
        copy_dir_recursive(&static_dir, &output_dir)?;
    }

    let compiler = TypstCompiler { project_root };
    let hints = config_hints(&config.hints);

    // Compile the content tree
    compile_tree(
        config,
        &tera,
        &tree.root,
        &output_dir,
        &compiler,
        &tree,
        &hints,
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

fn compile_tree(
    config: &Config,
    tera: &tera::Tera,
    node: &ContentNode,
    output_dir: &Path,
    compiler: &TypstCompiler,
    tree: &ContentTree,
    hints: &[kiln_doc::ast::Hint],
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

        let mut inject = InjectHints(hints.to_vec());
        let mut hoist = kiln_doc::resolve::HoistDirectives;
        let mut cascade = kiln_doc::resolve::CascadeHints;
        let mut compile = kiln_doc::resolve::CompileBlocks::new(compiler);
        let passes: &mut [&mut dyn kiln_doc::visit::Pass] =
            &mut [&mut inject, &mut hoist, &mut cascade, &mut compile];

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
        compile_tree(config, tera, child, output_dir, compiler, tree, hints)?;
    }

    Ok(())
}

fn config_hints(configs: &[crate::config::HintConfig]) -> Vec<kiln_doc::ast::Hint> {
    use kiln_doc::ast::{Hint, Value};

    configs
        .iter()
        .flat_map(|cfg| {
            let selector = parse_config_selector(cfg);
            let Some(selector) = selector else {
                return vec![];
            };

            cfg.properties
                .iter()
                .map(|(k, v)| Hint {
                    selector: selector.clone(),
                    property: k.clone(),
                    value: match v {
                        toml::Value::Boolean(b) => Value::Bool(*b),
                        toml::Value::Integer(n) => Value::Number(*n as f64),
                        toml::Value::Float(n) => Value::Number(*n),
                        other => Value::Text(other.to_string()),
                    },
                })
                .collect()
        })
        .collect()
}

fn parse_config_selector(cfg: &crate::config::HintConfig) -> Option<kiln_doc::ast::Selector> {
    use kiln_doc::ast::{CompoundSelector, Selector, SelectorExpr};

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
    if let Some(id) = &cfg.id {
        return Some(Selector::Expr(SelectorExpr::Simple(CompoundSelector {
            id: Some(id.clone()),
            ..Default::default()
        })));
    }
    None
}

/// Pass that injects config-level hints onto the document root.
struct InjectHints(Vec<kiln_doc::ast::Hint>);

impl kiln_doc::visit::Pass for InjectHints {
    fn run(&mut self, doc: &mut kiln_doc::ast::Document) {
        doc.content.attrs.hints.extend(self.0.iter().cloned());
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
    use kiln_doc::ast::RawBlockCompiler;

    fn compiler() -> TypstCompiler<'static> {
        TypstCompiler {
            project_root: Path::new("/tmp"),
        }
    }

    #[test]
    fn typst_compiler_handles_html() {
        let c = compiler();
        let result = c.compile("html", "<div>hello</div>");
        assert_eq!(result.unwrap(), "<div>hello</div>");
    }

    #[test]
    fn typst_compiler_handles_mermaid() {
        let c = compiler();
        let result = c.compile("mermaid", "flowchart LR\n    A --> B");
        let svg = result.unwrap();
        assert!(svg.contains("<svg"), "mermaid should produce SVG output");
    }

    #[test]
    fn typst_compiler_rejects_unknown() {
        let c = compiler();
        let result = c.compile("latex", "\\frac{1}{2}");
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
}
