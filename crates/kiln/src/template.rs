use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use kiln_doc::ast::Metadata;
use tera::{Tera, Value};

use serde::Serialize;

use crate::config::Config;
use crate::content::{ContentItem, ContentKind, ContentNode, ContentTree};

pub fn load_templates(
    template_dir: &Path,
    data_dir: &Path,
    static_dir: &Path,
) -> anyhow::Result<Tera> {
    let pattern = format!("{}/**/*", template_dir.display());
    let mut tera = Tera::new(&pattern).with_context(|| "failed to load templates")?;
    tera.register_function(
        "load_data",
        LoadData {
            data_dir: data_dir.to_path_buf(),
        },
    );
    tera.register_function(
        "asset_url",
        AssetUrl {
            static_dir: static_dir.to_path_buf(),
        },
    );
    tera.register_filter("markdown", markdown_filter);
    Ok(tera)
}

// ── Template data and filters ──

fn path_arg<'a>(args: &'a HashMap<String, Value>, function: &str) -> tera::Result<&'a str> {
    args.get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| tera::Error::msg(format!("{function} requires a string `path` argument")))
}

/// Reject paths that could escape the directory they are resolved against.
fn contained_path(rel: &str) -> anyhow::Result<&Path> {
    let path = Path::new(rel);
    if !path.components().all(|c| matches!(c, Component::Normal(_))) {
        anyhow::bail!("path must be relative and stay inside its directory: '{rel}'");
    }
    Ok(path)
}

/// `load_data(path="cv.toml")`: parse a TOML, YAML, or JSON file from the
/// data directory and return it as a template value.
struct LoadData {
    data_dir: PathBuf,
}

impl tera::Function for LoadData {
    fn call(&self, args: &HashMap<String, Value>) -> tera::Result<Value> {
        let rel = path_arg(args, "load_data")?;
        load_data_file(&self.data_dir, rel).map_err(|e| tera::Error::msg(format!("{e:#}")))
    }
}

/// `asset_url(path="css/main.css")`: the site URL of a static file with a hash
/// of its contents appended, so browsers fetch it again whenever it changes.
struct AssetUrl {
    static_dir: PathBuf,
}

impl tera::Function for AssetUrl {
    fn call(&self, args: &HashMap<String, Value>) -> tera::Result<Value> {
        let rel = path_arg(args, "asset_url")?.trim_start_matches('/');
        asset_url(&self.static_dir, rel)
            .map(Value::String)
            .map_err(|e| tera::Error::msg(format!("{e:#}")))
    }

    fn is_safe(&self) -> bool {
        true
    }
}

fn asset_url(static_dir: &Path, rel: &str) -> anyhow::Result<String> {
    let path = static_dir.join(contained_path(rel)?);
    let bytes = std::fs::read(&path)
        .with_context(|| format!("failed to read static file {}", path.display()))?;
    // FNV-1a: stable across builds and toolchains, unlike DefaultHasher.
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    Ok(format!("/{rel}?v={:08x}", hash as u32))
}

fn load_data_file(data_dir: &Path, rel: &str) -> anyhow::Result<Value> {
    let path = data_dir.join(contained_path(rel)?);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read data file {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let value = match ext {
        "toml" => toml_to_value(toml::Value::Table(text.parse::<toml::Table>()?)),
        "yaml" | "yml" => serde_yaml_ng::from_str(&text)?,
        "json" => serde_json::from_str(&text)?,
        _ => anyhow::bail!("unsupported data file type: '{rel}' (expected .toml, .yaml, or .json)"),
    };
    Ok(value)
}

/// Convert TOML to a template value, rendering datetimes as their TOML string
/// form rather than serde's private datetime wrapper.
fn toml_to_value(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(n) => Value::from(n),
        toml::Value::Float(n) => Value::from(n),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_value).collect()),
        toml::Value::Table(t) => {
            Value::Object(t.into_iter().map(|(k, v)| (k, toml_to_value(v))).collect())
        }
    }
}

