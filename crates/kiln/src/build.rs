use std::path::Path;

use anyhow::Context;
use typst_html::HtmlDocument;

use crate::config::Config;
use crate::content::{discover_content, ContentItem, ContentKind, ContentNode, ContentTree};
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
            "mermaid" => return Ok(format!("<pre class=\"mermaid\">{source}</pre>")),
            "typst" => {}
            _ => return Err(format!("unsupported raw block format: {format}")),
        }
        let wrapped = format!("#set page(width: auto, height: auto, margin: 0pt)\n{source}");

        let tmp_dir =
            tempfile::tempdir().map_err(|e| format!("failed to create temp dir: {e}"))?;
        let tmp_path = tmp_dir.path().join("block.typ");
        std::fs::write(&tmp_path, &wrapped)
            .map_err(|e| format!("failed to write temp file: {e}"))?;

        let world = KilnWorld::new(&tmp_path, self.project_root)
            .map_err(|e| format!("failed to create typst world: {e}"))?;

        let typst::diag::Warned { output, warnings } =
            typst::compile::<HtmlDocument>(&world);

        for warning in &warnings {
            eprintln!("    typst block warn: {warning:?}");
        }

        let document = output.map_err(|errs| {
            let msgs: Vec<String> = errs.iter().map(|e| format!("{e:?}")).collect();
            format!("typst compilation failed: {}", msgs.join("; "))
        })?;

        let raw_html = typst_html::html(&document).map_err(|errs| {
            let msgs: Vec<String> = errs.iter().map(|e| format!("{e:?}")).collect();
            format!("typst html encoding failed: {}", msgs.join("; "))
        })?;

        Ok(extract_body(&raw_html))
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

    // Compile the content tree
    compile_tree(config, &tera, &tree.root, &output_dir, &compiler, &tree)?;

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

    match &node.kind {
        ContentKind::Post(post) => {
            compile_post(config, tera, post, output_dir, compiler, tree, &child_dirs)?;
        }
        ContentKind::Page(page) => {
            compile_page(config, tera, page, output_dir, compiler, tree, &child_dirs)?;
        }
        ContentKind::Virtual { .. } => {}
    }

    for child in &node.children {
        compile_tree(config, tera, child, output_dir, compiler, tree)?;
    }

    Ok(())
}

fn compile_content(
    md_path: &Path,
    source_dir: &Path,
    dest_dir: &Path,
    compiler: &TypstCompiler,
    skip_dirs: &[&str],
    render_fn: impl FnOnce(&str) -> anyhow::Result<String>,
) -> anyhow::Result<()> {
    let markdown = std::fs::read_to_string(md_path)
        .with_context(|| format!("failed to read {}", md_path.display()))?;

    let doc = kiln_doc::parse::parse(&markdown)
        .map_err(|e| anyhow::anyhow!("parse error in '{}': {e}", md_path.display()))?;

    let body_html = kiln_doc::html::render_html(&doc, Some(compiler));
    let full_html = render_fn(&body_html)?;

    std::fs::create_dir_all(dest_dir)?;
    std::fs::write(dest_dir.join("index.html"), &full_html)?;

    copy_assets(source_dir, dest_dir, skip_dirs)?;

    Ok(())
}

fn compile_post(
    config: &Config,
    tera: &tera::Tera,
    post: &ContentItem,
    output_dir: &Path,
    compiler: &TypstCompiler,
    tree: &ContentTree,
    skip_dirs: &[&str],
) -> anyhow::Result<()> {
    eprintln!("  compiling post: {}", post.path);
    let dest = output_dir.join("posts").join(&post.path);
    compile_content(&post.md_path, &post.source_dir, &dest, compiler, skip_dirs, |body| {
        render_post(tera, config, post, body, tree)
    })
}

fn compile_page(
    config: &Config,
    tera: &tera::Tera,
    page: &ContentItem,
    output_dir: &Path,
    compiler: &TypstCompiler,
    tree: &ContentTree,
    skip_dirs: &[&str],
) -> anyhow::Result<()> {
    eprintln!("  compiling page: {}", page.path);
    let dest = output_dir.join(&page.path);
    compile_content(&page.md_path, &page.source_dir, &dest, compiler, skip_dirs, |body| {
        render_page(tera, config, page, body, tree)
    })
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

fn copy_assets(
    source_dir: &Path,
    dest_dir: &Path,
    skip_dirs: &[&str],
) -> anyhow::Result<()> {
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

/// Extract the inner content from Typst's full HTML document output.
fn extract_body(html: &str) -> String {
    let body_start = html.find("<body>").map(|i| i + 6);
    let body_end = html.rfind("</body>");

    match (body_start, body_end) {
        (Some(start), Some(end)) if start < end => html[start..end].trim().to_string(),
        _ => html.to_string(),
    }
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
        let result = c.compile("mermaid", "graph TD\n    A --> B");
        let html = result.unwrap();
        assert!(html.contains("<pre class=\"mermaid\">"));
        assert!(html.contains("graph TD"));
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
