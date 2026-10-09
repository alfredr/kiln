use std::path::Path;

use anyhow::Context;
use tera::Tera;

use serde::Serialize;

use crate::config::Config;
use crate::content::{ContentItem, ContentKind, ContentNode, ContentTree};

pub fn load_templates(template_dir: &Path) -> anyhow::Result<Tera> {
    let pattern = format!("{}/**/*", template_dir.display());
    let tera = Tera::new(&pattern).with_context(|| "failed to load templates")?;
    Ok(tera)
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
    render_content(tera, config, tree, "post.html", body_html, |ctx| {
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
    render_content(tera, config, tree, "page.html", body_html, |ctx| {
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
}