/// `{{ text | markdown }}` renders kiln markdown to HTML. With `inline=true`,
/// a single paragraph is unwrapped so the result can sit inside other markup.
fn markdown_filter(value: &Value, args: &HashMap<String, Value>) -> tera::Result<Value> {
    let text = value.as_str().ok_or("markdown filter expects a string")?;
    let inline = args.get("inline").and_then(Value::as_bool).unwrap_or(false);
    let doc = kiln_doc::parse::parse(text).map_err(|e| tera::Error::msg(e.to_string()))?;
    let html = kiln_doc::html::render_html(&doc);
    let html = html.trim();
    let html = match html.strip_prefix("<p>").and_then(|h| h.strip_suffix("</p>")) {
        Some(inner) if inline && !inner.contains("<p>") => inner,
        _ => html,
    };
    Ok(Value::String(html.to_string()))
}

/// The template a content item renders with: its `template` frontmatter key,
/// or the given default.
fn template_for<'a>(meta: &'a Metadata, default: &'a str) -> &'a str {
    meta.extra
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or(default)
}

// ── Serializable template types ──

#[derive(Serialize)]
struct PostEntry {
    path: String,
    slug: String,
    title: String,
    date: String,
    summary: String,
    tags: Vec<String>,
}

#[derive(Serialize)]
struct PageEntry {
    path: String,
    slug: String,
    title: String,
}

#[derive(Serialize)]
pub struct NavNode {
    pub path: String,
    pub slug: String,
    pub title: String,
    pub has_page: bool,
    pub is_post: bool,
    pub children: Vec<NavNode>,
    pub number: Option<String>,
}

// ── Conversion helpers ──

fn post_entries(posts: &[ContentItem]) -> Vec<PostEntry> {
    posts
        .iter()
        .map(|p| PostEntry {
            path: p.path.clone(),
            slug: p.slug.clone(),
            title: p.meta.title.clone().unwrap_or_else(|| p.slug.clone()),
            date: p.meta.date.clone().unwrap_or_default(),
            summary: p.meta.summary.clone().unwrap_or_default(),
            tags: p.meta.tags.clone(),
        })
        .collect()
}

fn page_entries(pages: &[ContentItem]) -> Vec<PageEntry> {
    pages
        .iter()
        .map(|p| PageEntry {
            path: p.path.clone(),
            slug: p.slug.clone(),
            title: p.meta.title.clone().unwrap_or_else(|| p.slug.clone()),
        })
        .collect()
}

fn build_nav_tree(node: &ContentNode, number_prefix: &str) -> NavNode {
    let (title, is_post, has_page) = match &node.kind {
        ContentKind::Post(item) | ContentKind::Page(item) => (
            item.meta.title.clone().unwrap_or_else(|| node.slug.clone()),
            matches!(&node.kind, ContentKind::Post(_)),
            true,
        ),
        ContentKind::Virtual { slug } => (slug.clone(), false, false),
    };

    let number = if number_prefix.is_empty() {
        None
    } else {
        Some(number_prefix.to_string())
    };

    let children: Vec<NavNode> = node
        .children
        .iter()
        .enumerate()
        .map(|(i, child)| {
            let prefix = if number_prefix.is_empty() {
                String::new()
            } else {
                format!("{number_prefix}.")
            };
            let child_number = format!("{prefix}{}", i + 1);
            build_nav_tree(child, &child_number)
        })
        .collect();

    NavNode {
        path: node.path.clone(),
        slug: node.slug.clone(),
        title,
        has_page,
        is_post,
        children,
        number,
    }
}

// ── Context insertion ──

fn insert_site(ctx: &mut tera::Context, config: &Config) {
    ctx.insert("site_title", &config.site.title);
    ctx.insert("site_url", &config.site.url);
    ctx.insert("site_description", &config.site.description);
    ctx.insert("site_author", &config.site.author);
}

fn insert_nav(ctx: &mut tera::Context, tree: &ContentTree) {
    // Flat lists (backward compatible)
    ctx.insert("posts", &post_entries(&tree.posts));
    ctx.insert("pages", &page_entries(&tree.pages));

    // Tree navigation
    let nav_tree: Vec<NavNode> = tree
        .root
        .children
        .iter()
        .enumerate()
        .map(|(i, child)| build_nav_tree(child, &format!("{}", i + 1)))
        .collect();
    ctx.insert("nav_tree", &nav_tree);
}

// ── Render functions ──

fn render_content(
    tera: &Tera,
    config: &Config,
    tree: &ContentTree,
    template: &str,
    body_html: &str,
    populate: impl FnOnce(&mut tera::Context),
) -> anyhow::Result<String> {
    let mut ctx = tera::Context::new();
    insert_site(&mut ctx, config);
    insert_nav(&mut ctx, tree);
    populate(&mut ctx);
    ctx.insert("body", body_html);
    tera.render(template, &ctx)
        .with_context(|| format!("failed to render {template}"))
}

pub fn render_post(
    tera: &Tera,
    config: &Config,
    post: &ContentItem,
    body_html: &str,
    tree: &ContentTree,
) -> anyhow::Result<String> {
    let template = template_for(&post.meta, "post.html");
    render_content(tera, config, tree, template, body_html, |ctx| {
        ctx.insert("title", &post.meta.title.as_deref().unwrap_or(&post.slug));
        ctx.insert("path", &post.path);
        ctx.insert("slug", &post.slug);
        ctx.insert("extra", &post.meta.extra);
        ctx.insert("date", &post.meta.date.as_deref().unwrap_or(""));
        ctx.insert("tags", &post.meta.tags);
        ctx.insert("summary", &post.meta.summary.as_deref().unwrap_or(""));
    })
}

pub fn render_page(
    tera: &Tera,
    config: &Config,
    page: &ContentItem,
    body_html: &str,
    tree: &ContentTree,
) -> anyhow::Result<String> {
    let template = template_for(&page.meta, "page.html");
    render_content(tera, config, tree, template, body_html, |ctx| {
        ctx.insert("title", &page.meta.title.as_deref().unwrap_or(&page.slug));
        ctx.insert("path", &page.path);
        ctx.insert("slug", &page.slug);
        ctx.insert("extra", &page.meta.extra);
    })
}

pub fn render_index(tera: &Tera, config: &Config, tree: &ContentTree) -> anyhow::Result<String> {
    let mut ctx = tera::Context::new();
    insert_site(&mut ctx, config);
    insert_nav(&mut ctx, tree);

    tera.render("index.html", &ctx)
        .with_context(|| "failed to render index template")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ContentItem, ContentKind, ContentNode};
    use kiln_doc::ast::Metadata;
    use std::path::PathBuf;

    fn test_item(slug: &str, meta: Metadata) -> ContentItem {
        ContentItem {
            meta,
            path: slug.to_string(),
            slug: slug.to_string(),
            md_path: PathBuf::from("test.md"),
            source_dir: PathBuf::from("."),
        }
    }

    fn test_config() -> Config {
        Config {
            site: crate::config::SiteConfig {
                title: "Test Site".to_string(),
                url: "https://example.com".to_string(),
                description: "A test site".to_string(),
                author: "Tester".to_string(),
            },
            build: crate::config::BuildConfig::default(),
            hints: vec![],
            typst: Default::default(),
        }
    }

    fn leaf_node(kind: ContentKind) -> ContentNode {
        let (path, slug) = match &kind {
            ContentKind::Post(item) | ContentKind::Page(item) => {
                (item.path.clone(), item.slug.clone())
            }
            ContentKind::Virtual { slug } => (slug.clone(), slug.clone()),
        };
        ContentNode {
            path: path.clone(),
            slug,
            sort_key: path,
            kind,
            children: Vec::new(),
            position: 0,
        }
    }

    #[test]
    fn post_entries_basic() {
        let item = test_item(
            "hello",
            Metadata {
                title: Some("Hello World".to_string()),
                date: Some("2026-01-01".to_string()),
                summary: Some("A post".to_string()),
                tags: vec!["rust".to_string(), "web".to_string()],
                ..Default::default()
            },
        );
        let entries = post_entries(&[item]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Hello World");
        assert_eq!(entries[0].date, "2026-01-01");
        assert_eq!(entries[0].summary, "A post");
        assert_eq!(entries[0].tags, vec!["rust", "web"]);
        assert_eq!(entries[0].slug, "hello");
    }

    #[test]
    fn post_entries_defaults() {
        let item = test_item("fallback", Metadata::default());
        let entries = post_entries(&[item]);
        assert_eq!(entries[0].title, "fallback"); // slug as fallback
        assert_eq!(entries[0].date, "");
        assert_eq!(entries[0].summary, "");
    }

    #[test]
    fn page_entries_basic() {
        let item = test_item(
            "about",
            Metadata {
                title: Some("About Us".to_string()),
                ..Default::default()
            },
        );
        let entries = page_entries(&[item]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "About Us");
        assert_eq!(entries[0].slug, "about");
    }

    #[test]
    fn page_entries_no_title() {
        let item = test_item("contact", Metadata::default());
        let entries = page_entries(&[item]);
        assert_eq!(entries[0].title, "contact"); // slug as fallback
    }

    #[test]
    fn build_nav_tree_post() {
        let item = test_item(
            "my-post",
            Metadata {
                title: Some("Post Title".to_string()),
                date: Some("2026-01-01".to_string()),
                ..Default::default()
            },
        );
        let node = leaf_node(ContentKind::Post(item));
        let nav = build_nav_tree(&node, "1");
        assert_eq!(nav.title, "Post Title");
        assert!(nav.is_post);
        assert!(nav.has_page);
        assert_eq!(nav.number, Some("1".to_string()));
    }

    #[test]
    fn build_nav_tree_page() {
        let item = test_item(
            "about",
            Metadata {
                title: Some("About".to_string()),
                ..Default::default()
            },
        );
        let node = leaf_node(ContentKind::Page(item));
        let nav = build_nav_tree(&node, "2");
        assert_eq!(nav.title, "About");
        assert!(!nav.is_post);
        assert!(nav.has_page);
    }

    #[test]
    fn build_nav_tree_virtual() {
        let node = leaf_node(ContentKind::Virtual {
            slug: "guides".to_string(),
        });
        let nav = build_nav_tree(&node, "");
        assert_eq!(nav.title, "guides");
        assert!(!nav.is_post);
        assert!(!nav.has_page);
        assert!(nav.number.is_none()); // empty prefix → None
    }

    #[test]
    fn build_nav_tree_numbering() {
        let child1 = leaf_node(ContentKind::Page(test_item(
            "a",
            Metadata {
                title: Some("A".to_string()),
                ..Default::default()
            },
        )));
        let child2 = leaf_node(ContentKind::Page(test_item(
            "b",
            Metadata {
                title: Some("B".to_string()),
                ..Default::default()
            },
        )));
        let parent = ContentNode {
            path: "docs".to_string(),
            slug: "docs".to_string(),
            sort_key: "docs".to_string(),
            kind: ContentKind::Page(test_item(
                "docs",
                Metadata {
                    title: Some("Docs".to_string()),
                    ..Default::default()
                },
            )),
            children: vec![child1, child2],
            position: 0,
        };
        let nav = build_nav_tree(&parent, "1");
        assert_eq!(nav.children.len(), 2);
        assert_eq!(nav.children[0].number, Some("1.1".to_string()));
        assert_eq!(nav.children[1].number, Some("1.2".to_string()));
    }

    #[test]
    fn insert_site_populates_context() {
        let config = test_config();
        let mut ctx = tera::Context::new();
        insert_site(&mut ctx, &config);
        // Context should have site fields — verify by serializing
        let json = ctx.into_json();
        assert_eq!(json["site_title"], "Test Site");
        assert_eq!(json["site_url"], "https://example.com");
        assert_eq!(json["site_description"], "A test site");
        assert_eq!(json["site_author"], "Tester");
    }

    #[test]
    fn load_data_formats() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("a.toml"),
            "name = \"A\"\nborn = 2026-01-01\n[[items]]\nn = 1\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("b.yaml"), "name: B\nitems: [1, 2]\n").unwrap();
        std::fs::write(tmp.path().join("c.json"), r#"{"name": "C"}"#).unwrap();

        let a = load_data_file(tmp.path(), "a.toml").unwrap();
        assert_eq!(a["name"], "A");
        assert_eq!(a["born"], "2026-01-01"); // datetime → plain string
        assert_eq!(a["items"][0]["n"], 1);
        assert_eq!(load_data_file(tmp.path(), "b.yaml").unwrap()["items"][1], 2);
        assert_eq!(load_data_file(tmp.path(), "c.json").unwrap()["name"], "C");
    }

    #[test]
    fn load_data_rejects_paths_outside_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_data_file(tmp.path(), "../kiln.toml").is_err());
        assert!(load_data_file(tmp.path(), "/etc/hosts").is_err());
    }

    #[test]
    fn asset_url_changes_with_content() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("css")).unwrap();
        let css = tmp.path().join("css/main.css");

        std::fs::write(&css, "body {}").unwrap();
        let first = asset_url(tmp.path(), "css/main.css").unwrap();
        assert!(first.starts_with("/css/main.css?v="), "{first}");
        assert_eq!(first, asset_url(tmp.path(), "css/main.css").unwrap());

        std::fs::write(&css, "body { color: red }").unwrap();
        assert_ne!(first, asset_url(tmp.path(), "css/main.css").unwrap());

        assert!(asset_url(tmp.path(), "../secret").is_err());
        assert!(asset_url(tmp.path(), "missing.css").is_err());
    }

    #[test]
    fn load_data_from_template() {
        let tmp = tempfile::tempdir().unwrap();
        let templates = tmp.path().join("templates");
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(
            templates.join("t.html"),
            r#"{% set d = load_data(path="d.toml") %}{{ d.who }}: {{ d.note | markdown(inline=true) | safe }}"#,
        )
        .unwrap();
        std::fs::write(data.join("d.toml"), "who = \"me\"\nnote = \"*hi*\"\n").unwrap();

        let tera = load_templates(&templates, &data, tmp.path()).unwrap();
        let out = tera.render("t.html", &tera::Context::new()).unwrap();
        assert_eq!(out, "me: <em>hi</em>");
    }

    #[test]
    fn markdown_filter_inline_unwraps_single_paragraph() {
        let render = |text: &str, inline: bool| {
            let args = HashMap::from([("inline".to_string(), Value::Bool(inline))]);
            markdown_filter(&Value::from(text), &args).unwrap()
        };
        assert_eq!(render("*a* b", true), "<em>a</em> b");
        assert_eq!(render("*a* b", false), "<p><em>a</em> b</p>");
        assert_eq!(render("a\n\nb", true), "<p>a</p><p>b</p>");
    }

    #[test]
    fn template_frontmatter_override() {
        let mut meta = Metadata::default();
        assert_eq!(template_for(&meta, "page.html"), "page.html");
        meta.extra
            .insert("template".to_string(), serde_yaml_ng::Value::from("cv.html"));
        assert_eq!(template_for(&meta, "page.html"), "cv.html");
    }
}
